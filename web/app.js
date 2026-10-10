const $ = (id) => document.getElementById(id);
const state = { system: null, displays: [], draft: [], pending: null, timer: null };

const nativeInvoke = window.__TAURI__?.core?.invoke;

async function api(path, options = {}) {
  if (nativeInvoke) {
    const commands = {
      '/api/system/status': 'system_status',
      '/api/displays': 'displays',
      '/api/displays/validate': 'validate_displays',
      '/api/displays/apply': 'apply_displays',
      '/api/displays/confirm': 'confirm_displays',
      '/api/displays/revert': 'revert_displays',
      '/api/wallpaper/set': 'wallpaper_set',
      '/api/wallpaper/select': 'wallpaper_select',
      '/api/wallpaper/search': 'wallpaper_search',
    };
    const command = commands[path];
    if (!command) throw new Error(`未知桌面命令：${path}`);
    const body = options.body ? JSON.parse(options.body) : null;
    try {
      return await nativeInvoke(command, body ? { request: body } : {});
    } catch (error) {
      throw new Error(typeof error === 'string' ? error : error?.message || '桌面命令执行失败');
    }
  }

  const response = await fetch(path, {
    cache: 'no-store',
    headers: options.body ? { 'Content-Type': 'application/json' } : {},
    ...options,
  });
  let data;
  try { data = await response.json(); } catch { data = null; }
  if (!response.ok) throw new Error(data?.error?.message || `请求失败 (${response.status})`);
  return data;
}

function notify(message, isError = false) {
  const toast = $('toast');
  toast.textContent = message;
  toast.className = `toast${isError ? ' error' : ''}`;
  clearTimeout(notify.timer);
  notify.timer = setTimeout(() => toast.classList.add('hidden'), 4500);
}

function modeKey(width, height) { return `${width}x${height}`; }
function rateLabel(value) { return Number(value).toFixed(2).replace(/0+$/, '').replace(/\.$/, ''); }
function preferredMode(display) {
  return display.modes.find((mode) => mode.preferred) || display.modes[0] || null;
}
function preferredRate(mode) {
  return mode?.refreshRates.find((rate) => rate.preferred)?.value ?? mode?.refreshRates[0]?.value ?? null;
}

function draftFromDisplays(displays) {
  return displays.filter((display) => display.connected).map((display) => ({
    output: display.name,
    enabled: display.enabled,
    mode: display.currentMode ? {
      width: display.currentMode.width,
      height: display.currentMode.height,
      refreshRate: display.currentMode.refreshRate,
    } : null,
    x: display.position?.x ?? 0,
    y: display.position?.y ?? 0,
    primary: display.primary,
    rotation: display.rotation || 'normal',
  }));
}

function updateStatus() {
  const system = state.system;
  const status = $('status');
  if (!system) return;
  status.className = `status ${system.canModify ? '' : 'error'}`;
  const arch = system.archLinux ? 'Arch Linux' : system.osName;
  status.innerHTML = `<strong>${escapeHtml(system.message)}</strong><span>${escapeHtml(arch)}</span><span>会话：${escapeHtml(system.sessionType.toUpperCase())}</span><span>DISPLAY：${escapeHtml(system.display || '未设置')}</span><span>已连接：${system.connectedDisplays ?? '—'}</span>`;
}

function render() {
  updateStatus();
  renderCanvas();
  renderCards();
  const editable = Boolean(state.system?.canModify) && !state.pending;
  $('autoArrange').disabled = !editable || state.draft.length < 2;
  $('reset').disabled = !editable;
  $('validate').disabled = !editable;
  $('apply').disabled = !editable;
  $('dirtyText').textContent = state.pending ? '配置等待确认' : '可修改后验证并应用';
}

