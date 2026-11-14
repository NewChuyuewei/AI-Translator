import { JSDOM } from 'jsdom';
import { readFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
const html = await readFile(new URL('../src/control.html', import.meta.url), 'utf8');
const dom = new JSDOM(html, { url: 'http://localhost', pretendToBeVisual: true });
globalThis.window = dom.window; globalThis.document = dom.window.document; globalThis.Option = dom.window.Option;
const callbacks = new Map(), handlers = new Map(), calls = [];
let nextId = 1;
let releaseHistory, clearWatermark = 0;
const history = new Promise(resolve => { releaseHistory = resolve; });
let current = { phase:'idle', message:'准备就绪', platform:'windows', shortcutAvailable:true, overlayLocked:true };
function emit(name, payload) { const cb = handlers.get(name); if (cb) cb({ payload }); }
function status(part) { current = { ...current, ...part }; emit('session-status', current); }
window.__TAURI_INTERNALS__ = {
  transformCallback(fn) { const id = nextId++; callbacks.set(id, fn); return id; },
  async invoke(cmd, args) {
    calls.push({ cmd, args });
    if (cmd === 'plugin:event|listen') { handlers.set(args.event, callbacks.get(args.handler)); return nextId++; }
    if (cmd === 'get_status') return current;
    if (cmd === 'get_saved_settings') return {
      version: 2, saveSensitive: true, options: {
        apiKey: '', model: 'general_live', thinkingLevel: 'LOW', transcriptionMode: 'VERBATIM',
        targetLanguage: 'zh', bidirectional: false, otherLanguage: 'en', mode: 'microphone_only',
        microphoneDevice: 'mic', systemDevice: null, playbackEnabled: false, playbackDevice: null,
        proxyMode: 'auto', proxyUrl: '',
      },
    };
    if (cmd === 'unlock_saved_secrets') {
      if (args.password !== 'vault-password') throw new Error('密码错误或加密配置已损坏');
      return { apiKey:'restored-key', proxyUrl:'' };
    }
    if (cmd === 'get_diagnostics') return history;
    if (cmd === 'clear_diagnostics') return clearWatermark;
    if (cmd === 'copy_diagnostic_text') return null;
    if (cmd === 'list_audio_devices') return {platform:'windows', microphones:[{id:'mic',name:'麦克风',isDefault:true}], systemSources:[], playbackDevices:[]};
    if (cmd === 'start_translation') { status({phase:'connecting'}); return; }
    if (cmd === 'stop_translation') { status({phase:'stopping'}); setTimeout(() => status({phase:'idle'}), 10); return; }
    if (cmd === 'set_overlay_locked') { status({overlayLocked:args.locked}); return; }
    if (cmd === 'set_overlay_scale') { status({overlayScale:args.scale}); return; }
    return null;
  }
};
const $ = (id) => document.getElementById(id);
const wait = ms => new Promise(resolve => setTimeout(resolve,ms));
await import(new URL('../src/control.js?test=1', import.meta.url).href);
await wait(30);
assert.equal($('platform-label').textContent, 'DESKTOP');
assert.equal($('microphone-device').options.length, 2);
assert.equal($('proxy-mode').value, 'auto');
assert.equal($('proxy-url-field').classList.contains('hidden'), true);
// A live event arriving before history resolves must merge in sequence without
// duplicates. Clear uses the native watermark to discard delayed old events.
const entry = (seq, message) => ({ seq, timestampMs: 1700000000000 + seq, sessionId: 7, level: 'INFO', message });
emit('diagnostic-log', entry(2, '初始化收到 二进制 帧，类型 setupComplete'));
assert.equal($('diagnostic-count').textContent, '1 条');
releaseHistory([entry(1, '开始连接'), entry(2, '初始化收到 二进制 帧，类型 setupComplete')]);
await wait(0);
assert.equal($('diagnostic-count').textContent, '2 条');
assert.ok($('diagnostic-output').value.indexOf('开始连接') < $('diagnostic-output').value.indexOf('setupComplete'));
clearWatermark = 2; $('clear-diagnostics').click(); await wait(0);
assert.equal($('diagnostic-count').textContent, '0 条');
emit('diagnostic-log', entry(1, '旧日志不应复活'));
emit('diagnostic-log', entry(3, '新的安全日志 <tag>'));
assert.equal($('diagnostic-count').textContent, '1 条');
assert.ok(!$('diagnostic-output').value.includes('旧日志不应复活'));
assert.ok($('diagnostic-output').value.includes('新的安全日志 <tag>'));
let desktopClipboard = '';
Object.defineProperty(window.navigator, 'clipboard', { configurable: true, value: { async writeText(text) { desktopClipboard = text; } } });
$('copy-diagnostics').click(); await wait(0);
assert.ok(desktopClipboard.includes('新的安全日志 <tag>'));
assert.equal($('notice').textContent, '诊断日志已复制');
assert.equal(calls.filter(c=>c.cmd==='get_saved_settings').length, 1);
assert.equal($('save-sensitive').checked, true);
assert.equal($('api-key').value, '');
assert.equal($('microphone-device').value, 'mic');
assert.equal($('vault-controls').classList.contains('hidden'), false);
$('encryption-password').value = 'vault-password'; $('unlock-vault').click(); await wait(0);
assert.equal(calls.filter(c=>c.cmd==='unlock_saved_secrets').length, 1);
assert.equal($('api-key').value, 'restored-key');
assert.ok($('vault-status').textContent.includes('已解锁'));
$('save-sensitive').checked = false; $('save-sensitive').dispatchEvent(new window.Event('change'));
await wait(260);
let rememberedCall = calls.filter(c=>c.cmd==='remember_settings').at(-1);
assert.equal(rememberedCall.args.saveSensitive, false);
assert.equal(rememberedCall.args.encryptionPassword, '');
assert.ok($('notice').textContent.includes('加密副本将被清除'));
$('api-key').value = 'test-key'; $('api-key').dispatchEvent(new window.Event('input'));
await wait(260);
rememberedCall = calls.filter(c=>c.cmd==='remember_settings').at(-1);
assert.equal(rememberedCall.args.options.apiKey,'test-key');
assert.equal(rememberedCall.args.saveSensitive, false);
$('save-sensitive').checked = true; $('save-sensitive').dispatchEvent(new window.Event('change'));
$('encryption-password').value = 'new-vault-password'; $('unlock-vault').click(); await wait(0);
rememberedCall = calls.filter(c=>c.cmd==='remember_settings').at(-1);
assert.equal(rememberedCall.args.saveSensitive, true);
assert.equal(rememberedCall.args.encryptionPassword, 'new-vault-password');
assert.ok($('notice').textContent.includes('已加密保存'));
$('start-stop').click(); await wait(0);
assert.equal(calls.filter(c=>c.cmd==='start_translation').at(-1).args.options.proxyMode, 'auto');
assert.equal(calls.filter(c=>c.cmd==='start_translation').at(-1).args.options.proxyUrl, '');
assert.equal($('button-text').textContent, '取消连接'); assert.equal($('start-stop').disabled, false);
$('start-stop').click(); await wait(30);
assert.equal($('button-text').textContent, '开始翻译');
assert.equal(calls.filter(c=>c.cmd==='stop_translation').length,1);
status({phase:'running'}); emit('subtitle-update',{original:'hello', translated:'你好'});
assert.equal($('preview-original').textContent,'hello'); assert.equal($('preview-translated').textContent,'你好');
$('overlay-lock').click(); await wait(0); assert.equal($('overlay-lock').textContent,'锁定并穿透');
$('overlay-scale').value='130'; $('overlay-scale').dispatchEvent(new window.Event('input')); await wait(260);
assert.equal($('overlay-scale-value').textContent, '130%');
assert.equal(calls.filter(c=>c.cmd==='set_overlay_scale').at(-1).args.scale, 1.3);
assert.equal(calls.filter(c=>c.cmd==='remember_settings').at(-1).args.options.overlayScale, 1.3);
// An explicit stop cancels a queued settings restart.
$('target').value='ja'; $('target').dispatchEvent(new window.Event('change'));
status({phase:'stopping'}); status({phase:'idle'});
let before = calls.filter(c=>c.cmd==='start_translation').length;
await wait(750); assert.equal(calls.filter(c=>c.cmd==='start_translation').length,before);
// A settings change during a running session does stop and start with updated options.
status({phase:'running'}); $('target').value='fr'; $('target').dispatchEvent(new window.Event('change'));
await wait(850);
assert.equal(calls.filter(c=>c.cmd==='start_translation').length,before+1);
assert.equal(calls.filter(c=>c.cmd==='start_translation').at(-1).args.options.targetLanguage,'fr');
status({ phase:'idle' });
assert.equal($('thinking-level-field').classList.contains('hidden'), true);
$('model').value='extended_thinking'; $('model').dispatchEvent(new window.Event('change'));
assert.equal($('thinking-level-field').classList.contains('hidden'), false);
assert.equal($('bidirectional').disabled, false);
$('bidirectional').checked=true; $('bidirectional').dispatchEvent(new window.Event('change'));
assert.equal($('other-language').disabled, false);
$('thinking-level').value='HIGH'; $('thinking-level').dispatchEvent(new window.Event('change'));
await wait(220);
const updated = calls.filter(c=>c.cmd==='remember_settings').at(-1).args.options;
assert.equal(updated.model, 'extended_thinking');
assert.equal(updated.thinkingLevel, 'HIGH');
assert.equal(updated.bidirectional, true);
$('model').value='general_live'; $('model').dispatchEvent(new window.Event('change'));
assert.equal($('thinking-level-field').classList.contains('hidden'), true);
assert.equal($('bidirectional').checked, true);
// The 3.5 translation model supports exactly one fixed target, but audio is optional.
$('playback-enabled').checked = true;
$('model').value='live_translate_35'; $('model').dispatchEvent(new window.Event('change'));
assert.equal($('bidirectional').disabled, true);
assert.equal($('bidirectional').checked, false);
assert.equal($('other-language').disabled, true);
assert.equal($('target').disabled, false);
assert.equal($('playback-enabled').disabled, false);
assert.ok($('model-hint').textContent.includes('固定一个目标语言'));
await wait(220);
let dedicated = calls.filter(c=>c.cmd==='remember_settings').at(-1).args.options;
assert.equal(dedicated.model, 'live_translate_35'); assert.equal(dedicated.bidirectional, false);
assert.equal(dedicated.playbackEnabled, true);
// Transcribe is original text only: hide translations and disable inapplicable settings.
$('model').value='transcribe_live_35'; $('model').dispatchEvent(new window.Event('change'));
assert.equal($('target').disabled, true);
assert.equal($('bidirectional').disabled, true);
assert.equal($('playback-enabled').disabled, true);
assert.equal($('playback-enabled').checked, false);
assert.equal($('playback-device').disabled, true);
assert.equal($('transcription-mode-field').classList.contains('hidden'), false);
assert.equal($('preview-translated').classList.contains('hidden'), true);
assert.equal($('preview-translated-label').classList.contains('hidden'), true);
assert.equal($('button-text').textContent, '开始转写');
assert.ok($('model-hint').textContent.includes('10 分钟'));
$('transcription-mode').value='SMART'; $('transcription-mode').dispatchEvent(new window.Event('change'));
await wait(220);
dedicated = calls.filter(c=>c.cmd==='remember_settings').at(-1).args.options;
assert.equal(dedicated.model, 'transcribe_live_35'); assert.equal(dedicated.transcriptionMode, 'SMART');
assert.equal(dedicated.bidirectional, false); assert.equal(dedicated.playbackEnabled, false); assert.equal(dedicated.playbackDevice, null);
status({phase:'running', transcriptionOnly:true, message:'正在采集并转写原文'});
emit('subtitle-update',{original:'这是识别原文', translated:'', transcriptionOnly:true});
assert.equal($('preview-original').textContent, '这是识别原文');
assert.equal($('status-short').textContent, '转写中'); assert.equal($('button-text').textContent, '停止转写');
$('model').value='live_translate_35'; $('model').dispatchEvent(new window.Event('change'));
assert.equal($('status-short').textContent, '转写中'); // old session remains active during restart debounce
assert.equal($('preview-translated').classList.contains('hidden'), true);
status({phase:'idle', transcriptionOnly:true});
assert.equal($('preview-translated').classList.contains('hidden'), false);
assert.equal($('target').disabled, false);
$('model').value='general_live'; $('model').dispatchEvent(new window.Event('change'));
status({phase:'idle', transcriptionOnly:false});
// Proxy address is only sent in manual mode; the displayed route contains no credentials.
$('proxy-mode').value='manual'; $('proxy-mode').dispatchEvent(new window.Event('change'));
assert.equal($('proxy-url-field').classList.contains('hidden'), false);
assert.equal($('proxy-url').disabled, false);
$('proxy-url').value='socks5://demo:temporary@127.0.0.1:1080'; $('proxy-url').dispatchEvent(new window.Event('input'));
await wait(220);
let proxyOptions = calls.filter(c=>c.cmd==='remember_settings').at(-1).args.options;
assert.equal(proxyOptions.proxyMode,'manual'); assert.equal(proxyOptions.proxyUrl,'socks5://demo:temporary@127.0.0.1:1080');
status({networkRoute:'手动 SOCKS5 代理'});
assert.equal($('network-route').textContent,'手动 SOCKS5 代理');
assert.equal($('network-route').textContent.includes('temporary'), false);
$('proxy-mode').value='direct'; $('proxy-mode').dispatchEvent(new window.Event('change'));
assert.equal($('proxy-url-field').classList.contains('hidden'), true);
await wait(220);
proxyOptions = calls.filter(c=>c.cmd==='remember_settings').at(-1).args.options;
assert.equal(proxyOptions.proxyMode,'direct'); assert.equal(proxyOptions.proxyUrl,'');
status({ phase:'idle', platform:'android' });
$('copy-diagnostics').click(); await wait(0);
assert.ok(calls.filter(c => c.cmd === 'copy_diagnostic_text').at(-1).args.text.includes('新的安全日志'));
assert.equal($('notice').textContent, '诊断日志已复制');
assert.equal(document.querySelector('.shortcut').classList.contains('hidden'), true);
assert.equal($('overlay-permission').classList.contains('hidden'), false);
assert.equal($('overlay-lock').disabled, true);
status({ phase:'connecting' }); assert.equal($('overlay-permission').disabled, true);
status({ phase:'running' }); assert.equal($('overlay-lock').disabled, false);
await wait(250); // Let the last debounced remember_settings invocation finish.
console.log('UI mock checks passed: diagnostics, encrypted vault unlock/save/delete, settings, proxy, models, events, overlay and Android controls.');
dom.window.close();
