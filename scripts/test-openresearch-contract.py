#!/usr/bin/env python3
"""Exercise the patched native API using disposable Git data and no model calls."""
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
import time
import urllib.error
import urllib.request


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--trackio-python',type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix='native-', dir=args.output_dir.resolve()))
    for name in ['home', 'data', 'config/openresearch', 'cache', 'repo']:
        (root / name).mkdir(parents=True, exist_ok=True)
    (root / 'config/openresearch/settings.json').write_text('{"telemetryDisabled":true,"autoUpdate":false}')
    env = {key: os.environ[key] for key in ['PATH', 'TMPDIR'] if key in os.environ}
    env.update(HOME=str(root/'home'), ORX_DATA_DIR=str(root/'data'), ORX_CACHE_DIR=str(root/'cache'), XDG_CONFIG_HOME=str(root/'config'), ORX_NO_UPDATE_CHECK='1', NO_COLOR='1', GIT_TERMINAL_PROMPT='0')
    repo = root/'repo'
    def git(*command, cwd=repo):
        return subprocess.check_output(['git', *command], cwd=cwd, env=env, text=True).strip()
    git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@invalid')
    (repo/'run.py').write_text('import sys,time,os\nassert not os.path.exists("dirty.txt")\nprint("fixture-start",flush=True)\nif sys.argv[1]=="slow": time.sleep(90)\nprint("fixture-end",flush=True)\nsys.exit(7 if sys.argv[1]=="fail" else 0)\n')
    if args.trackio_python:
        env['PATH']=str(args.trackio_python.absolute().parent)+os.pathsep+env.get('PATH','')
        env.update({'HF_HUB_OFFLINE':'1','HF_HUB_DISABLE_TELEMETRY':'1','GRADIO_ANALYTICS_ENABLED':'False'})
        (repo/'metrics.py').write_text('import os\nimport trackio\ntrackio.init(project=os.environ["TRACKIO_PROJECT"],name=os.environ["ORX_RUN_ID"],embed=False,auto_log_gpu=False,auto_log_cpu=False,capture_console=False)\ntrackio.log({"psnr":27.4,"mae":0.084},step=0)\ntrackio.log({"psnr":28.6,"mae":0.071},step=10)\ntrackio.finish()\nprint("fixture-end",flush=True)\n')
        git('add','metrics.py')
    git('add', 'run.py'); git('commit', '-qm', 'fixture')
    commit = git('rev-parse', 'HEAD')
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0)); metrics_port = sock.getsockname()[1]
    env['ORX_POSTUDIO_TRACKIO_PORT'] = str(metrics_port)
    descriptor = Path(f'/tmp/orx-{os.getuid()}') / (hashlib.sha256(str((root/'data').resolve()).encode()).digest()[:16].hex()+'.server.json')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    proc = None
    token = ''
    def request(method, path, payload=None, statuses=(200,)):
        body = None if payload is None else json.dumps(payload).encode()
        req = urllib.request.Request(f'http://127.0.0.1:{port}'+path, data=body, method=method, headers={'Content-Type':'application/json', 'Authorization':'Bearer '+token})
        try:
            with opener.open(req, timeout=12) as response: status, data = response.status, response.read()
        except urllib.error.HTTPError as error: status, data = error.code, error.read()
        assert status in statuses, (method,path,status,data[:1000])
        try: return json.loads(data) if data else None
        except ValueError: return data.decode(errors="replace")
    def start():
        nonlocal proc, token
        proc = subprocess.Popen([str(binary),'--no-telemetry','up','--host','127.0.0.1','--port',str(port),'--no-browser','--no-agent'], env=env, cwd=root, stdin=subprocess.DEVNULL, stdout=(root/'server.log').open('ab'), stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic()+45
        while time.monotonic()<deadline:
            assert proc.poll() is None, f'native stopped: {root}/server.log'
            try:
                details=json.loads(descriptor.read_text())
                if details.get('pid')!=proc.pid: time.sleep(.15); continue
                token=details['token']
                hello=request('GET','/api/v2/hello?nonce=fixture-check')
                assert hello['postudioContract']==1
                expected=base64.b64encode(hmac.new(token.encode(),(details['instance']+'\0fixture-check').encode(),hashlib.sha256).digest()).decode()
                assert hmac.compare_digest(hello['proof'],expected)
                assert metrics_port in details['privatePorts']
                return
            except (OSError,ValueError,urllib.error.URLError): time.sleep(.15)
        raise AssertionError('native startup timeout')
    def stop():
        nonlocal proc
        if proc and proc.poll() is None:
            proc.terminate()
            try: proc.wait(timeout=10)
            except subprocess.TimeoutExpired: os.killpg(proc.pid,signal.SIGKILL); proc.wait()
        proc=None
    def create(title,command,parent=None):
        cmd=[str(binary),'--no-telemetry','create-experiment',project,'--title',title,'--run-command',command]
        cmd+=['--parent',parent] if parent else ['--baseline']
        subprocess.run(cmd,env=env,cwd=repo,check=True,stdout=subprocess.DEVNULL,timeout=25)
        return next(e for e in request('GET',f'/api/projects/{project}/experiments')['experiments'] if e['title']==title)['id']
    def launch(experiment,expected=commit,statuses=(200,)):
        return request('POST','/api/v2/runs',{'exp_id':experiment,'backend':'local','no_container':False,'force':False,'forwarded':True,'telemetry_suppressed':True,'expected_commit':expected},statuses)
    def wait(run_id,terminal=True):
        deadline=time.monotonic()+45
        while time.monotonic()<deadline:
            run=request('GET',f'/api/v2/runs/{run_id}')
            if (terminal and run['status'] in ['done','failed','cancelled']) or (not terminal and run['status']=='running'): return run
            time.sleep(.2)
        raise AssertionError(('run timeout',run))
    try:
        start()
        created=request('POST','/api/projects',{'name':'Isolated contract','path':str(repo),'createFolder':False,'initializeGit':False,'githubSyncEnabled':False,'locale':'ko','starterAgent':{'harness':'','profile':None,'model':None}})
        project=created.get('project',created)['id']
        command=f'{args.trackio_python.absolute()} metrics.py' if args.trackio_python else 'python3 run.py ok'
        baseline=create('baseline',command)
        fail=create('failure','python3 run.py fail',baseline)
        slow=create('cancel','python3 run.py slow',baseline)
        prefix=f'/api/v2/projects/{project}/experiments/{baseline}'
        workspace=request('POST',prefix+'/workspace',{})
        cwd=Path(workspace['cwd']); (cwd/'dirty.txt').write_text('excluded')
        assert request('POST',prefix+'/workspace',{})['cwd']==str(cwd)
        snap=request('GET',prefix+'/snapshot'); assert snap['headCommit']==commit and snap['uncommittedCount']==1
        launch(baseline,'0'*40,(409,))
        print('PASS native hello, reserved ports, stable dirty workspace, expected commit rejection',flush=True)
        done=wait(launch(baseline)['id']); assert done['status']=='done' and done['exitCode']==0 and done['commitSha']==commit, done
        failed=wait(launch(fail)['id']); assert failed['status']=='failed' and failed['exitCode']==7, failed
        queued=launch(slow); wait(queued['id'],False); reconnected=request('POST',f'/api/v2/runs/{queued["id"]}/reconnect',{}); assert reconnected['status']=='running'; request('POST',f'/api/v2/runs/{queued["id"]}/cancel',{}); canceled=wait(queued['id']); assert canceled['status']=='cancelled',canceled
        print('PASS actual committed snapshot success / failure 7 / acknowledged cancellation',flush=True)
        if args.trackio_python:
            metric=request('GET',f'/api/trackio/projects/{project}/runs/{done["id"]}/metrics')
            print('Trackio metric response:',json.dumps(metric),flush=True)
            values={item['name']:item for item in metric['metrics']}
            assert values['psnr']['last']['value']==28.6 and values['mae']['last']['value']==0.071,metric
            assert [(point['step'],point['value']) for point in values['psnr']['series']]==[(0,27.4),(10,28.6)],metric
            assert metrics_port in json.loads(descriptor.read_text())['privatePorts']
            (root/'metric-evidence.json').write_text(json.dumps(metric,indent=2))
            print('PASS actual Trackio numeric values, step series and reserved helper port',flush=True)

        log=request('GET',f'/api/v2/runs/{done["id"]}/log?start=0&end=65536'); before=base64.b64decode(log['data']); assert b'fixture-end' in before
        git('checkout','--detach',cwd=cwd); request('GET',prefix+'/snapshot',statuses=(409,)); git('checkout',workspace['branch'],cwd=cwd)
        stop(); start()
        for expected in [done,failed,canceled]:
            current=request('GET',f'/api/v2/runs/{expected["id"]}'); assert (current['status'],current['exitCode'])==(expected['status'],expected['exitCode'])
        after=request('GET',f'/api/v2/runs/{done["id"]}/log?start=0&end=65536'); assert base64.b64decode(after['data'])==before
        assert (cwd/'dirty.txt').read_text()=='excluded'
        print('PASS switched workspace rejected, terminal states/logs/dirty files survive server restart',flush=True)
        print('Separate checks required: SSH disconnect/reconnect, model conversation, HTTPS dashboard')
    finally:
        stop(); print('Evidence:',root,flush=True)

if __name__=='__main__': main()