function renderCanvas() {
  const canvas = $('canvas');
  const enabled = state.draft.filter((item) => item.enabled);
  if (!enabled.length) {
    canvas.innerHTML = '<p>当前草稿将关闭所有显示器（后端会拒绝应用）</p>';
    return;
  }
  const details = new Map(state.displays.map((display) => [display.name, display]));
  const minX = Math.min(...enabled.map((item) => item.x));
  const minY = Math.min(...enabled.map((item) => item.y));
  const dimensions = enabled.map((item) => {
    const display = details.get(item.output);
    const mode = item.mode || preferredMode(display) || { width: 1024, height: 768 };
    const rotated = item.rotation === 'left' || item.rotation === 'right';
    return { item, width: rotated ? mode.height : mode.width, height: rotated ? mode.width : mode.height };
  });
  const maxX = Math.max(...dimensions.map(({ item, width }) => item.x - minX + width));
  const maxY = Math.max(...dimensions.map(({ item, height }) => item.y - minY + height));
  const scale = Math.min(0.12, 760 / Math.max(maxX, 1), 235 / Math.max(maxY, 1));
  canvas.innerHTML = '';
  dimensions.forEach(({ item, width, height }) => {
    const screen = document.createElement('div');
    screen.className = `screen${item.primary ? ' primary-screen' : ''}`;
    screen.style.left = `${24 + (item.x - minX) * scale}px`;
    screen.style.top = `${24 + (item.y - minY) * scale}px`;
    screen.style.width = `${Math.max(120, width * scale)}px`;
    screen.style.height = `${Math.max(76, height * scale)}px`;
    const mode = item.mode ? modeKey(item.mode.width, item.mode.height) : '自动';
    screen.innerHTML = `<strong>${escapeHtml(item.output)}</strong><small>${mode} · ${item.x},${item.y}</small>${item.primary ? '<span class="primary-badge">PRIMARY</span>' : ''}`;
    screen.title = `拖动 ${item.output} 调整位置`;
    screen.addEventListener('pointerdown', (event) => beginScreenDrag(event, screen, item, dimensions, scale));
    canvas.appendChild(screen);
  });
}

function beginScreenDrag(event, element, item, dimensions, scale) {
  if (state.pending || !state.system?.canModify || event.button !== 0) return;
  event.preventDefault();
  element.setPointerCapture(event.pointerId);
  element.classList.add('dragging');
  const start = { pointerX: event.clientX, pointerY: event.clientY, x: item.x, y: item.y };
  let deltaX = 0;
  let deltaY = 0;

  const move = (moveEvent) => {
    deltaX = (moveEvent.clientX - start.pointerX) / scale;
    deltaY = (moveEvent.clientY - start.pointerY) / scale;
    element.style.transform = `translate(${deltaX * scale}px, ${deltaY * scale}px)`;
    const label = element.querySelector('small');
    label.textContent = `${item.mode ? modeKey(item.mode.width, item.mode.height) : '自动'} · ${Math.max(0, Math.round(start.x + deltaX))},${Math.max(0, Math.round(start.y + deltaY))}`;
  };

  const finish = () => {
    element.removeEventListener('pointermove', move);
    element.removeEventListener('pointerup', finish);
    element.removeEventListener('pointercancel', cancel);
    const own = dimensions.find((entry) => entry.item === item);
    const rawX = Math.max(0, Math.round(start.x + deltaX));
    const rawY = Math.max(0, Math.round(start.y + deltaY));
    const snapped = snapPosition(rawX, rawY, own, dimensions, 18 / scale);
    item.x = snapped.x;
    item.y = snapped.y;
    normalizeDraftCoordinates();
    render();
  };
  const cancel = () => render();
  element.addEventListener('pointermove', move);
  element.addEventListener('pointerup', finish);
  element.addEventListener('pointercancel', cancel);
}

