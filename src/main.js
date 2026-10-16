// withGlobalTauri: true なので、window.__TAURI__ から Tauri の API を使える
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
let settings = null;
let hotkeyRaw = ''; // 登録するキーの文字列（例: Ctrl+Shift+KeyK）。欄には見やすい形（Ctrl+Shift+K）で出す
const prettyKey = (s) => s.replace(/(^|\+)(Key|Digit)(?=\w$)/g, '$1');

function stamp(ms) {
  const d = new Date(ms);
  return d.toTimeString().slice(0, 8) + '.' + String(d.getMilliseconds()).padStart(3, '0');
}

function addLine(line) {
  const pre = $('log');
  pre.textContent += stamp(line.t) + '  ' + line.msg + '\n';
  const lines = pre.textContent.split('\n');
  if (lines.length > 300) pre.textContent = lines.slice(-300).join('\n');
  pre.scrollTop = pre.scrollHeight;
}

function applyOpacity(v) {
  document.documentElement.style.setProperty('--alpha', String(v));
}

// 画面の設定値を settings に取り込み、見た目（透明度）にはすぐ反映する。保存はしない。
function readControls() {
  settings = {
    ...settings,
    anchor: $('anchor').value,
    hideOnBlur: $('hideOnBlur').checked,
    guardMs: Number($('guardMs').value) || 0,
    opacity: Number($('opacity').value),
    hotkey: hotkeyRaw,
    searchOnHotkey: $('searchOnHotkey').checked,
  };
  applyOpacity(settings.opacity);
  $('opacityValue').textContent = settings.opacity.toFixed(2);
}

async function saveNow() {
  readControls();
  try {
    await invoke('set_settings', { settings });
  } catch (e) {
    addLine({ t: Date.now(), msg: '設定の保存に失敗: ' + e });
  }
}

// スライダーを動かしている間は、値が変わるたびに入力イベントが出る。
// そのたびにファイルへ書かず、操作が止まって300ms後に1回だけ保存する。
let saveTimer = null;
function saveSoon() {
  readControls();
  clearTimeout(saveTimer);
  saveTimer = setTimeout(saveNow, 300);
}

