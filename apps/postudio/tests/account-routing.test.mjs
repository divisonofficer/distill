import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite;
let routing;
let accounts;

before(async () => {
	vite = await createServer({
		root,
		configFile: resolve(root, 'vite.config.ts'),
		server: { middlewareMode: true },
		appType: 'custom',
		logLevel: 'silent'
	});
	[routing, accounts] = await Promise.all([
		vite.ssrLoadModule('/src/lib/components/AccountRouting.svelte'),
		vite.ssrLoadModule('/src/lib/components/AccountSettings.svelte')
	]);
});

after(async () => {
	await vite?.close();
});

test('routing schema keeps exact optional worker shape and rejects invalid contracts', () => {
	const config = {
		enabled: true,
		strategy: 'ordered_first',
		options: [
			{ id: 'first', account_id: 'claude-main', main_model: 'claude/model' },
			{ id: 'second', account_id: 'openrouter', main_model: 'vendor/main', worker_model: 'vendor/worker' }
		]
	};
	assert.deepEqual(routing.parseRoutingConfig(config), config);
assert.deepEqual(routing.parseRoutingConfig({ ...config, options: [{ ...config.options[0], worker_model: null }] }).options, [config.options[0]]);
	assert.deepEqual(routing.routingPayload({ ...config, ignored: true }), config);
	assert.throws(() => routing.parseRoutingConfig({ ...config, strategy: 'random' }), /Invalid routing configuration/);
	assert.throws(() => routing.parseRoutingConfig({ ...config, options: [{ id: 'missing-fields' }] }), /Invalid routing option/);
});

test('adding multiple accounts under one provider generates distinct Distill IDs without using nicknames', () => {
	const first = accounts.newProviderAccountId('claude');
	const second = accounts.newProviderAccountId('claude');
	assert.match(first, /^claude-[a-f0-9]{24}$/);
	assert.match(second, /^claude-[a-f0-9]{24}$/);
	assert.notEqual(first, second);
	assert.match(accounts.newProviderAccountId('openrouter'), /^openrouter-[a-f0-9]{24}$/);
});

test('browser login allows remote HTTPS and links to supported OAuth providers only', () => {
	for (const host of ['localhost', 'LOCALHOST', '127.0.0.1', '::1', '[::1]']) assert.equal(accounts.isLocalLoginHost(host), true);
	for (const host of ['example.com', 'localhost.example.com', '127.0.0.2', '::2']) assert.equal(accounts.isLocalLoginHost(host), false);
	assert.equal(accounts.isWebLoginPage('https:', 'node.tailnet.ts.net'), true);
	assert.equal(accounts.isWebLoginPage('http:', 'localhost'), true);
	assert.equal(accounts.isWebLoginPage('http:', 'node.tailnet.ts.net'), false);
	assert.equal(accounts.safeAuthorizationHref('https://auth.openai.com/authorize'), 'https://auth.openai.com/authorize');
	assert.equal(accounts.safeAuthorizationHref('https://auth.example/authorize'), null);
	assert.equal(accounts.safeAuthorizationHref('http://claude.ai/authorize'), null);
	assert.equal(accounts.safeAuthorizationHref('https://user:pass@claude.ai/authorize'), null);
	assert.equal(accounts.safeAuthorizationHref('javascript:alert(1)'), null);
});

test('Grok link is allowed and the job callback shown is an exact loopback address', () => {
	const auth = 'https://auth.x.ai/authorize?redirect_uri=http%3A%2F%2F127.0.0.1%3A4000%2Fcallback&state=secret';
	assert.equal(accounts.safeAuthorizationHref(auth), auth);
	assert.equal(accounts.loginCallbackAddress(auth), 'http://127.0.0.1:4000/callback');
	assert.equal(accounts.loginCallbackAddress(auth.replace('127.0.0.1', 'example.com')), null);
	assert.equal(accounts.safeAuthorizationHref(auth.replace('auth.x.ai', 'auth.x.ai.evil.test')), null);
});

test('login response helpers enforce host-loopback start and known poll statuses', () => {
	assert.deepEqual(accounts.parseLoginStart({ jobId: 'job-1', status: 'starting', callbackMode: 'host_loopback' }), {
		jobId: 'job-1', status: 'starting', callbackMode: 'host_loopback'
	});
	assert.throws(() => accounts.parseLoginStart({ jobId: 'job-1', status: 'starting', callbackMode: 'remote' }), /Invalid login response/);
	assert.deepEqual(accounts.parseLoginPoll({ status: 'pending', authorizationUrl: 'https://auth.example/' }), {
		status: 'pending', authorizationUrl: 'https://auth.example/'
	});
	assert.throws(() => accounts.parseLoginPoll({ status: 'unknown' }), /Invalid login status response/);
});
