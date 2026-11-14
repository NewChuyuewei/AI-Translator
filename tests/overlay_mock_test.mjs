import { JSDOM } from 'jsdom';
import { readFile } from 'node:fs/promises';
import assert from 'node:assert/strict';

const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
const dom = new JSDOM(html, { url: 'http://localhost', pretendToBeVisual: true });
globalThis.window = dom.window; globalThis.document = dom.window.document;
const callbacks = new Map(), handlers = new Map();
let nextId = 1;
window.__TAURI_INTERNALS__ = {
  transformCallback(fn) { const id = nextId++; callbacks.set(id, fn); return id; },
  async invoke(cmd, args) {
    if (cmd === 'plugin:event|listen') { handlers.set(args.event, callbacks.get(args.handler)); return nextId++; }
    if (cmd === 'get_status') return { phase: 'running', overlayLocked: true, transcriptionOnly: true, overlayScale: 1.3 };
    return null;
  }
};
const emit = (name, payload) => handlers.get(name)?.({ payload });
const $ = id => document.getElementById(id);
await import(new URL('../src/overlay.js?test=1', import.meta.url).href);
await new Promise(resolve => setTimeout(resolve, 20));
assert.equal($('overlay-live-text').textContent, 'LIVE TRANSCRIPTION');
assert.equal($('overlay-status').textContent, '转写中');
assert.equal($('translated').classList.contains('hidden'), true);
assert.equal($('original').classList.contains('overlay-transcript'), true);
assert.equal(document.body.style.getPropertyValue('--overlay-scale'), '1.3');
Object.defineProperty($('original'), 'scrollHeight', { configurable:true, value:240 });
emit('subtitle-update', { original: '正在转写的新内容', translated: '', transcriptionOnly: true });
await new Promise(resolve => setTimeout(resolve, 20));
assert.equal($('original').textContent, '正在转写的新内容');
assert.equal($('original').scrollTop, 240);
assert.equal($('translated').classList.contains('hidden'), true);
emit('session-status', { phase: 'running', overlayLocked: false, transcriptionOnly: false });
emit('subtitle-update', { original: 'hello', translated: '你好', transcriptionOnly: false });
assert.equal($('overlay-live-text').textContent, 'LIVE TRANSLATION');
assert.equal($('overlay-status').textContent, '翻译中');
assert.equal($('translated').classList.contains('hidden'), false);
assert.equal($('translated').textContent, '你好');
assert.equal(document.body.classList.contains('unlocked'), true);
console.log('Overlay mock checks passed: transcription-only and translation states.');
dom.window.close();