function snapPosition(x, y, own, dimensions, threshold) {
  const xCandidates = [x];
  const yCandidates = [y];
  dimensions.filter((entry) => entry !== own).forEach((other) => {
    xCandidates.push(other.item.x - own.width, other.item.x + other.width, other.item.x, other.item.x + other.width - own.width);
    yCandidates.push(other.item.y - own.height, other.item.y + other.height, other.item.y, other.item.y + other.height - own.height);
  });
  const nearest = (value, candidates) => candidates
    .filter((candidate) => candidate >= 0 && Math.abs(candidate - value) <= threshold)
    .sort((left, right) => Math.abs(left - value) - Math.abs(right - value))[0] ?? value;
  return { x: Math.round(nearest(x, xCandidates)), y: Math.round(nearest(y, yCandidates)) };
}

function normalizeDraftCoordinates() {
  const enabled = state.draft.filter((item) => item.enabled);
  if (!enabled.length) return;
  const minX = Math.min(...enabled.map((item) => item.x));
  const minY = Math.min(...enabled.map((item) => item.y));
  enabled.forEach((item) => {
    item.x -= minX;
    item.y -= minY;
  });
}

function renderCards() {
  const list = $('displayList');
  list.innerHTML = '';
  state.displays.forEach((display) => {
    if (!display.connected) {
      const card = document.createElement('article');
      card.className = 'display-card disconnected';
      card.innerHTML = `<div class="card-header"><div class="output-name"><h3>${escapeHtml(display.name)}</h3><span class="badge">未连接</span></div></div>`;
      list.appendChild(card);
      return;
    }
    const item = state.draft.find((draft) => draft.output === display.name);
    const card = document.createElement('article');
    card.className = 'display-card';
    const selectedMode = item.mode ? modeKey(item.mode.width, item.mode.height) : 'auto';
    const modeOptions = ['<option value="auto">自动 / 推荐</option>', ...display.modes.map((mode) => `<option value="${modeKey(mode.width, mode.height)}"${selectedMode === modeKey(mode.width, mode.height) ? ' selected' : ''}>${mode.width} × ${mode.height}${mode.preferred ? '（推荐）' : ''}</option>`)].join('');
    const selectedModeData = item.mode && display.modes.find((mode) => mode.width === item.mode.width && mode.height === item.mode.height);
    const selectedRate = item.mode?.refreshRate;
    const rateOptions = selectedModeData?.refreshRates.length
      ? selectedModeData.refreshRates.map((rate) => `<option value="${rate.value}"${selectedRate != null && Math.abs(rate.value - selectedRate) < .05 ? ' selected' : ''}>${rateLabel(rate.value)} Hz${rate.preferred ? '（推荐）' : ''}</option>`).join('')
      : '<option value="">自动</option>';
    card.innerHTML = `
      <div class="card-header"><div class="output-name"><h3>${escapeHtml(display.name)}</h3><span class="badge on">已连接</span></div><label class="toggle"><input type="checkbox" data-field="enabled" ${item.enabled ? 'checked' : ''}>启用</label></div>
      <div class="form-grid">
        <div class="field mode"><label>分辨率</label><select data-field="mode" ${!item.enabled ? 'disabled' : ''}>${modeOptions}</select></div>
        <div class="field"><label>刷新率</label><select data-field="rate" ${!item.enabled || !item.mode ? 'disabled' : ''}>${rateOptions}</select></div>
        <div class="field"><label>X 坐标</label><input type="number" min="0" max="32767" data-field="x" value="${item.x}" ${!item.enabled ? 'disabled' : ''}></div>
        <div class="field"><label>Y 坐标</label><input type="number" min="0" max="32767" data-field="y" value="${item.y}" ${!item.enabled ? 'disabled' : ''}></div>
        <div class="field"><label>旋转</label><select data-field="rotation" ${!item.enabled ? 'disabled' : ''}><option value="normal">正常</option><option value="left">左转 90°</option><option value="right">右转 90°</option><option value="inverted">旋转 180°</option></select></div>
      </div>
      <label class="toggle" style="margin-top:14px"><input type="radio" name="primary" data-field="primary" ${item.primary ? 'checked' : ''} ${!item.enabled ? 'disabled' : ''}>设为主显示器</label>`;
    card.querySelector('[data-field="rotation"]').value = item.rotation;
    card.querySelectorAll('input,select').forEach((input) => input.addEventListener('change', () => updateDraft(display, item, input)));
    list.appendChild(card);
  });
}

