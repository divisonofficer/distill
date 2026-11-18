import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import postcss from 'postcss';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite;

before(async () => {
	vite = await createServer({
		root,
		configFile: resolve(root, 'vite.config.ts'),
		server: { middlewareMode: true },
		appType: 'custom',
		logLevel: 'silent'
	});
});

after(async () => {
	await vite?.close();
});

test('emitted CSS lets Tailwind typography utilities style form controls', async () => {
	const emitted = await vite.transformRequest('/src/app.css');
	assert.ok(emitted, 'Vite emits app.css');
	const cssModule = emitted.code.match(/const __vite__css = (".*")\n__vite__updateStyle/s);
	assert.ok(cssModule, 'Vite returns compiled CSS module');
	const root = postcss.parse(JSON.parse(cssModule[1]));
	const unlayeredFontReset = [];
	let preflightInheritsForms = false;

	root.walkRules((rule) => {
		if (!/(^|,)(button|textarea|input)(,|$)/.test(rule.selector.replace(/\\/g, ''))) return;
		const font = rule.nodes?.find((node) => node.type === 'decl' && node.prop === 'font');
		if (!font) return;
		let layer = false;
		for (let parent = rule.parent; parent && parent.type !== 'root'; parent = parent.parent) {
			if (parent.type === 'atrule' && parent.name === 'layer') layer = true;
		}
		if (!layer) unlayeredFontReset.push(rule.selector);
		else if (font.value === 'inherit') preflightInheritsForms = true;
	});

	assert.deepEqual(unlayeredFontReset, [], 'no unlayered form font shorthand overrides utilities');
	assert.equal(preflightInheritsForms, true, 'Tailwind preflight retains form-control inheritance');
});
