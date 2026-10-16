// 🔎検索。左の一覧と同じアプリを、入力で絞り込んで Enter で起動する。
// 並び順: 前方一致 → 単語の頭が一致 → 途中一致。同じ順位の中では名前の短い順。
(() => {
  const invoke = window.__TAURI__.core.invoke;
  const collator = new Intl.Collator('ja', { sensitivity: 'base', numeric: true });
  const MAX = 60;

  // 大文字小文字・全角半角・カタカナ/ひらがなの違いを無視して比べる
  function norm(s) {
    return s.normalize('NFKC').toLowerCase()
      .replace(/[ァ-ヶ]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0x60));
  }

  // 0: 前方一致, 1: 単語の頭に一致, 2: 途中に一致, -1: 一致しない
  function rankOf(name, token) {
    const i = name.indexOf(token);
    if (i < 0) return -1;
    if (i === 0) return 0;
    return /[\s\-_.()（）【】\[\]]/.test(name[i - 1]) ? 1 : 2;
  }

  // apps: [{name, path}]。query: 入力文字列。空白で区切ると、すべてを含むものだけ残す
  function search(apps, query) {
    const tokens = norm(query).split(/\s+/).filter(Boolean);
    if (!tokens.length) return [];
    const out = [];
    for (const a of apps) {
      const name = norm(a.name);
      let rank = -1;
      let ok = true;
      for (let k = 0; k < tokens.length; k++) {
        const r = rankOf(name, tokens[k]);
        if (r < 0) { ok = false; break; }
        if (k === 0) rank = r;
      }
      if (ok) out.push({ a, rank, len: name.length });
    }
    out.sort((x, y) => x.rank - y.rank || x.len - y.len || collator.compare(x.a.name, y.a.name));
    return out.slice(0, MAX).map((x) => x.a);
  }

  window.DDL = window.DDL || {};
  window.DDL.searchApps = search; // テスト用

  const bar = document.getElementById('searchBar');
  const input = document.getElementById('searchInput');
  const appsEl = document.getElementById('apps');
  const results = window.DDL.resultsBox; // apps.js が #apps の中に用意する
  if (!bar || !input || !results) return;

  let sel = -1;
  let current = [];

  function isOpen() { return !bar.hidden; }

  function open(initial) {
    bar.hidden = false;
    appsEl.classList.add('searching');
    input.focus();
    if (typeof initial === 'string') {
      input.value = initial; // 打った1文字目。続きを打てるよう、全選択はしない
      input.setSelectionRange(initial.length, initial.length);
    } else {
      input.select(); // 前の入力が残っていれば、打てば置き換わる
    }
    update();
  }

  function close() {
    bar.hidden = true;
    appsEl.classList.remove('searching');
    input.value = '';
    results.textContent = '';
    current = [];
    sel = -1;
  }

  function highlight() {
    [...results.children].forEach((el, i) => el.classList.toggle('sel', i === sel));
    const el = results.children[sel];
    if (el && el.scrollIntoView) el.scrollIntoView({ block: 'nearest' });
  }

  function update() {
    const apps = window.DDL.allApps ? window.DDL.allApps() : [];
    current = search(apps, input.value);
    results.textContent = '';
    if (!input.value.trim()) {
      const m = document.createElement('div');
      m.className = 'muted searchHint';
      m.textContent = '名前の一部を入力。↑↓で選んで Enter で起動、Esc で閉じる。';
      results.appendChild(m);
      sel = -1;
      return;
    }
    if (!current.length) {
      const m = document.createElement('div');
      m.className = 'muted searchHint';
      m.textContent = '見つかりません';
      results.appendChild(m);
      sel = -1;
      return;
    }
    current.forEach((n) => results.appendChild(window.DDL.makeItem(n)));
    sel = 0;
    highlight();
  }

  function launchSelected() {
    const n = current[sel];
    if (!n) return;
    invoke('launch', { path: n.path }).catch(() => {});
    close();
  }

  input.addEventListener('input', update);
  input.addEventListener('keydown', (e) => {
    if (e.isComposing) return; // 日本語の変換中の Enter / 矢印は変換側に任せる
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (!current.length) return;
      sel = (sel + (e.key === 'ArrowDown' ? 1 : -1) + current.length) % current.length;
      highlight();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      launchSelected();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation(); // ランチャー自体は閉じず、検索だけ閉じる
      close();
    }
  });
  // 結果をクリックして起動したとき（apps.js 側の click が launch する）も検索を閉じる
  results.addEventListener('click', () => setTimeout(close, 0));

  window.DDL.openSearch = () => open(); // ホットキーで開いたとき（Rust から）
  try { window.__TAURI__.event.listen('ddl-open-search', () => open()); } catch (_) {}

  const btn = document.getElementById('openSearch');
  if (btn) btn.addEventListener('click', () => (isOpen() ? close() : open()));

  window.addEventListener('keydown', (e) => {
    const dlg = document.getElementById('dialog');
    if (dlg && !dlg.hidden) return;
    // Ctrl+S（または Ctrl+F）で検索を開く
    if (e.ctrlKey && !e.altKey && !e.metaKey && (e.key === 's' || e.key === 'S' || e.key === 'f' || e.key === 'F')) {
      e.preventDefault();
      open();
      return;
    }
    // 検索を開いていないとき、文字キーを打つと検索が開いて、その文字が入る（日本語入力は🔎か Ctrl+S から）
    if (!isOpen() && !e.ctrlKey && !e.altKey && !e.metaKey && e.key.length === 1 && e.key !== ' ') {
      const t = e.target;
      if (t && /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName)) return;
      e.preventDefault();
      open(e.key);
    }
  });

  // 開き直したとき、前の入力は全選択して、打てばそのまま置き換わるようにする
  window.addEventListener('focus', () => { if (isOpen()) { input.focus(); input.select(); } });
})();
