import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const $ = (id) => document.getElementById(id);
const isTauri = !!window.__TAURI_INTERNALS__;
let status = { phase: 'idle', message: '准备就绪', platform: 'preview', shortcutAvailable: false, overlayLocked: true, networkRoute: '尚未检测', transcriptionOnly: false, overlayScale: 1 };
let reconfigureTimer, rememberTimer, overlayScaleTimer, reconfiguring = false, noticeTimer;
let previewTranscriptionOnly = null, devicePlatform = null;
let vaultUnlocked = false, hasSavedVault = false, secretsDirty = false, secretEditSeq = 0;
const waiting = [];
const diagnosticEntries = new Map();
let diagnosticClearSeq = 0;

function renderDiagnostics() {
  const ordered = [...diagnosticEntries.values()].sort((a, b) => a.seq - b.seq);
  const area = $('diagnostic-output');
  const wasAtBottom = area.scrollTop + area.clientHeight >= area.scrollHeight - 16;
  area.value = ordered.map(entry => {
    const time = new Date(entry.timestampMs).toLocaleString('zh-CN', { hour12: false });
    return `${time} [${entry.level}] #${entry.sessionId} ${entry.message}`;
  }).join('\n');
  $('diagnostic-count').textContent = `${ordered.length} 条`;
  if (wasAtBottom) area.scrollTop = area.scrollHeight;
}
function receiveDiagnostic(entry) {
  if (!entry || !Number.isSafeInteger(entry.seq) || entry.seq <= diagnosticClearSeq ||
      !Number.isFinite(entry.timestampMs) || typeof entry.message !== 'string') return;
  diagnosticEntries.set(entry.seq, entry);
  const ids = [...diagnosticEntries.keys()].sort((a, b) => a - b);
  for (const id of ids.slice(0, Math.max(0, ids.length - 300))) diagnosticEntries.delete(id);
  renderDiagnostics();
}
async function loadDiagnostics() {
  try {
    // Register first, then merge the bounded native history: no event can be
    // lost between window creation and the snapshot, and seq deduplicates.
    await listen('diagnostic-log', event => receiveDiagnostic(event.payload));
    const history = await invoke('get_diagnostics');
    if (Array.isArray(history)) for (const entry of history) receiveDiagnostic(entry);
  } catch (_) { toast('无法加载诊断日志，请重启应用后重试'); }
}
async function clearDiagnostics() {
  const button = $('clear-diagnostics'); button.disabled = true;
  try {
    if (isTauri) {
      const watermark = await invoke('clear_diagnostics');
      if (!Number.isSafeInteger(watermark)) throw new Error('invalid watermark');
      diagnosticClearSeq = Math.max(diagnosticClearSeq, watermark);
      for (const id of diagnosticEntries.keys()) if (id <= diagnosticClearSeq) diagnosticEntries.delete(id);
    } else { diagnosticEntries.clear(); }
    renderDiagnostics(); toast('诊断日志已清空');
  } catch (_) { toast('清空失败，请重试'); }
  finally { button.disabled = false; }
}
async function copyDiagnostics() {
  const area = $('diagnostic-output'), text = area.value;
  if (!text.trim()) { toast('暂无日志，请先尝试开始会话'); return; }
  if (isTauri && status.platform === 'android') {
    try { await invoke('copy_diagnostic_text', { text }); toast('诊断日志已复制'); return; } catch (_) { /* manual fallback below */ }
  } else if (window.navigator.clipboard?.writeText) {
    try { await window.navigator.clipboard.writeText(text); toast('诊断日志已复制'); return; } catch (_) { /* manual fallback below */ }
  }
  area.focus(); area.select();
  try {
    if (typeof document.execCommand === 'function' && document.execCommand('copy')) { toast('诊断日志已复制'); return; }
  } catch (_) { /* leave selected for manual copy */ }
  toast('自动复制不可用，请长按或选中日志后手动复制');
}

