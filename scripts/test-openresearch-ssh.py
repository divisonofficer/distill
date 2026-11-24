#!/usr/bin/env python3
"""Exercise native SSH run interruption and reconnect with disposable SSH state."""
import argparse
import base64
import hashlib
import hmac
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


class FaultProxy:
    def __init__(self, listen_port, target_port):
        self.listen_port, self.target_port = listen_port, target_port
        self.stop = threading.Event()
        self.pause = threading.Event()
        self.connections = set()
        self.lock = threading.Lock()
        self.thread = threading.Thread(target=self.serve, daemon=True)

    def start(self):
        self.thread.start()

    def serve(self):
        with socket.socket() as server:
            server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            server.bind(('127.0.0.1', self.listen_port))
            server.listen()
            server.settimeout(.2)
            while not self.stop.is_set():
                try:
                    client, _ = server.accept()
                except socket.timeout:
                    continue
                if self.pause.is_set():
                    client.close()
                    continue
                try:
                    upstream = socket.create_connection(('127.0.0.1', self.target_port), timeout=5)
                except OSError:
                    client.close()
                    continue
                with self.lock:
                    self.connections.update((client, upstream))
                threading.Thread(target=self.pump, args=(client, upstream), daemon=True).start()
                threading.Thread(target=self.pump, args=(upstream, client), daemon=True).start()

    def pump(self, left, right):
        try:
            left.settimeout(.3)
            while not self.stop.is_set() and not self.pause.is_set():
                try:
                    data = left.recv(65536)
                except socket.timeout:
                    continue
                if not data:
                    break
                right.sendall(data)
        except OSError:
            pass
        finally:
            with self.lock:
                self.connections.discard(left)
                self.connections.discard(right)
            for sock in (left, right):
                try: sock.shutdown(socket.SHUT_RDWR)
                except OSError: pass
                sock.close()

    def interrupt(self):
        self.pause.set()
        with self.lock:
            sockets = list(self.connections)
            self.connections.clear()
        for sock in sockets:
            try: sock.shutdown(socket.SHUT_RDWR)
            except OSError: pass
            sock.close()

    def resume(self):
        self.pause.clear()

    def close(self):
        self.stop.set()
        self.interrupt()
        self.thread.join(timeout=3)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix='native-ssh-', dir=args.output_dir.resolve()))
    paths = {name: root / name for name in ('home', 'data', 'config/openresearch', 'cache', 'repo', 'ssh')}
    for path in paths.values(): path.mkdir(parents=True, exist_ok=True)
    (paths['config/openresearch'] / 'settings.json').write_text('{"telemetryDisabled":true,"autoUpdate":false}')
    env = {key: os.environ[key] for key in ('PATH', 'TMPDIR') if key in os.environ}
    env.update(HOME=str(paths['home']), ORX_DATA_DIR=str(paths['data']), ORX_CACHE_DIR=str(paths['cache']), XDG_CONFIG_HOME=str(root/'config'), ORX_NO_UPDATE_CHECK='1', NO_COLOR='1', GIT_TERMINAL_PROMPT='0')
    repo = paths['repo']
    def git(*args): return subprocess.check_output(['git', *args], cwd=repo, env=env, text=True).strip()
    git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@invalid')
    (repo/'run.py').write_text('import time\nfor i in range(60):\n print(f"tick-{i}",flush=True); time.sleep(1)\n')
    git('add', 'run.py'); git('commit', '-qm', 'ssh fixture')
    commit = git('rev-parse', 'HEAD')
    sshdir = paths['ssh']; remote = root/'remote'; remote.mkdir()
    key, hostkey = sshdir/'client', sshdir/'host'
    for name, filename in (('client', key), ('host', hostkey)):
        subprocess.run(['/usr/bin/ssh-keygen','-q','-t','ed25519','-N','','-f',str(filename)],check=True,timeout=10,stdout=subprocess.DEVNULL)
    port, proxy_port = free_port(), free_port()
    authorized = sshdir/'authorized_keys'; authorized.write_text(key.with_suffix('.pub').read_text()); authorized.chmod(0o600)
    (paths['home']/'.ssh').mkdir(mode=0o700)
    config = paths['home']/'.ssh/config'
    config.write_text(f'Host fixture\n HostName 127.0.0.1\n Port {proxy_port}\n User {os.environ.get("USER", "") or "nobody"}\n IdentityFile {key}\n IdentitiesOnly yes\n StrictHostKeyChecking yes\n UserKnownHostsFile {sshdir}/known_hosts\n')
    subprocess.run(['/usr/bin/ssh-keyscan','-p',str(port),'-t','ed25519','127.0.0.1'],check=False,stdout=open(sshdir/'known_hosts','w'),stderr=subprocess.DEVNULL,timeout=5)
    # ssh-keyscan cannot connect before daemon; write own known-host record.
    import base64 as b64
    hostblob = hostkey.with_suffix('.pub').read_text().split()[1]
    (sshdir/'known_hosts').write_text(f'[127.0.0.1]:{proxy_port} ssh-ed25519 {hostblob}\n')
    sshd_config = sshdir/'sshd_config'
    sshd_config.write_text(f'Port {port}\nListenAddress 127.0.0.1\nHostKey {hostkey}\nPidFile {sshdir}/sshd.pid\nAuthorizedKeysFile {authorized}\nAllowUsers {os.environ.get("USER", "nobody")}\nPubkeyAuthentication yes\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\nPermitRootLogin yes\nLogLevel VERBOSE\n')
    daemon = proxy = proc = None
    descriptor = Path(f'/tmp/orx-{os.getuid()}') / (hashlib.sha256(str(paths['data'].resolve()).encode()).digest()[:16].hex()+'.server.json')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        subprocess.run(['/usr/sbin/sshd','-t','-f',str(sshd_config)],check=True,timeout=5)
        daemon = subprocess.Popen(['/usr/sbin/sshd','-D','-e','-f',str(sshd_config)],stdin=subprocess.DEVNULL,stdout=open(root/'sshd.log','ab'),stderr=subprocess.STDOUT,start_new_session=True)
        deadline=time.monotonic()+8
        while time.monotonic()<deadline:
            if daemon.poll() is not None: raise RuntimeError('sshd exited; inspect '+str(root/'sshd.log'))
            try:
                with socket.create_connection(('127.0.0.1',port),timeout=.2): break
            except OSError: time.sleep(.1)
        else: raise RuntimeError('sshd startup timeout')
        subprocess.run(['/usr/bin/ssh-keygen','-q','-R',f'[127.0.0.1]:{proxy_port}','-f',str(sshdir/'known_hosts')],check=False,timeout=5)
        (sshdir/'known_hosts').write_text(f'[127.0.0.1]:{proxy_port} ssh-ed25519 {hostblob}\n')
        proxy=FaultProxy(proxy_port,port); proxy.start()
        with socket.socket() as sock: api_port=free_port()
        env['ORX_POSTUDIO_TRACKIO_PORT']=str(free_port())
        proc=subprocess.Popen([str(binary),'--no-telemetry','up','--host','127.0.0.1','--port',str(api_port),'--no-browser','--no-agent'],cwd=root,env=env,stdin=subprocess.DEVNULL,stdout=open(root/'server.log','ab'),stderr=subprocess.STDOUT,start_new_session=True)
        deadline=time.monotonic()+45; token=''
        while time.monotonic()<deadline:
            if proc.poll() is not None: raise RuntimeError('native server exited; inspect '+str(root/'server.log'))
            try:
                details=json.loads(descriptor.read_text())
                if details.get('pid')==proc.pid:
                    token=details['token']; break
            except (OSError,ValueError): pass
            time.sleep(.2)
        if not token: raise RuntimeError('native startup timeout')
        def request(method,path,payload=None):
            body=None if payload is None else json.dumps(payload).encode()
            req=urllib.request.Request(f'http://127.0.0.1:{api_port}'+path,data=body,method=method,headers={'Content-Type':'application/json','Authorization':'Bearer '+token})
            with opener.open(req,timeout=12) as response: return json.loads(response.read())
        nonce='ssh-fixture'; hello=request('GET','/api/v2/hello?nonce='+nonce)
        proof=base64.b64encode(hmac.new(token.encode(),(details['instance']+'\0'+nonce).encode(),hashlib.sha256).digest()).decode()
        assert hmac.compare_digest(hello['proof'],proof), 'native hello proof mismatch'
        project_response=request('POST','/api/projects',{'name':'SSH interruption fixture','path':str(repo),'createFolder':False,'initializeGit':False,'githubSyncEnabled':False,'locale':'en','starterAgent':{'harness':'','profile':None,'model':None}})
        project=project_response.get('project',project_response)['id']
        subprocess.run([str(binary),'--no-telemetry','create-experiment',project,'--title','ssh-long','--run-command','python3 run.py','--baseline'],cwd=repo,env=env,check=True,timeout=25,stdout=subprocess.DEVNULL)
        experiment=next(item for item in request('GET',f'/api/projects/{project}/experiments')['experiments'] if item['title']=='ssh-long')['id']
        subprocess.run(['/usr/bin/ssh','-F',str(config),'-o','BatchMode=yes','fixture','true'],env=env,check=True,timeout=10)
        settings=request('GET','/api/settings/ssh')
        save=request('POST','/api/settings/ssh',{'host':'fixture','container':None,'remoteRoot':str(remote),'keepRemoteRuns':True})
        print('SSH settings GET/POST:',json.dumps({'get':settings,'save':save})[:1200],flush=True)
        result=request('POST','/api/v2/runs',{'exp_id':experiment,'backend':'ssh','host':'fixture','no_container':False,'force':False,'forwarded':True,'telemetry_suppressed':True,'expected_commit':commit})
        run_id=result['id']; print('launched',run_id,flush=True)
        deadline=time.monotonic()+60
        while time.monotonic()<deadline:
            run=request('GET',f'/api/v2/runs/{run_id}')
            log=request('GET',f'/api/v2/runs/{run_id}/log?start=0&end=65536')
            if b'tick-2' in base64.b64decode(log.get('data','')): break
            if run['status'] in ('done','failed','cancelled'): raise RuntimeError(f'SSH run ended before interruption: {run}')
            time.sleep(.3)
        else: raise RuntimeError('workload output did not appear before timeout')
        proxy.interrupt()
        deadline=time.monotonic()+20
        while time.monotonic()<deadline:
            interrupted=request('GET',f'/api/v2/runs/{run_id}')
            assert interrupted['status']=='running',interrupted
            if (interrupted.get('contact') or {}).get('type')=='unreachable':break
            time.sleep(.3)
        else:raise AssertionError(('host did not report lost contact',interrupted))
        print('PASS phase remains running with reported unreachable contact',flush=True)
        proxy.resume()
        reconnected=request('POST',f'/api/v2/runs/{run_id}/reconnect',{})
        assert reconnected['status']=='running', reconnected
        deadline=time.monotonic()+90
        while time.monotonic()<deadline:
            finished=request('GET',f'/api/v2/runs/{run_id}')
            if finished['status'] in ('done','failed','cancelled'): break
            time.sleep(.5)
        assert finished['status']=='done', finished
        final=request('GET',f'/api/v2/runs/{run_id}/log?start=0&end=65536')
        text=base64.b64decode(final['data']).decode(errors='replace')
        assert 'tick-59' in text, 'resumed run log missing final workload output'
        print('PASS native SSH workload survived TCP interruption, reconnect completed, final logs recovered',flush=True)
    finally:
        if proxy: proxy.close()
        if proc and proc.poll() is None:
            proc.terminate()
            try: proc.wait(timeout=8)
            except subprocess.TimeoutExpired: os.killpg(proc.pid,signal.SIGKILL)
        if daemon and daemon.poll() is None:
            daemon.terminate()
            try: daemon.wait(timeout=5)
            except subprocess.TimeoutExpired: os.killpg(daemon.pid,signal.SIGKILL)
        print('Evidence:',root,flush=True)


if __name__=='__main__':
    main()