async function init() {
  // 画面を読み込む前に出たログも取り出す（初回の表示ログなど）
  const past = await invoke('get_logs');
  past.forEach(addLine);
  await listen('ddl-log', (e) => addLine(e.payload));

  settings = await invoke('get_settings');
  $('anchor').value = settings.anchor;
  $('hideOnBlur').checked = settings.hideOnBlur;
  $('guardMs').value = settings.guardMs;
  $('opacity').value = settings.opacity;
  hotkeyRaw = settings.hotkey || '';
  $('hotkey').value = prettyKey(hotkeyRaw);
  $('searchOnHotkey').checked = settings.searchOnHotkey !== false;
  applyOpacity(settings.opacity);
  $('opacityValue').textContent = Number(settings.opacity).toFixed(2);

  for (const id of ['hideOnBlur', 'guardMs', 'opacity', 'searchOnHotkey']) {
    $(id).addEventListener('input', saveSoon);
  }
  $('anchor').addEventListener('change', async () => {
    await saveNow();
    invoke('preview_position', { anchor: settings.anchor });
  });
  $('quit').addEventListener('click', async () => {
    if (window.flushTiles) await window.flushTiles(); // 保存待ちのタイル配置を先に書く
    invoke('quit_app');
  });
  // ---- ホットキーの登録欄：欄を選んでキーを押すと、その組み合わせが入る
  const hkIn = $('hotkey');
  const hkMsg = $('hotkeyMsg');
  const HK_HELP = hkMsg.textContent;
  const MODS = ['Control', 'Shift', 'Alt', 'Meta'];
  let hkDone = false; // この入力で、登録を確定した（または解除した）か
  async function applyHotkey(next) {
    hkDone = true;
    const prev = hotkeyRaw;
    try {
      await invoke('apply_hotkey', { accel: next });
      hotkeyRaw = next;
      hkIn.value = prettyKey(next);
      await saveNow();
      hkMsg.textContent = next ? '登録しました: ' + prettyKey(next) : '解除しました';
      hkIn.blur(); // 欄に残したままだと、同じキーを押したときにウィンドウが開閉してしまう
    } catch (e) {
      hkIn.value = prettyKey(prev);
      try { await invoke('apply_hotkey', { accel: prev }); } catch (_) {}
      hkMsg.textContent = '登録できませんでした（他で使われている、または使えないキー）: ' + e;
    }
  }
  hkIn.addEventListener('keydown', (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape') { hkIn.blur(); return; } // 取り消し
    if (MODS.includes(e.key)) return; // 修飾キーだけでは確定しない
    const fkey = /^F([1-9]|1[0-9]|2[0-4])$/.test(e.code);
    if (!(e.ctrlKey || e.altKey || e.shiftKey || e.metaKey) && !fkey) {
      hkMsg.textContent = 'Ctrl / Alt / Shift のどれかと組み合わせてください。';
      return;
    }
    // global-hotkey の書式: 修飾キーを先に、最後に1つのキー。キー名は e.code（KeyS, Digit1, Space, F5 …）
    const parts = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');
    if (e.metaKey) parts.push('Super');
    parts.push(e.code);
    applyHotkey(parts.join('+'));
  });
  // 入力中は、いま登録済みのキーを一時的に解除する（押した組み合わせで、ウィンドウが開閉してしまわないように）
  hkIn.addEventListener('focus', () => {
    hkDone = false;
    hkMsg.textContent = 'キーの組み合わせを押してください（Esc で取り消し）';
    invoke('apply_hotkey', { accel: '' }).catch(() => {});
  });
  hkIn.addEventListener('blur', () => {
    if (!hkDone) invoke('apply_hotkey', { accel: hotkeyRaw }).catch(() => {}); // 取り消し：元に戻す
    if (hkMsg.textContent.startsWith('キーの組み合わせ')) hkMsg.textContent = HK_HELP;
  });
  $('hotkeyClear').addEventListener('click', () => applyHotkey(''));

  // ---- 設定ダイアログ
  const dlg = $('dialog');
  const showTab = (name) => {
    document.querySelectorAll('.tab').forEach((b) => b.classList.toggle('active', b.dataset.tab === name));
    document.querySelectorAll('.pane').forEach((p) => { p.hidden = p.dataset.pane !== name; });
    if (name === 'log') $('log').scrollTop = $('log').scrollHeight;
  };
  const openDialog = async (tab) => {
    try { $('autostart').checked = await invoke('get_autostart'); } catch (_) {}
    showTab(tab || 'general');
    dlg.hidden = false;
  };
  const closeDialog = () => { dlg.hidden = true; };
  document.querySelectorAll('.tab').forEach((b) => b.addEventListener('click', () => showTab(b.dataset.tab)));
  $('openSettings').addEventListener('click', () => openDialog('general'));
  $('dialogClose').addEventListener('click', closeDialog);
  dlg.addEventListener('mousedown', (e) => { if (e.target === dlg) closeDialog(); }); // 外側クリックで閉じる
  await listen('ddl-open-settings', () => openDialog('general')); // トレイの「設定…」
  $('autostart').addEventListener('change', async () => {
    const want = $('autostart').checked;
    try {
      await invoke('set_autostart', { enable: want });
    } catch (e) {
      $('autostart').checked = !want;
      addLine({ t: Date.now(), msg: '自動起動の切り替えに失敗: ' + e });
    }
  });

  // ---- バックアップ
  const bmsg = (t) => { $('backupMsg').textContent = t; };
  $('backupExport').addEventListener('click', async () => {
    bmsg('');
    try {
      if (window.flushTiles) await window.flushTiles(); // 保存待ちのタイル配置を先に書く
      await saveNow();
      const path = await invoke('export_backup');
      bmsg(path ? '書き出しました: ' + path : '');
    } catch (e) { bmsg('書き出せませんでした: ' + e); }
  });
  $('backupImport').addEventListener('click', async () => {
    bmsg('');
    try {
      if (window.flushTiles) await window.flushTiles();
      const res = await invoke('import_backup');
      if (res) { bmsg(res); setTimeout(() => location.reload(), 600); } // 読み込んだ内容で画面を作り直す
    } catch (e) { bmsg('読み込めませんでした: ' + e); }
  });

  window.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return;
    if (!dlg.hidden) { closeDialog(); return; } // ダイアログが開いていれば、それだけ閉じる
    invoke('hide_me');
  });

  window.initRail(); // 左端の列（読み込みを待たない）
  window.initBookmarks();
  await window.initTiles(); // 一覧の読み込み(1秒ほど)を待たずに、タイルを先に出す
  await window.initApps();
}

init();