function toast(message) {
  const el = $('notice'); el.textContent = String(message); el.classList.remove('hidden');
  clearTimeout(noticeTimer); noticeTimer = setTimeout(() => el.classList.add('hidden'), 5000);
}
function renderScale(value) {
  const percent = Math.round((Number(value) || 1) * 100);
  $('overlay-scale').value = String(Math.min(160, Math.max(80, percent)));
  $('overlay-scale-value').textContent = `${$('overlay-scale').value}%`;
}
function renderStatus(next) {
  status = { ...status, ...next };
  if (Number.isFinite(status.overlayScale)) renderScale(status.overlayScale);
  // A manual stop must cancel a queued settings restart, not restart capture.
  if (status.phase !== 'running' && !reconfiguring) clearTimeout(reconfigureTimer);
  $('status-message').textContent = status.message;
  $('network-route').textContent = status.networkRoute || '尚未检测';
  $('status-dot').className = status.phase; $('footer-dot').className = `footer-dot ${status.phase}`;
  $('platform-label').textContent = status.platform === 'android' ? 'ANDROID' : status.platform === 'preview' ? 'BROWSER PREVIEW' : 'DESKTOP';
  $('button-icon').textContent = ['connecting', 'running'].includes(status.phase) ? '■' : '▶';
  $('start-stop').disabled = status.phase === 'stopping' || reconfiguring;
  $('overlay-lock').textContent = status.overlayLocked ? '解锁字幕窗以拖动' : '锁定并穿透';
  $('overlay-hint').textContent = status.overlayLocked ? '默认鼠标穿透' : '拖动完成后请重新锁定';
  $('overlay-permission').classList.toggle('hidden', status.platform !== 'android');
  $('overlay-permission').disabled = ['connecting', 'stopping'].includes(status.phase) || reconfiguring;
  $('overlay-lock').disabled = status.platform === 'android' && status.phase !== 'running';
  $('overlay-scale').disabled = status.platform === 'android';
  const shortcut = document.querySelector('.shortcut');
  shortcut.classList.toggle('hidden', status.platform === 'android');
  shortcut.title = status.shortcutAvailable ? 'F8 开始 / 停止' : '当前平台未注册 F8';
  if (status.phase === 'idle' || status.phase === 'error') { while (waiting.length) waiting.shift()(); }
  updateModel();
}
function collectOptions() {
  const model = $('model').value;
  const transcribe = model === 'transcribe_live_35';
  const bidirectionalSupported = model === 'general_live' || model === 'extended_thinking';
  return {
    apiKey: $('api-key').value.trim(), model, thinkingLevel: $('thinking-level').value,
    transcriptionMode: $('transcription-mode').value,
    targetLanguage: $('target').value, bidirectional: bidirectionalSupported && $('bidirectional').checked,
    otherLanguage: $('other-language').value,
    mode: document.querySelector('input[name=mode]:checked').value,
    microphoneDevice: $('microphone-device').value || null,
    systemDevice: $('system-device').value || null,
    playbackEnabled: !transcribe && $('playback-enabled').checked,
    playbackDevice: transcribe ? null : ($('playback-device').value || null),
    proxyMode: $('proxy-mode').value,
    proxyUrl: $('proxy-mode').value === 'manual' ? $('proxy-url').value.trim() : '',
    overlayScale: Number($('overlay-scale').value) / 100,
  };
}
function waitForStopped() {
  if (['idle','error'].includes(status.phase)) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const done = () => { clearTimeout(timer); resolve(); };
    const timer = setTimeout(() => {
      const index = waiting.indexOf(done); if (index >= 0) waiting.splice(index, 1);
      reject(new Error('等待停止超时；请确认 Android 授权窗口已关闭。'));
    }, 30000);
    waiting.push(done);
  });
}
// Keep F8 synchronized and persist preferences. Secrets are removed by the
// native backend unless the user explicitly opts in on this device.
function rememberOptions() {
  if (!isTauri) return;
  clearTimeout(rememberTimer);
  if ($('save-sensitive').checked && !hasSavedVault && !vaultUnlocked) return;
  const editSeq = secretEditSeq;
  rememberTimer = setTimeout(() => {
    const encryptChanges = vaultUnlocked && secretsDirty;
    invoke('remember_settings', {
      options: collectOptions(), saveSensitive: $('save-sensitive').checked,
      encryptionPassword: encryptChanges ? $('encryption-password').value : '',
    }).then(() => { if (encryptChanges && secretEditSeq === editSeq) secretsDirty = false; }).catch(toast);
  }, 180);
}
function updateVaultUi() {
  const enabled = $('save-sensitive').checked;
  $('vault-controls').classList.toggle('hidden', !enabled);
  $('vault-status').textContent = vaultUnlocked ? '已解锁 · 修改后会重新加密保存'
    : hasSavedVault ? '已加密保存 · 输入密码以解锁' : '设置至少 8 个字符的密码';
  $('unlock-vault').textContent = hasSavedVault && !vaultUnlocked ? '解锁' : '保存加密配置';
}
function setSelectValue(id, value) {
  const select = $(id);
  if (value != null && [...select.options].some(option => option.value === value)) select.value = value;
}
function applySavedSettings(saved) {
  if (!saved?.options) return;
  const options = saved.options;
  $('save-sensitive').checked = !!saved.saveSensitive;
  hasSavedVault = !!saved.saveSensitive;
  vaultUnlocked = false; secretsDirty = false;
  $('encryption-password').value = '';
  $('api-key').value = options.apiKey || '';
  for (const [id, value] of [
    ['model', options.model], ['thinking-level', options.thinkingLevel],
    ['transcription-mode', options.transcriptionMode], ['target', options.targetLanguage],
    ['other-language', options.otherLanguage], ['microphone-device', options.microphoneDevice || ''],
    ['system-device', options.systemDevice || ''], ['playback-device', options.playbackDevice || ''],
    ['proxy-mode', options.proxyMode],
  ]) setSelectValue(id, value);
  $('bidirectional').checked = !!options.bidirectional;
  $('playback-enabled').checked = !!options.playbackEnabled;
  renderScale(options.overlayScale || 1);
  $('proxy-url').value = options.proxyUrl || '';
  const mode = [...document.querySelectorAll('input[name=mode]')].find(input => input.value === options.mode);
  if (mode) mode.checked = true;
  updateVaultUi(); updateProxy(); updateModel();
  if (isTauri && status.platform !== 'android') {
    invoke('set_overlay_scale', { scale: Number($('overlay-scale').value) / 100 }).catch(toast);
  }
}
async function start() {
  if (!isTauri) { toast('这是浏览器界面预览。请在 Tauri 桌面或 Android 应用中运行。'); return; }
  const options = collectOptions();
  if (!options.apiKey) { $('api-key').focus(); toast('请先填写 Gemini API Key'); return; }
  try { await invoke('start_translation', { options }); } catch (error) { toast(error); }
}
async function stop() { try { if (isTauri) await invoke('stop_translation'); } catch (e) { toast(e); } }
function scheduleReconfigure() {
  clearTimeout(reconfigureTimer);
  if (status.phase !== 'running' || reconfiguring || !isTauri) return;
  reconfigureTimer = setTimeout(async () => {
    if (status.phase !== 'running' || reconfiguring) return;
    reconfiguring = true; $('start-stop').disabled = true;
    try {
      toast(status.platform === 'android' ? '正在应用设置；系统内录可能需要重新授权。' : '正在重连以应用新的会话设置…');
      await invoke('stop_translation'); await waitForStopped();
      const options = collectOptions();
      if (!options.apiKey) throw new Error('请填写 API Key');
      await invoke('start_translation', { options });
    } catch (e) { toast(e); }
    finally { reconfiguring = false; renderStatus(status); }
  }, 650);
}
function fill(selectId, devices) {
  const select = $(selectId), previous = select.value;
  select.replaceChildren(new Option('系统默认', ''));
  for (const d of devices || []) select.add(new Option(`${d.name}${d.isDefault ? ' · 默认' : ''}`, d.id));
  if ([...select.options].some(o => o.value === previous)) select.value = previous;
}
async function refreshDevices() {
  if (!isTauri) { $('platform-label').textContent = 'BROWSER PREVIEW'; return; }
  try {
    const devices = await invoke('list_audio_devices');
    devicePlatform = devices.platform;
    fill('microphone-device', devices.microphones); fill('system-device', devices.systemSources); fill('playback-device', devices.playbackDevices);
    $('microphone-device').disabled = devices.platform === 'android';
    $('system-device').disabled = devices.platform === 'android';
    updateModel();
  } catch (e) { toast(`设备枚举失败：${e}`); }
}
function updateProxy() {
  const manual = $('proxy-mode').value === 'manual';
  $('proxy-url-field').classList.toggle('hidden', !manual);
  $('proxy-url').disabled = !manual;
}
function setPreviewMode(transcriptionOnly) {
  if (previewTranscriptionOnly !== transcriptionOnly) {
    previewTranscriptionOnly = transcriptionOnly;
    $('preview-original').textContent = '等待语音输入…';
    $('preview-translated').textContent = '翻译结果将在这里显示。';
  }
  $('preview-translated-label').classList.toggle('hidden', transcriptionOnly);
  $('preview-translated').classList.toggle('hidden', transcriptionOnly);
  $('preview-original').classList.toggle('preview-transcript', transcriptionOnly);
  $('preview-hint').textContent = transcriptionOnly ? '仅识别并显示原文，不翻译、不播放译音' : '原文与译文将显示在悬浮窗';
}
function updateModel() {
  const model = $('model').value;
  const extended = model === 'extended_thinking';
  const translation35 = model === 'live_translate_35';
  const transcribe = model === 'transcribe_live_35';
  const bidirectionalSupported = !translation35 && !transcribe;
  $('thinking-level-field').classList.toggle('hidden', !extended);
  $('thinking-level').disabled = !extended;
  $('transcription-mode-field').classList.toggle('hidden', !transcribe);
  $('transcription-mode').disabled = !transcribe;
  $('target').disabled = transcribe;
  $('bidirectional').disabled = !bidirectionalSupported;
  if (!bidirectionalSupported) $('bidirectional').checked = false;
  $('other-language').disabled = !bidirectionalSupported || !$('bidirectional').checked;
  $('playback-enabled').disabled = transcribe;
  if (transcribe) $('playback-enabled').checked = false;
  $('playback-device').disabled = transcribe || status.platform === 'android' || devicePlatform === 'android';
  $('playback-hint').textContent = transcribe ? '转写模式仅输出原文文字，没有翻译语音。' : 'Gemini 输出 24 kHz PCM；开启前请确认不会形成回声。';
  let modelInfo;
  if (transcribe) modelInfo = '仅实时转写原文，不翻译、不播放译音；自动检测语言。单次最长 10 分钟，断开后需手动重新开始。';
  else if (translation35) modelInfo = '专用语音翻译：每次会话固定一个目标语言，不支持双向切换；输入已是目标语言时不复述。';
  else if (extended) modelInfo = 'Extended Thinking 使用译员指令；推理深度越高可能延迟越大。双向识别依赖模型判断。';
  else modelInfo = '3.8 使用译员指令；双向切换依赖模型判断，并非两条独立译流。';
  $('model-hint').textContent = modelInfo + (status.platform === 'android' ? ' Android 系统内录需 Android 10+、每次授权，且源应用允许捕获。' : '');
  // During a settings change the old session stays active until it stops; its
  // status and captions must reflect the actual model, not the new selection.
  const active = ['connecting', 'running', 'stopping'].includes(status.phase) ? status.transcriptionOnly : transcribe;
  const labels = { idle: '待命', connecting: '连接中', running: active ? '转写中' : '翻译中', stopping: '停止中', error: '出错' };
  $('status-short').textContent = labels[status.phase] || status.phase;
  $('button-text').textContent = status.phase === 'running' ? (active ? '停止转写' : '停止翻译') : status.phase === 'connecting' ? '取消连接' : status.phase === 'stopping' ? '停止中…' : (transcribe ? '开始转写' : '开始翻译');
  setPreviewMode(!!active);
}
$('start-stop').addEventListener('click', () => ['connecting', 'running'].includes(status.phase) ? stop() : start());
$('copy-diagnostics').addEventListener('click', copyDiagnostics);
$('clear-diagnostics').addEventListener('click', clearDiagnostics);
$('refresh-devices').addEventListener('click', refreshDevices);
$('reveal-key').addEventListener('click', () => { const input = $('api-key'); const showing = input.type === 'text'; input.type = showing ? 'password' : 'text'; $('reveal-key').textContent = showing ? '显示' : '隐藏'; });
$('overlay-scale').addEventListener('input', () => {
  const scale = Number($('overlay-scale').value) / 100;
  renderScale(scale); rememberOptions();
  if (!isTauri || status.platform === 'android') return;
  clearTimeout(overlayScaleTimer);
  overlayScaleTimer = setTimeout(() => invoke('set_overlay_scale', { scale }).catch(toast), 80);
});
$('overlay-lock').addEventListener('click', async () => {
  if (!isTauri) return toast('请在 Tauri 应用中控制悬浮窗');
  try { await invoke('set_overlay_locked', { locked: !status.overlayLocked }); } catch (e) { toast(e); }
});
$('overlay-permission').addEventListener('click', async () => {
  try { await invoke('request_overlay_permission'); toast('已获得悬浮窗权限；正在运行时可解锁并移动字幕。'); } catch (e) { toast(e); }
});
$('quit').addEventListener('click', () => isTauri ? invoke('quit_app') : toast('浏览器预览无需退出'));
for (const id of ['model','thinking-level','transcription-mode','target','bidirectional','other-language','microphone-device','system-device','playback-enabled','playback-device']) {
  $(id).addEventListener('change', () => { updateModel(); rememberOptions(); scheduleReconfigure(); });
}
$('api-key').addEventListener('input', () => {
  secretsDirty = true; secretEditSeq += 1;
  if ($('save-sensitive').checked && !vaultUnlocked) toast('请先解锁加密保险库，或关闭加密保存。');
  rememberOptions();
});
$('api-key').addEventListener('change', scheduleReconfigure);
$('save-sensitive').addEventListener('change', () => {
  if ($('save-sensitive').checked) {
    vaultUnlocked = false;
    updateVaultUi();
    $('encryption-password').focus();
    toast('请设置加密密码，然后点击“保存加密配置”。');
  } else {
    vaultUnlocked = false; hasSavedVault = false; secretsDirty = false;
    $('encryption-password').value = '';
    updateVaultUi(); rememberOptions();
    toast('已关闭加密保存；本地加密副本将被清除。');
  }
});
$('unlock-vault').addEventListener('click', async () => {
  const password = $('encryption-password').value;
  if (password.length < 8) { $('encryption-password').focus(); toast('加密密码至少需要 8 个字符'); return; }
  $('unlock-vault').disabled = true;
  try {
    if (hasSavedVault && !vaultUnlocked) {
      const secrets = await invoke('unlock_saved_secrets', { password });
      $('api-key').value = secrets.apiKey || '';
      $('proxy-url').value = secrets.proxyUrl || '';
      updateProxy();
      vaultUnlocked = true; secretsDirty = false;
      toast('加密配置已解锁，仅在本次运行内保持解锁。');
    } else {
      const options = collectOptions();
      if (!options.apiKey) { $('api-key').focus(); throw new Error('请先填写 API Key'); }
      await invoke('remember_settings', { options, saveSensitive: true, encryptionPassword: password });
      vaultUnlocked = true; hasSavedVault = true; secretsDirty = false;
      toast('API Key 和手动代理地址已加密保存。');
    }
    updateVaultUi();
  } catch (error) { toast(error); }
  finally { $('unlock-vault').disabled = false; }
});
$('encryption-password').addEventListener('keydown', event => { if (event.key === 'Enter') $('unlock-vault').click(); });
$('proxy-mode').addEventListener('change', () => {
  updateProxy(); rememberOptions();
  if ($('proxy-mode').value !== 'manual' || $('proxy-url').value.trim()) scheduleReconfigure();
});
$('proxy-url').addEventListener('input', () => {
  secretsDirty = true; secretEditSeq += 1;
  if ($('save-sensitive').checked && !vaultUnlocked) toast('请先解锁加密保险库，或关闭加密保存。');
  rememberOptions();
});
$('proxy-url').addEventListener('change', scheduleReconfigure);
updateProxy();
for (const radio of document.querySelectorAll('input[name=mode]')) radio.addEventListener('change', () => { rememberOptions(); scheduleReconfigure(); });
if (isTauri) {
  loadDiagnostics();
  listen('session-status', event => renderStatus(event.payload));
  listen('subtitle-update', event => {
    if (typeof event.payload.transcriptionOnly === 'boolean') setPreviewMode(event.payload.transcriptionOnly);
    $('preview-original').textContent = event.payload.original || '等待语音输入…';
    $('preview-translated').textContent = event.payload.translated || '翻译结果将在这里显示。';
    window.requestAnimationFrame(() => {
      const preview = document.querySelector('.subtitle-preview');
      preview.scrollTop = preview.scrollHeight;
    });
  });
  listen('session-notice', event => toast(event.payload));
  (async () => {
    try {
      renderStatus(await invoke('get_status'));
      await refreshDevices();
      applySavedSettings(await invoke('get_saved_settings'));
    } catch (error) { toast(error); }
  })();
} else {
  renderStatus({ platform: 'preview', message: '界面预览 · 请在 Tauri 客户端中运行' });
  updateModel();
}
