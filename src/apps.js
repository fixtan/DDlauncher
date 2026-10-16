// 左のアプリ一覧。Win10 の「すべてのアプリ」と同じ形（頭文字の見出し、フォルダは展開式）。
(() => {
  const invoke = window.__TAURI__.core.invoke;
  const collator = new Intl.Collator('ja', { sensitivity: 'base', numeric: true });

  // ---------- アイコン取得（同時に4件まで。同じパスは1回だけ）
  const iconCache = new Map(); // path|size -> Promise<dataURL | null>（成功だけ残す）
  const queue = [];
  let active = 0;
  function pump() {
    while (active < 4 && queue.length) {
      const { path, size, resolve } = queue.shift();
      active++;
      invoke('get_icon', { path, size })
        .then(resolve, () => resolve(null))
        .finally(() => { active--; pump(); });
    }
  }
  // size: 表示したい大きさ(px)。同じパス・同じサイズは1回だけ取る
  function loadIcon(path, size = 32) {
    const key = path + '|' + size;
    if (!iconCache.has(key)) {
      const p = new Promise((resolve) => { queue.push({ path, size, resolve }); pump(); });
      iconCache.set(key, p);
      // 失敗（null）は覚えない。あとでもう一度取れるようにする
      p.then((url) => { if (!url && iconCache.get(key) === p) iconCache.delete(key); });
    }
    return iconCache.get(key);
  }

  window.DDL = window.DDL || {};
  window.DDL.loadIcon = loadIcon; // タイルからも使う

  // 画面に見えたものだけ取得する（数百件を最初に全部取らない）
  const root = document.getElementById('apps');
  const seen = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      seen.unobserve(e.target);
      const ico = e.target;
      loadIcon(ico.dataset.path).then((url) => {
        if (!url) return; // 失敗したら頭文字のまま
        const img = document.createElement('img');
        img.src = url;
        img.alt = '';
        ico.textContent = '';
        ico.style.background = 'none';
        ico.appendChild(img);
      });
    }
  }, { root, rootMargin: '200px' });

  // ---------- 見出し
  function groupKey(name) {
    const c = name.trim().charAt(0);
    if (/[A-Za-z]/.test(c)) return c.toUpperCase();
    if (/[\x21-\x40\x5b-\x60\x7b-\x7e]/.test(c)) return '#'; // 数字と記号
    return 'その他';
  }
  function groupOrder(k) {
    if (k === '#') return 0;
    if (k === 'その他') return 2;
    return 1;
  }
  function byName(a, b) { return collator.compare(a.name, b.name); }

  // ---------- 描画
  function hue(name) {
    let h = 0;
    for (const ch of name) h = (h * 31 + ch.codePointAt(0)) % 360;
    return h;
  }
  function makeIcon(node) {
    const ico = document.createElement('span');
    ico.className = 'ico';
    ico.textContent = (node.name.trim().charAt(0) || '?').toUpperCase();
    ico.style.background = `hsl(${hue(node.name)} 45% 38%)`;
    ico.dataset.path = node.path;
    seen.observe(ico);
    return ico;
  }

  function makeItem(node) {
    const row = document.createElement('div');
    row.className = 'row app';
    row.title = node.name;
    row.dataset.path = node.path; // タイル領域へのドラッグ追加で使う
    row.dataset.name = node.name;
    row.appendChild(makeIcon(node));
    const label = document.createElement('span');
    label.className = 'label';
    label.textContent = node.name;
    row.appendChild(label);
    row.addEventListener('click', () => {
      invoke('launch', { path: node.path }).catch(() => {}); // 失敗はログに出る
    });
    row.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      window.dispatchEvent(new CustomEvent('ddl-app-context', {
        detail: { x: e.clientX, y: e.clientY, node },
      }));
    });
    return row;
  }

  function makeFolder(node) {
    const wrap = document.createElement('div');
    const row = document.createElement('div');
    row.className = 'row folder';
    row.innerHTML = '<span class="ico folder-ico">📁</span><span class="label"></span><span class="chev">▾</span>';
    row.querySelector('.label').textContent = node.name;
    const kids = document.createElement('div');
    kids.className = 'kids';
    kids.hidden = true;
    let built = false;
    row.addEventListener('click', () => {
      kids.hidden = !kids.hidden;
      row.classList.toggle('open', !kids.hidden);
      if (!kids.hidden && !built) { // 初めて開くときに中身を作る
        built = true;
        node.children.slice().sort(byName).forEach((c) => kids.appendChild(makeNode(c)));
      }
    });
    wrap.appendChild(row);
    wrap.appendChild(kids);
    return wrap;
  }

  function makeNode(node) {
    return node.children ? makeFolder(node) : makeItem(node);
  }

  function render(tree) {
    const groups = new Map();
    for (const n of tree) {
      const k = groupKey(n.name);
      if (!groups.has(k)) groups.set(k, []);
      groups.get(k).push(n);
    }
    const keys = [...groups.keys()].sort((a, b) =>
      groupOrder(a) - groupOrder(b) || (a < b ? -1 : a > b ? 1 : 0));
    const frag = document.createDocumentFragment();
    for (const k of keys) {
      const h = document.createElement('div');
      h.className = 'heading';
      h.textContent = k;
      frag.appendChild(h);
      groups.get(k).sort(byName).forEach((n) => frag.appendChild(makeNode(n)));
    }
    root.textContent = '';
    root.appendChild(recentBox);
    root.appendChild(frag);
    root.appendChild(resultsBox);
    byPath = new Map();
    (function walk(list) {
      for (const n of list) {
        if (n.children) walk(n.children); else byPath.set(n.path.toLowerCase(), n);
      }
    })(tree);
    refreshRecent();
  }

  // ---------- 最近使ったアプリ（先頭に出す）。履歴は Rust 側の usage.json
  const RECENT_N = 6;
  const recentBox = document.createElement('div');
  // 検索結果の置き場。検索中だけ表示し、アイコンの遅延読み込みが効くよう #apps の中に置く
  const resultsBox = document.createElement('div');
  resultsBox.id = 'results';
  window.DDL.resultsBox = resultsBox;
  window.DDL.makeItem = makeItem;
  window.DDL.allApps = () => [...byPath.values()];
  let byPath = new Map();

  function nodeFor(path) {
    const hit = byPath.get(path.toLowerCase());
    if (hit) return hit;
    // スタートメニューにないもの（ドロップした exe など）はファイル名から作る
    const base = path.split(/[\\/]/).pop().replace(/\.(exe|lnk|url|bat|cmd|com)$/i, '');
    return { name: base || path, path };
  }

  async function refreshRecent() {
    let list = [];
    try { list = (await invoke('get_recent', { limit: RECENT_N })) || []; } catch (_) { list = []; }
    recentBox.textContent = '';
    if (!list.length) return;
    const h = document.createElement('div');
    h.className = 'heading';
    h.textContent = '最近使った';
    recentBox.appendChild(h);
    list.forEach((e) => recentBox.appendChild(makeItem(nodeFor(e.path))));
  }

  // ---------- 読み込み。開くたびに見直し、変わっているときだけ描き直す
  let lastJson = '';
  let lastLoad = 0;
  async function reload(force) {
    if (!force && Date.now() - lastLoad < 30000) return;
    lastLoad = Date.now();
    try {
      const tree = await invoke('list_apps');
      const json = JSON.stringify(tree);
      if (json !== lastJson) {
        lastJson = json;
        render(tree);
      }
    } catch (e) {
      root.textContent = 'アプリ一覧の取得に失敗: ' + e;
    }
  }

  window.initApps = async () => {
    window.addEventListener('focus', () => { reload(false); refreshRecent(); });
    await reload(true);
  };
})();
