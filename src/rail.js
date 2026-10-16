// 左端のアイコン列。自分で追加したショートカット（アプリ・フォルダ・ms-settings: などのコマンド）と、下に ⚙ 設定。
(() => {
  const invoke = window.__TAURI__.core.invoke;
  const railEl = document.getElementById('rail');
  const itemsEl = document.getElementById('railItems');
  const listEl = document.getElementById('railList'); // 設定ダイアログの「左端」タブ

  let items = []; // {id, name, path}
  let nextId = 1;

  function logMsg(msg) { invoke('log_msg', { msg }).catch(() => {}); }

  // ---------- 読み込み・保存（画面側で検証する）
  function sanitize(raw) {
    const out = { items: [], nextId: 1 };
    if (!raw || !Array.isArray(raw.items)) return out;
    for (const it of raw.items.slice(0, 50)) {
      if (!it || typeof it.path !== 'string' || !it.path.trim()) continue;
      const name = typeof it.name === 'string' && it.name.trim() ? it.name.trim().slice(0, 100) : it.path;
      out.items.push({ id: out.items.length + 1, name, path: it.path.trim() });
    }
    out.nextId = out.items.length + 1;
    return out;
  }

  async function save() {
    try {
      await invoke('save_rail', { json: JSON.stringify({ version: 1, items: items.map(({ name, path }) => ({ name, path })) }) });
    } catch (e) {
      logMsg('左端の保存に失敗: ' + e);
    }
  }

  function same(a, b) { return a.toLowerCase() === b.toLowerCase(); }
  const has = (path) => items.some((x) => same(x.path, path));

  function changed() { render(); renderList(); save(); }

  function add(name, path) {
    path = (path || '').trim();
    if (!path || has(path)) return false;
    items.push({ id: nextId++, name: (name || '').trim() || path, path });
    changed();
    return true;
  }

  function remove(id) { items = items.filter((x) => x.id !== id); changed(); }

  function move(id, d) {
    const i = items.findIndex((x) => x.id === id);
    const j = i + d;
    if (i < 0 || j < 0 || j >= items.length) return;
    [items[i], items[j]] = [items[j], items[i]];
    changed();
  }

  // ---------- 描画
  function letterIcon(name) {
    const ico = document.createElement('span');
    ico.className = 'ico';
    ico.textContent = (name.trim().charAt(0) || '?').toUpperCase();
    ico.style.background = 'rgba(255,255,255,.18)';
    return ico;
  }

  function fillIcon(ico, path) {
    if (!window.DDL || !window.DDL.loadIcon) return;
    window.DDL.loadIcon(path, 32).then((url) => {
      if (!url) return; // コマンドなど取れないものは頭文字のまま
      const img = document.createElement('img');
      img.src = url;
      img.alt = '';
      ico.textContent = '';
      ico.style.background = 'none';
      ico.appendChild(img);
    });
  }

  function render() {
    itemsEl.textContent = '';
    for (const it of items) {
      const b = document.createElement('button');
      b.className = 'railBtn';
      b.title = it.name;
      const ico = letterIcon(it.name);
      b.appendChild(ico);
      fillIcon(ico, it.path);
      b.addEventListener('click', () => invoke('launch', { path: it.path }).catch(() => {}));
      b.addEventListener('contextmenu', (e) => {
        e.preventDefault();
        window.DDL.showMenu(e.clientX, e.clientY, [
          { label: '左端から外す', fn: () => remove(it.id) },
          '-',
          { label: '上へ', fn: () => move(it.id, -1) },
          { label: '下へ', fn: () => move(it.id, 1) },
        ]);
      });
      itemsEl.appendChild(b);
    }
  }

  // 設定ダイアログの「左端」タブ：一覧（並べ替え・削除）と、名前＋パス/コマンドでの追加
  function renderList() {
    if (!listEl) return;
    listEl.textContent = '';
    if (!items.length) {
      const m = document.createElement('div');
      m.className = 'muted';
      m.textContent = 'まだ何も登録されていません。';
      listEl.appendChild(m);
    }
    for (const it of items) {
      const row = document.createElement('div');
      row.className = 'railRow';
      const t = document.createElement('div');
      t.className = 'railRowText';
      t.innerHTML = '<div class="n"></div><div class="p muted"></div>';
      t.querySelector('.n').textContent = it.name;
      t.querySelector('.p').textContent = it.path;
      row.appendChild(t);
      for (const [label, fn] of [['↑', () => move(it.id, -1)], ['↓', () => move(it.id, 1)], ['✕', () => remove(it.id)]]) {
        const b = document.createElement('button');
        b.textContent = label;
        b.addEventListener('click', fn);
        row.appendChild(b);
      }
      listEl.appendChild(row);
    }
  }

  // ---------- ドロップ・右クリックからの追加用
  function contains(cssX, cssY) {
    const r = railEl.getBoundingClientRect();
    return cssX >= r.left && cssX <= r.right && cssY >= r.top && cssY <= r.bottom;
  }

  window.DDL = window.DDL || {};
  window.DDL.railContains = contains;
  window.DDL.railAdd = add;
  window.DDL.railHas = has;
  // ドラッグ中、左端の上に来たら光らせる（p: Tauri のドロップイベントの payload。位置は物理ピクセル）
  window.DDL.railHover = (p) => {
    if (p.type === 'leave' || p.type === 'drop' || !p.position) {
      railEl.classList.remove('drop-over');
      return;
    }
    const dpr = window.devicePixelRatio || 1;
    const x = p.position.x / dpr, y = p.position.y / dpr;
    const overBm = window.DDL.bookmarkBtnContains && window.DDL.bookmarkBtnContains(x, y);
    railEl.classList.toggle('drop-over', !overBm && contains(x, y)); // 🔖 の上では、左端の列は光らせない
  };
  // 一覧のアプリを右クリックしたときのメニュー項目
  window.DDL.railMenuItems = (node) => {
    const exists = items.find((x) => same(x.path, node.path));
    return exists
      ? { label: '左端から外す', fn: () => remove(exists.id) }
      : { label: '左端に追加する', fn: () => add(node.name, node.path) };
  };

  window.initRail = async () => {
    try {
      const raw = await invoke('load_rail');
      const s = sanitize(raw ? JSON.parse(raw) : null);
      items = s.items;
      nextId = s.nextId;
    } catch (e) {
      logMsg('左端の読み込みに失敗: ' + e);
    }
    render();
    renderList();
    const nameIn = document.getElementById('railName');
    const pathIn = document.getElementById('railPath');
    const addBtn = document.getElementById('railAdd');
    if (addBtn) {
      addBtn.addEventListener('click', () => {
        if (add(nameIn.value, pathIn.value)) { nameIn.value = ''; pathIn.value = ''; }
      });
    }
  };
})();
