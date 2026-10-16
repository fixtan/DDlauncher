// ブックマーク。左端の 🔖 を押すとリストが出る。アプリ・フォルダ・コマンドを、グループ（フォルダ）に分けて並べられる。
// 編集は 設定 →「ブックマーク」タブ。アプリ/フォルダは 🔖 へのドロップ、一覧の右クリックからも追加できる。
(() => {
  const invoke = window.__TAURI__.core.invoke;
  const btn = document.getElementById('openBookmarks');
  const flyout = document.getElementById('flyout');
  const listEl = document.getElementById('bmList');
  const MAX_DEPTH = 4;
  const MAX_NODES = 500;

  // ノード: {id, t: 'dir'|'app'|'folder'|'cmd', name, path?, cmd?, cwd?, keep?, children?}
  let root = [];
  let nextId = 1;
  let editingId = null;

  function logMsg(msg) { invoke('log_msg', { msg }).catch(() => {}); }

  // ---------- 読み込み・保存（画面側で検証する）
  function sanitize(raw) {
    let count = 0;
    function walk(list, depth) {
      const out = [];
      if (!Array.isArray(list)) return out;
      for (const n of list) {
        if (!n || typeof n !== 'object' || count >= MAX_NODES) continue;
        const name = typeof n.name === 'string' ? n.name.trim().slice(0, 100) : '';
        if (n.t === 'dir') {
          if (!name || depth >= MAX_DEPTH) continue;
          count++;
          out.push({ id: nextId++, t: 'dir', name, children: walk(n.children, depth + 1) });
        } else if (n.t === 'app' || n.t === 'folder') {
          if (typeof n.path !== 'string' || !n.path.trim()) continue;
          count++;
          out.push({ id: nextId++, t: n.t, name: name || n.path, path: n.path.trim() });
        } else if (n.t === 'cmd') {
          if (typeof n.cmd !== 'string' || !n.cmd.trim()) continue;
          count++;
          out.push({
            id: nextId++, t: 'cmd', name: name || n.cmd.trim().split('\n')[0].slice(0, 40),
            cmd: n.cmd, cwd: typeof n.cwd === 'string' ? n.cwd.trim() : '', keep: n.keep !== false,
          });
        }
      }
      return out;
    }
    nextId = 1;
    return walk(raw && raw.nodes, 0);
  }

  function plain(list) {
    return list.map((n) => {
      if (n.t === 'dir') return { t: 'dir', name: n.name, children: plain(n.children) };
      if (n.t === 'cmd') return { t: 'cmd', name: n.name, cmd: n.cmd, cwd: n.cwd, keep: n.keep };
      return { t: n.t, name: n.name, path: n.path };
    });
  }

  async function save() {
    try {
      await invoke('save_bookmarks', { json: JSON.stringify({ version: 1, nodes: plain(root) }) });
    } catch (e) {
      logMsg('ブックマークの保存に失敗: ' + e);
    }
  }

  // ---------- ツリー操作
  function findNode(id, list = root, parent = null) {
    for (let i = 0; i < list.length; i++) {
      if (list[i].id === id) return { node: list[i], list, index: i, parent };
      if (list[i].children) {
        const r = findNode(id, list[i].children, list[i]);
        if (r) return r;
      }
    }
    return null;
  }

  function depthOf(id) {
    let d = 0;
    (function walk(list, k) {
      for (const n of list) {
        if (n.id === id) d = k;
        if (n.children) walk(n.children, k + 1);
      }
    })(root, 0);
    return d;
  }

  function countNodes(list = root) {
    return list.reduce((s, n) => s + 1 + (n.children ? countNodes(n.children) : 0), 0);
  }

  function changed() { renderFlyout(); renderList(); save(); }

  function same(a, b) { return a.toLowerCase() === b.toLowerCase(); }
  function hasPath(path, t, list = root) {
    return list.some((n) => (n.path && n.t === t && same(n.path, path)) || (n.children && hasPath(path, t, n.children)));
  }

  function baseName(p) {
    return p.replace(/[\\/]+$/, '').split(/[\\/]/).pop().replace(/\.(exe|lnk|url|bat|cmd|com)$/i, '') || p;
  }

  // 追加（親が null ならルート）。戻り値: 追加できたか
  function addNode(node, parentId = null) {
    if (countNodes() >= MAX_NODES) return false;
    let target = root;
    if (parentId != null) {
      const p = findNode(parentId);
      if (!p || p.node.t !== 'dir') return false;
      target = p.node.children;
    }
    node.id = nextId++;
    target.push(node);
    changed();
    return true;
  }

  function removeNode(id) {
    const r = findNode(id);
    if (!r) return;
    r.list.splice(r.index, 1);
    if (editingId === id) cancelEdit();
    changed();
  }

  function moveNode(id, d) {
    const r = findNode(id);
    if (!r) return;
    const j = r.index + d;
    if (j < 0 || j >= r.list.length) return;
    [r.list[r.index], r.list[j]] = [r.list[j], r.list[r.index]];
    changed();
  }

  // ---------- 起動
  function run(n) {
    closeFlyout();
    if (n.t === 'cmd') {
      invoke('run_command', { command: n.cmd, cwd: n.cwd || null, keepOpen: n.keep !== false }).catch(() => {});
    } else {
      invoke('launch', { path: n.path }).catch(() => {}); // 失敗はログに出る
    }
  }

  // ---------- 🔖 のリスト（フライアウト）
  function iconFor(n) {
    const ico = document.createElement('span');
    ico.className = 'ico';
    if (n.t === 'dir' || n.t === 'folder') { ico.textContent = '📁'; ico.classList.add('folder-ico'); return ico; }
    if (n.t === 'cmd') { ico.textContent = '>_'; ico.style.background = '#3a4150'; ico.style.fontSize = '11px'; return ico; }
    ico.textContent = (n.name.trim().charAt(0) || '?').toUpperCase();
    ico.style.background = 'rgba(255,255,255,.18)';
    if (window.DDL && window.DDL.loadIcon) {
      window.DDL.loadIcon(n.path, 32).then((url) => {
        if (!url) return;
        const img = document.createElement('img');
        img.src = url; img.alt = '';
        ico.textContent = ''; ico.style.background = 'none'; ico.appendChild(img);
      });
    }
    return ico;
  }

  const openDirs = new Set(); // 開いているグループ（この起動中だけ覚える）

  function renderFlyout() {
    flyout.textContent = '';
    if (!root.length) {
      const m = document.createElement('div');
      m.className = 'muted searchHint';
      m.textContent = 'まだ登録がありません。🔖 にファイルやフォルダをドロップするか、設定 →「ブックマーク」から追加できます。';
      flyout.appendChild(m);
      return;
    }
    (function draw(list, into) {
      for (const n of list) {
        const row = document.createElement('div');
        row.className = 'row bmRow';
        row.title = n.t === 'cmd' ? n.cmd : (n.path || n.name);
        row.appendChild(iconFor(n));
        const label = document.createElement('span');
        label.className = 'label';
        label.textContent = n.name;
        row.appendChild(label);
        into.appendChild(row);
        if (n.t === 'dir') {
          const chev = document.createElement('span');
          chev.className = 'chev';
          chev.textContent = '▾';
          row.appendChild(chev);
          const kids = document.createElement('div');
          kids.className = 'kids';
          kids.hidden = !openDirs.has(n.id);
          row.classList.toggle('open', !kids.hidden);
          draw(n.children, kids);
          if (!n.children.length) {
            const e = document.createElement('div');
            e.className = 'muted searchHint';
            e.textContent = '（空）';
            kids.appendChild(e);
          }
          row.addEventListener('click', () => {
            kids.hidden = !kids.hidden;
            row.classList.toggle('open', !kids.hidden);
            if (kids.hidden) openDirs.delete(n.id); else openDirs.add(n.id);
          });
          into.appendChild(kids);
        } else {
          row.addEventListener('click', () => run(n));
        }
      }
    })(root, flyout);
  }

  function isFlyoutOpen() { return !flyout.hidden; }
  function openFlyout() { renderFlyout(); flyout.hidden = false; btn.classList.add('active'); }
  function closeFlyout() { flyout.hidden = true; btn.classList.remove('active'); }
  btn.addEventListener('click', () => (isFlyoutOpen() ? closeFlyout() : openFlyout()));
  document.addEventListener('pointerdown', (e) => {
    if (isFlyoutOpen() && !flyout.contains(e.target) && !btn.contains(e.target)) closeFlyout();
  }, true);
  window.addEventListener('blur', closeFlyout);
  // Esc: リストが開いていれば、それだけ閉じる（ランチャーは閉じない）
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && isFlyoutOpen()) { closeFlyout(); e.stopImmediatePropagation(); e.preventDefault(); }
  }, true);

  // ---------- 設定 →「ブックマーク」タブ
  const $ = (id) => document.getElementById(id);
  const KIND_LABEL = { dir: 'グループ', app: 'アプリ', folder: 'フォルダ', cmd: 'コマンド' };

  function renderList() {
    if (!listEl) return;
    listEl.textContent = '';
    if (!root.length) {
      const m = document.createElement('div');
      m.className = 'muted';
      m.textContent = 'まだ何も登録されていません。';
      listEl.appendChild(m);
    }
    (function draw(list, depth) {
      for (const n of list) {
        const row = document.createElement('div');
        row.className = 'railRow';
        row.style.paddingLeft = depth * 18 + 'px';
        const t = document.createElement('div');
        t.className = 'railRowText';
        t.innerHTML = '<div class="n"></div><div class="p muted"></div>';
        t.querySelector('.n').textContent = (n.t === 'dir' ? '📁 ' : '') + n.name;
        t.querySelector('.p').textContent = KIND_LABEL[n.t] + (n.path ? '  ' + n.path : n.cmd ? '  ' + n.cmd.split('\n')[0] : '');
        row.appendChild(t);
        for (const [label, fn] of [['↑', () => moveNode(n.id, -1)], ['↓', () => moveNode(n.id, 1)], ['編集', () => startEdit(n.id)], ['✕', () => removeNode(n.id)]]) {
          const b = document.createElement('button');
          b.textContent = label;
          b.addEventListener('click', fn);
          row.appendChild(b);
        }
        listEl.appendChild(row);
        if (n.children) draw(n.children, depth + 1);
      }
    })(root, 0);
    fillParents();
  }

  // 「所属」の選択肢：ルートと、すべてのグループ（編集中は自分とその中を除く）
  function fillParents(selected) {
    const sel = $('bmParent');
    if (!sel) return;
    const keep = selected !== undefined ? selected : sel.value;
    sel.textContent = '';
    const add = (value, text) => { const o = document.createElement('option'); o.value = value; o.textContent = text; sel.appendChild(o); };
    add('', 'ルート（一番上）');
    (function walk(list, depth, skip) {
      for (const n of list) {
        if (n.t !== 'dir') continue;
        const hidden = skip || n.id === editingId;
        if (!hidden && depth + 1 < MAX_DEPTH) add(String(n.id), '　'.repeat(depth) + '📁 ' + n.name);
        walk(n.children, depth + 1, hidden);
      }
    })(root, 0, false);
    sel.value = [...sel.options].some((o) => o.value === String(keep)) ? String(keep) : '';
  }

  function syncFormKind() {
    const k = $('bmKind').value;
    $('bmPathRow').hidden = !(k === 'app' || k === 'folder');
    $('bmCmdRows').hidden = k !== 'cmd';
  }

  function resetForm() {
    editingId = null;
    $('bmKind').disabled = false;
    $('bmKind').value = 'app';
    $('bmName').value = ''; $('bmPath').value = ''; $('bmCmd').value = ''; $('bmCwd').value = ''; $('bmKeep').checked = true;
    $('bmSubmit').textContent = '追加';
    $('bmCancel').hidden = true;
    $('bmMsg').textContent = '';
    syncFormKind();
    fillParents('');
  }
  function cancelEdit() { resetForm(); }

  function startEdit(id) {
    const r = findNode(id);
    if (!r) return;
    editingId = id;
    const n = r.node;
    $('bmKind').value = n.t;
    $('bmKind').disabled = true; // 種類は変えない（作り直してもらう）
    $('bmName').value = n.name;
    $('bmPath').value = n.path || '';
    $('bmCmd').value = n.cmd || '';
    $('bmCwd').value = n.cwd || '';
    $('bmKeep').checked = n.keep !== false;
    fillParents(r.parent ? String(r.parent.id) : '');
    $('bmSubmit').textContent = '保存';
    $('bmCancel').hidden = false;
    $('bmMsg').textContent = '「' + n.name + '」を編集中';
    syncFormKind();
    $('bmName').focus();
  }

  function submitForm() {
    const kind = $('bmKind').value;
    const name = $('bmName').value.trim();
    const path = $('bmPath').value.trim();
    const cmd = $('bmCmd').value.replace(/\s+$/, '');
    const msg = (t) => { $('bmMsg').textContent = t; };
    if (kind === 'dir' && !name) return msg('グループの名前を入力してください。');
    if ((kind === 'app' || kind === 'folder') && !path) return msg('パスを入力してください。');
    if (kind === 'cmd' && !cmd.trim()) return msg('コマンドを入力してください。');
    const parentId = $('bmParent').value ? Number($('bmParent').value) : null;
    const base = {
      t: kind,
      name: name || (kind === 'cmd' ? cmd.trim().split('\n')[0].slice(0, 40) : baseName(path)),
    };
    if (kind === 'app' || kind === 'folder') base.path = path;
    if (kind === 'cmd') { base.cmd = cmd; base.cwd = $('bmCwd').value.trim(); base.keep = $('bmKeep').checked; }
    if (kind === 'dir') base.children = [];

    if (editingId == null) {
      if (!addNode(base, parentId)) return msg('追加できませんでした（上限、または所属先が見つかりません）。');
      resetForm();
      msg('追加しました');
      return;
    }
    const r = findNode(editingId);
    if (!r) { resetForm(); return; }
    Object.assign(r.node, base, { id: r.node.id, children: r.node.children });
    if (!r.node.children) delete r.node.children;
    const curParent = r.parent ? r.parent.id : null;
    if (curParent !== parentId) { // 所属を変える
      const target = parentId == null ? root : (findNode(parentId) || {}).node;
      if (target) {
        r.list.splice(r.index, 1);
        (parentId == null ? target : target.children).push(r.node);
      }
    }
    resetForm();
    changed();
    msg('保存しました');
  }

  // ---------- 他からの追加用
  function contains(cssX, cssY) {
    const r = btn.getBoundingClientRect();
    return cssX >= r.left && cssX <= r.right && cssY >= r.top && cssY <= r.bottom;
  }

  window.DDL = window.DDL || {};
  window.DDL.bookmarkBtnContains = contains;
  // ドロップされたファイル/フォルダ。{name, path, isDir}
  window.DDL.bookmarkAddPath = (info) => {
    const t = info.isDir ? 'folder' : 'app';
    if (hasPath(info.path, t)) return false;
    return addNode({ t, name: info.name || baseName(info.path), path: info.path });
  };
  window.DDL.bookmarkMenuItems = (node) => ({
    label: hasPath(node.path, 'app') ? 'ブックマークにあります' : 'ブックマークに追加する',
    fn: () => { if (!hasPath(node.path, 'app')) addNode({ t: 'app', name: node.name, path: node.path }); },
  });
  window.DDL.bookmarkHover = (p) => {
    if (p.type === 'leave' || p.type === 'drop' || !p.position) { btn.classList.remove('drop-over'); return; }
    const dpr = window.devicePixelRatio || 1;
    btn.classList.toggle('drop-over', contains(p.position.x / dpr, p.position.y / dpr));
  };

  window.initBookmarks = async () => {
    try {
      const raw = await invoke('load_bookmarks');
      root = sanitize(raw ? JSON.parse(raw) : null);
    } catch (e) {
      logMsg('ブックマークの読み込みに失敗: ' + e);
    }
    renderFlyout();
    renderList();
    if ($('bmKind')) {
      $('bmKind').addEventListener('change', syncFormKind);
      $('bmSubmit').addEventListener('click', submitForm);
      $('bmCancel').addEventListener('click', cancelEdit);
      resetForm();
    }
  };
})();
