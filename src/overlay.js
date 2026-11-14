import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
const original = document.getElementById('original');
const translated = document.getElementById('translated');
const state = document.getElementById('overlay-status');
const liveText = document.getElementById('overlay-live-text');
let transcriptionOnly = false;
function mode(value) {
  transcriptionOnly = !!value;
  translated.classList.toggle('hidden', transcriptionOnly);
  original.classList.toggle('overlay-transcript', transcriptionOnly);
  liveText.textContent = transcriptionOnly ? 'LIVE TRANSCRIPTION' : 'LIVE TRANSLATION';
}
function status(value) {
  document.body.classList.toggle('unlocked', !value.overlayLocked);
  mode(value.transcriptionOnly);
  state.textContent = ({ idle:'待命', connecting:'连接中', running:transcriptionOnly ? '转写中' : '翻译中', stopping:'停止中', error:'连接错误' })[value.phase] || value.phase;
}
if (window.__TAURI_INTERNALS__) {
  listen('subtitle-update', ({payload}) => {
    if (typeof payload.transcriptionOnly === 'boolean') mode(payload.transcriptionOnly);
    original.textContent = payload.original || '等待语音输入…';
    translated.textContent = payload.translated || '翻译将在这里显示';
  });
  listen('session-status', ({payload}) => status(payload));
  invoke('get_status').then(status).catch(() => {});
}