function updateDraft(display, item, input) {
  const field = input.dataset.field;
  if (field === 'enabled') {
    item.enabled = input.checked;
    if (item.enabled && !item.mode && display.modes.length) {
      const mode = preferredMode(display);
      item.mode = { width: mode.width, height: mode.height, refreshRate: preferredRate(mode) };
    }
    if (!item.enabled && item.primary) {
      item.primary = false;
      const replacement = state.draft.find((draft) => draft.enabled && draft !== item);
      if (replacement) replacement.primary = true;
    }
  } else if (field === 'primary') {
    state.draft.forEach((draft) => draft.primary = draft === item);
  } else if (field === 'mode') {
    if (input.value === 'auto') item.mode = null;
    else {
      const [width, height] = input.value.split('x').map(Number);
      const mode = display.modes.find((candidate) => candidate.width === width && candidate.height === height);
      item.mode = { width, height, refreshRate: preferredRate(mode) };
    }
  } else if (field === 'rate') {
    item.mode.refreshRate = input.value ? Number(input.value) : null;
  } else if (field === 'x' || field === 'y') {
    item[field] = Number(input.value);
  } else if (field === 'rotation') item.rotation = input.value;
  render();
}

async function load() {
  if (!$('status')) return;
  $('status').className = 'status loading';
  $('status').textContent = '正在连接本地服务…';
  try {
    const [system, response] = await Promise.all([api('/api/system/status'), api('/api/displays')]);
    state.system = system;
    state.displays = response.displays;
    state.draft = draftFromDisplays(response.displays);
    state.pending = response.pending;
    render();
    if (state.pending) beginConfirmation(state.pending);
  } catch (error) {
    try { state.system = await api('/api/system/status'); } catch { state.system = null; }
    if (state.system) updateStatus();
    else { $('status').className = 'status error'; $('status').textContent = error.message; }
    notify(error.message, true);
  }
}

function autoArrange() {
  let x = 0;
  state.draft.filter((item) => item.enabled).forEach((item, index) => {
    const display = state.displays.find((candidate) => candidate.name === item.output);
    const mode = preferredMode(display);
    if (mode) item.mode = { width: mode.width, height: mode.height, refreshRate: preferredRate(mode) };
    item.x = x; item.y = 0; item.primary = index === 0;
    const width = item.mode?.width || 1024;
    x += (item.rotation === 'left' || item.rotation === 'right') ? (item.mode?.height || 768) : width;
  });
  render();
}

async function validateDraft() {
  const result = await api('/api/displays/validate', { method: 'POST', body: JSON.stringify({ displays: state.draft }) });
  if (result.valid) notify(result.warnings.length ? `配置有效；警告：${result.warnings.join('；')}` : '配置验证通过');
  else notify(result.errors.join('；'), true);
  return result.valid;
}

async function applyDraft() {
  $('apply').disabled = true;
  try {
    const result = await api('/api/displays/apply', { method: 'POST', body: JSON.stringify({ displays: state.draft }) });
    state.displays = result.displays;
    state.draft = draftFromDisplays(result.displays);
    state.pending = { token: result.token, expiresAt: result.expiresAt };
    render(); beginConfirmation(state.pending); notify(result.message);
  } catch (error) { notify(error.message, true); render(); }
}

function beginConfirmation(pending) {
  clearInterval(state.timer);
  $('confirmBar').classList.remove('hidden');
  const tick = () => {
    const left = Math.max(0, pending.expiresAt - Math.floor(Date.now() / 1000));
    $('countdown').textContent = left;
    if (left === 0) {
      clearInterval(state.timer);
      $('confirmBar').classList.add('hidden');
      state.pending = null;
      setTimeout(() => load().then(() => notify('确认超时，已恢复之前的设置')), 700);
    }
  };
  tick(); state.timer = setInterval(tick, 250);
}

async function finishPending(action) {
  if (!state.pending) return;
  const endpoint = action === 'confirm' ? 'confirm' : 'revert';
  try {
    const result = await api(`/api/displays/${endpoint}`, { method: 'POST', body: JSON.stringify(state.pending) });
    clearInterval(state.timer); state.pending = null; $('confirmBar').classList.add('hidden');
    state.displays = result.displays; state.draft = draftFromDisplays(result.displays); render(); notify(result.message);
  } catch (error) { notify(error.message, true); }
}

function escapeHtml(value) {
  return String(value).replace(/[&<>'"]/g, (character) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', "'": '&#39;', '"': '&quot;' })[character]);
}

async function selectWallpaper() {
  const button = $('wallpaperSelect');
  button.disabled = true;
  try {
    const result = await api('/api/wallpaper/select', { method: 'POST', body: '{}' });
    if (!result.success) { notify(result.message); return; }
    $('wallpaperPath').value = result.path || '';
    showWallpaperPreview(result.preview);
    notify('已载入所选壁纸');
  } catch (error) { notify(error.message, true); }
  finally { button.disabled = false; }
}

function showWallpaperPreview(source) {
  const image = $('wallpaperPreview');
  const empty = $('wallpaperPreviewEmpty');
  if (!image || !empty) return;
  image.src = source || '';
  image.classList.toggle('hidden', !source);
  empty.classList.toggle('hidden', Boolean(source));
}

async function setWallpaper() {
  const path = $('wallpaperPath')?.value.trim();
  if (!path) { notify('请先填写图片路径', true); return; }
  const button = $('wallpaperSet');
  button.disabled = true;
  try {
    const result = await api('/api/wallpaper/set', { method: 'POST', body: JSON.stringify({ path, mode: $('wallpaperMode').value, persist: $('wallpaperPersist').checked }) });
    showWallpaperPreview(result.preview);
    notify(result.message);
  } catch (error) { notify(error.message, true); }
  finally { button.disabled = false; }
}

async function openWallpaperSearch() {
  try { await api('/api/wallpaper/search', { method: 'POST', body: '{}' }); notify('已在默认浏览器中打开 Pexels 壁纸库'); }
  catch (error) { notify(error.message, true); }
}

function route() {
  const name = location.hash.slice(1) || 'home';
  const active = ['home', 'display', 'wallpaper'].includes(name) ? name : 'home';
  document.querySelectorAll('.view').forEach((view) => view.classList.toggle('hidden', view.id !== `view-${active}`));
  document.querySelectorAll('[data-route]').forEach((link) => link.classList.toggle('active', link.dataset.route === active));
  if (active === 'display') load();
}

document.querySelectorAll('[data-route]').forEach((link) => link.addEventListener('click', () => { if (link.getAttribute('href')) return; location.hash = link.dataset.route; }));
window.addEventListener('hashchange', route);
$('refresh')?.addEventListener('click', load);
$('reset')?.addEventListener('click', () => { state.draft = draftFromDisplays(state.displays); render(); });
$('autoArrange').addEventListener('click', autoArrange);
$('validate').addEventListener('click', () => validateDraft().catch((error) => notify(error.message, true)));
$('apply').addEventListener('click', applyDraft);
$('confirm').addEventListener('click', () => finishPending('confirm'));
$('revert')?.addEventListener('click', () => finishPending('revert'));
$('wallpaperSelect')?.addEventListener('click', selectWallpaper);
$('wallpaperSet')?.addEventListener('click', setWallpaper);
$('wallpaperPath')?.addEventListener('change', () => showWallpaperPreview(null));
$('wallpaperSearchSecondary')?.addEventListener('click', openWallpaperSearch);
route();
