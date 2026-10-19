// 右のタイル領域。配置の計算は tile-engine.js、ここは画面と保存。
(() => {
  const invoke = window.__TAURI__.core.invoke;
  const CELL = 60, GAP = 4, STEP = CELL + GAP;
  const state = { cols: 6, gravity: false, tiles: [], nextId: 1 };

  const area = document.getElementById('tileArea');
  const gridLayer = document.getElementById('tileGrid');
  const ghost = document.getElementById('ghost');
  const hint = document.getElementById('emptyHint');
  const menu = document.getElementById('menu');
  const els = new Map();
  let drag = null;
  let gridKey = '';

  const logMsg = (msg) => invoke('log_msg', { msg }).catch(() => {});

  // ---------- 共通の右クリックメニュー（apps.js からも使う）
  function closeMenu() { menu.style.display = 'none'; }
  function showMenu(x, y, items) {
    menu.innerHTML = '';
    for (const it of items) {
      if (it === '-') {
        const hr = document.createElement('div');
        hr.className = 'sep';
        menu.appendChild(hr);
        continue;
      }
      const b = document.createElement('button');
      b.textContent = it.label;
      b.onclick = () => { closeMenu(); it.fn(); };
      menu.appendChild(b);
    }
    menu.style.display = 'flex';
    // 画面の外にはみ出さないよう寄せる
    const r = menu.getBoundingClientRect();
    menu.style.left = Math.max(0, Math.min(x, window.innerWidth - r.width - 2)) + 'px';
    menu.style.top = Math.max(0, Math.min(y, window.innerHeight - r.height - 2)) + 'px';
  }
  window.DDL = window.DDL || {};
  window.DDL.showMenu = showMenu;
  document.addEventListener('pointerdown', (e) => { if (!menu.contains(e.target)) closeMenu(); }, true);
  document.addEventListener('contextmenu', (e) => e.preventDefault()); // ブラウザ標準のメニューは出さない

  // 本家のスタートメニューの「その他」にあたる項目（管理者として実行 / ファイルの場所を開く）。
  // ストアアプリ（shell:AppsFolder\...）には、実体のファイルも管理者実行もないので出さない。
  // 管理者実行は、実行できるもの（exe・ショートカット・バッチ）だけ。フォルダや書類には出さない。
  window.DDL.pathMenuItems = (path) => {
    if (!path || /^shell:/i.test(path)) return [];
    const items = [];
    if (/\.(exe|lnk|bat|cmd|com|msi)$/i.test(path)) {
      items.push({ label: '管理者として実行', fn: () => invoke('launch_admin', { path }).catch(() => {}) });
    }
    items.push({ label: 'ファイルの場所を開く', fn: () => invoke('reveal', { path }).catch(() => {}) });
    return items;
  };

  // ---------- 描画
  function hue(str) {
    let h = 0;
    for (const ch of str) h = (h * 31 + ch.codePointAt(0)) >>> 0;
    return h % 360;
  }

  // タイルの大きさに合わせて、取得するアイコンの大きさを変える（CSS の .icon の大きさと揃える）
  const ICON_PX = { S: 40, M: 64, W: 64, L: 128 };

  function setIcon(el, t) {
    const px = ICON_PX[t.size];
    if (el._iconPx === px) return;
    el._iconPx = px;
    if (!t.path || !window.DDL.loadIcon) return;
    window.DDL.loadIcon(t.path, px).then((url) => {
      if (!url || el._iconPx !== px) return; // 取れなかった／もう別の大きさに変わった
      const icon = el.firstChild;
      const img = document.createElement('img');
      img.src = url;
      img.alt = '';
      icon.textContent = '';
      icon.style.background = 'none';
      icon.appendChild(img);
    });
  }

  function makeTileEl(t) {
    const el = document.createElement('div');
    el.className = 'tile';
    el.dataset.id = t.id;
    el.innerHTML = '<div class="icon"></div><div class="label"></div>';
    const icon = el.firstChild;
    icon.textContent = (t.name || '?').trim().charAt(0).toUpperCase() || '?'; // 取れるまでの頭文字
    icon.style.background = `hsl(${hue(t.name)} 45% 38%)`;
    return el;
  }

  function updateTileEl(el, t) {
    el.dataset.size = t.size;
    setIcon(el, t);
    el.style.width = (t.w * STEP - GAP) + 'px';
    el.style.height = (t.h * STEP - GAP) + 'px';
    const label = el.lastChild;
    if (label.textContent !== t.name) label.textContent = t.name;
    el.title = t.name;
  }

  function drawGrid(rows) {
    const key = state.cols + ':' + rows;
    if (key === gridKey) return;
    gridKey = key;
    gridLayer.innerHTML = '';
    const frag = document.createDocumentFragment();
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < state.cols; c++) {
        const d = document.createElement('div');
        d.className = 'cell';
        d.style.left = c * STEP + 'px';
        d.style.top = r * STEP + 'px';
        d.style.width = CELL + 'px';
        d.style.height = CELL + 'px';
        frag.appendChild(d);
      }
    }
    gridLayer.appendChild(frag);
  }

  function render(layout = state.tiles, dragInfo = null) {
    area.style.width = (state.cols * STEP - GAP) + 'px';
    const maxBottom = layout.reduce((m, t) => Math.max(m, t.y + t.h), 0);
    const rows = Math.max(10, maxBottom + 4);
    area.style.height = (rows * STEP - GAP) + 'px';
    drawGrid(rows);
    hint.style.display = layout.length ? 'none' : 'block';
    area.classList.toggle('dragging-now', !!dragInfo);

    const seen = new Set();
    for (const t of layout) {
      seen.add(t.id);
      let el = els.get(t.id);
      if (!el) {
        el = makeTileEl(t);
        els.set(t.id, el);
        area.appendChild(el);
      }
      updateTileEl(el, t);
      if (dragInfo && t.id === dragInfo.id) {
        el.classList.add('dragging');
        el.style.left = dragInfo.px + 'px';
        el.style.top = dragInfo.py + 'px';
      } else {
        el.classList.remove('dragging');
        el.style.left = t.x * STEP + 'px';
        el.style.top = t.y * STEP + 'px';
      }
    }
    for (const [id, el] of els) {
      if (!seen.has(id)) { el.remove(); els.delete(id); }
    }
    if (dragInfo) {
      const g = layout.find((t) => t.id === dragInfo.id);
      ghost.style.display = 'block';
      ghost.style.left = g.x * STEP + 'px';
      ghost.style.top = g.y * STEP + 'px';
      ghost.style.width = (g.w * STEP - GAP) + 'px';
      ghost.style.height = (g.h * STEP - GAP) + 'px';
    } else {
      ghost.style.display = 'none';
    }
  }

  // ---------- 保存（操作が落ち着いて300ms後。ウィンドウが閉じるとき・終了前にも書く）
  let saveTimer = null;
  let dirty = false;
  function serialize() {
    return {
      version: 1, cols: state.cols, gravity: state.gravity,
      tiles: state.tiles.map(({ id, name, path, size, x, y }) => ({ id, name, path, size, x, y })),
    };
  }
  async function flush() {
    clearTimeout(saveTimer);
    if (!dirty) return;
    dirty = false;
    try {
      await invoke('save_tiles', { json: JSON.stringify(serialize()) });
    } catch (e) {
      dirty = true;
      logMsg('タイルの保存に失敗: ' + e);
    }
  }
  function saveSoon() {
    dirty = true;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(flush, 300);
  }
  window.flushTiles = flush;
  // トレイから終了するとき、保存待ちを先に書く
  try { window.__TAURI__.event.listen('ddl-before-quit', () => flush()); } catch (_) {}
  window.addEventListener('blur', flush);

  function commit() { saveSoon(); render(); }

  // ---------- 追加・削除
  // Windows のパスは大文字小文字を区別しない
  function findByPath(path) {
    const k = path.toLowerCase();
    return state.tiles.find((t) => t.path.toLowerCase() === k);
  }

  // at: {x, y}（マス単位）で置き場所を指定できる。defer: 保存・再描画を呼び出し側がまとめて行う
  function addTile(name, path, size = 'M', at = null, defer = false) {
    const s = SIZES[size];
    const t = { id: state.nextId++, name, path, size, x: 0, y: 0, w: s.w, h: s.h };
    const pos = at || findFree(state.tiles, size, state.cols);
    state.tiles = moveTile([...state.tiles, t], t.id, pos.x, pos.y, state.cols, state.gravity);
    if (!defer) commit();
  }

  function removeTile(id) {
    const rest = state.tiles.filter((x) => x.id !== id);
    state.tiles = state.gravity ? compact(rest) : rest;
    commit();
  }

  // 一覧のアプリを右クリックしたとき
  window.addEventListener('ddl-app-context', (e) => {
    const { x, y, node } = e.detail;
    const have = findByPath(node.path);
    const items = [
      have
        ? { label: 'スタートからピン留めを外す', fn: () => removeTile(have.id) }
        : { label: 'スタートにピン留めする', fn: () => addTile(node.name, node.path) },
    ];
    items.push(...window.DDL.pathMenuItems(node.path));
    if (window.DDL.railMenuItems) items.push(window.DDL.railMenuItems(node));
    if (window.DDL.bookmarkMenuItems) items.push(window.DDL.bookmarkMenuItems(node));
    showMenu(x, y, items);
  });

  // ---------- ドラッグ移動・クリックで起動
  area.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    const el = e.target.closest('.tile');
    if (!el) return;
    const id = +el.dataset.id;
    const t = state.tiles.find((x) => x.id === id);
    if (!t) return;
    const rect = area.getBoundingClientRect();
    drag = {
      id, pointerId: e.pointerId, startX: e.clientX, startY: e.clientY,
      offX: e.clientX - rect.left - t.x * STEP, offY: e.clientY - rect.top - t.y * STEP,
      active: false, preview: null,
    };
    el.setPointerCapture(e.pointerId);
  });

  area.addEventListener('pointermove', (e) => {
    if (!drag || e.pointerId !== drag.pointerId) return;
    if (!drag.active) {
      if (Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY) < 5) return;
      drag.active = true;
    }
    const rect = area.getBoundingClientRect();
    const px = e.clientX - rect.left - drag.offX;
    const py = e.clientY - rect.top - drag.offY;
    drag.preview = moveTile(state.tiles, drag.id, px / STEP, py / STEP, state.cols, state.gravity);
    render(drag.preview, { id: drag.id, px, py });
  });

  function endDrag(commitIt) {
    if (!drag) return;
    const d = drag;
    drag = null;
    if (commitIt && d.active && d.preview) {
      state.tiles = d.preview;
      commit();
    } else if (commitIt && !d.active) {
      // 動かさずに離した = クリック → 起動
      const t = state.tiles.find((x) => x.id === d.id);
      if (t) invoke('launch', { path: t.path }).catch(() => {}); // 失敗はログに出る
      render();
    } else {
      render();
    }
  }
  area.addEventListener('pointerup', (e) => { if (drag && e.pointerId === drag.pointerId) endDrag(true); });
  area.addEventListener('pointercancel', (e) => { if (drag && e.pointerId === drag.pointerId) endDrag(false); });
  window.addEventListener('blur', () => endDrag(false));

  // Esc: メニューが開いていれば閉じるだけ、ドラッグ中なら取り消すだけ（ウィンドウは閉じない）
  window.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return;
    if (menu.style.display === 'flex') { closeMenu(); e.stopImmediatePropagation(); return; }
    if (drag && drag.active) { endDrag(false); e.stopImmediatePropagation(); return; }
    if (listDrag && listDrag.active) { endListDrag(false); e.stopImmediatePropagation(); }
  }, true);

  // ---------- 左の一覧（検索結果・最近使った も含む）からタイル領域へドラッグして追加
  // ブラウザ標準のドラッグ&ドロップは、ファイルのドロップを受けている間は使えないので、
  // タイルの移動と同じく、ポインターイベントで作る。
  const appsEl = document.getElementById('apps');
  let listDrag = null; // {path, name, startX, startY, active, chip, at}
  let suppressClick = false;

  function listDragTarget(e) {
    const rect = area.getBoundingClientRect();
    const inside = e.clientX >= rect.left && e.clientX <= rect.right
      && e.clientY >= rect.top && e.clientY <= rect.bottom;
    if (!inside) return null;
    // 中タイルの中心がカーソルの位置に来るようにする
    const at = { x: (e.clientX - rect.left) / STEP - 1, y: (e.clientY - rect.top) / STEP - 1 };
    const s = SIZES.M;
    const probe = { id: -1, name: '', path: '', size: 'M', x: 0, y: 0, w: s.w, h: s.h };
    const laid = moveTile([...state.tiles, probe], -1, at.x, at.y, state.cols, state.gravity);
    return { at, cell: laid.find((x) => x.id === -1) };
  }

  function showListGhost(target) {
    if (!target) {
      ghost.style.display = 'none';
      area.classList.remove('drop-over');
      return;
    }
    const c = target.cell;
    area.classList.add('drop-over'); // マスを見せる
    ghost.style.display = 'block';
    ghost.style.left = c.x * STEP + 'px';
    ghost.style.top = c.y * STEP + 'px';
    ghost.style.width = (c.w * STEP - GAP) + 'px';
    ghost.style.height = (c.h * STEP - GAP) + 'px';
  }

  function endListDrag(commitIt, e) {
    const d = listDrag;
    listDrag = null;
    if (!d) return;
    if (d.chip) d.chip.remove();
    ghost.style.display = 'none';
    area.classList.remove('drop-over');
    if (!d.active) return;
    // ドラッグ後に離した位置で click が出て、アプリが起動してしまうのを防ぐ
    suppressClick = true;
    setTimeout(() => { suppressClick = false; }, 60);
    if (!commitIt || !e) return;
    const target = listDragTarget(e);
    if (!target) return;
    if (findByPath(d.path)) { logMsg('ドラッグ: 既にピン留め済み ' + d.path); return; }
    addTile(d.name, d.path, 'M', target.at);
  }

  appsEl.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    const row = e.target.closest('.row.app');
    if (!row || !row.dataset.path) return;
    listDrag = { path: row.dataset.path, name: row.dataset.name || row.dataset.path, startX: e.clientX, startY: e.clientY, active: false, chip: null };
  });
  window.addEventListener('pointermove', (e) => {
    if (!listDrag) return;
    if (!listDrag.active) {
      if (Math.hypot(e.clientX - listDrag.startX, e.clientY - listDrag.startY) < 6) return;
      listDrag.active = true;
      const chip = document.createElement('div');
      chip.className = 'dragChip';
      chip.textContent = listDrag.name;
      document.body.appendChild(chip);
      listDrag.chip = chip;
    }
    listDrag.chip.style.left = (e.clientX + 12) + 'px';
    listDrag.chip.style.top = (e.clientY + 8) + 'px';
    showListGhost(listDragTarget(e));
  });
  window.addEventListener('pointerup', (e) => endListDrag(e.button === 0, e));
  window.addEventListener('pointercancel', () => endListDrag(false));
  window.addEventListener('blur', () => endListDrag(false));
  window.addEventListener('click', (e) => {
    if (suppressClick) { e.stopPropagation(); e.preventDefault(); }
  }, true);

  // ---------- タイルの右クリック
  area.addEventListener('contextmenu', (e) => {
    const el = e.target.closest('.tile');
    if (!el || drag) return;
    const id = +el.dataset.id;
    const t = state.tiles.find((x) => x.id === id);
    if (!t) return;
    const items = ['S', 'M', 'W', 'L'].map((k) => ({
      label: 'サイズ: ' + SIZE_LABEL[k] + (k === t.size ? ' ✓' : ''),
      fn: () => { state.tiles = resizeTile(state.tiles, id, k, state.cols, state.gravity); commit(); },
    }));
    items.push('-', { label: 'スタートからピン留めを外す', fn: () => removeTile(id) });
    const pm = window.DDL.pathMenuItems(t.path);
    if (pm.length) items.push('-', ...pm);
    showMenu(e.clientX, e.clientY, items);
  });

  // ---------- エクスプローラーからのドロップ（exe・フォルダ・ショートカットなど）
  // ブラウザ標準のドロップではパスが取れないので、Tauri のドロップイベントを使う。
  async function dropPaths(paths, position) {
    let infos = [];
    try {
      infos = await invoke('inspect_paths', { paths });
    } catch (e) {
      logMsg('ドロップの解析に失敗: ' + e);
      return;
    }
    // 位置は物理ピクセル。CSS ピクセルに直して、タイル領域のマスに換算する
    const dpr = window.devicePixelRatio || 1;
    const cx = position.x / dpr;
    const cy = position.y / dpr;
    // 🔖 にドロップされたら、ブックマークに追加する
    if (window.DDL.bookmarkBtnContains && window.DDL.bookmarkBtnContains(cx, cy)) {
      let n = 0;
      for (const info of infos) if (window.DDL.bookmarkAddPath(info)) n++;
      logMsg(`ドロップ: ${paths.length} 件中 ${n} 件をブックマークに追加`);
      return;
    }
    // 左端の列にドロップされたら、タイルではなく左端に追加する
    if (window.DDL.railContains && window.DDL.railContains(cx, cy)) {
      let n = 0;
      for (const info of infos) if (window.DDL.railAdd(info.name, info.path)) n++;
      logMsg(`ドロップ: ${paths.length} 件中 ${n} 件を左端に追加`);
      return;
    }
    const rect = area.getBoundingClientRect();
    const inside = cx >= rect.left && cx <= rect.right && cy >= rect.top && cy <= rect.bottom;
    let added = 0;
    for (const info of infos) {
      if (findByPath(info.path)) {
        logMsg('ドロップ: 既にピン留め済み ' + info.path);
        continue;
      }
      // 最初の1件だけ落とした場所へ（中タイルの中心が指す位置）。残りは空いている場所へ
      const at = added === 0 && inside
        ? { x: (cx - rect.left) / STEP - 1, y: (cy - rect.top) / STEP - 1 }
        : null;
      addTile(info.name, info.path, 'M', at, true);
      added++;
    }
    logMsg(`ドロップ: ${paths.length} 件中 ${added} 件をタイルに追加`);
    if (added) commit();
  }

  function onDragDrop(p) {
    if (window.DDL.railHover) window.DDL.railHover(p);
    if (window.DDL.bookmarkHover) window.DDL.bookmarkHover(p);
    if (p.type === 'enter' || p.type === 'over') area.classList.add('drop-over'); // グリッドを見せる
    else if (p.type === 'leave') area.classList.remove('drop-over');
    else if (p.type === 'drop') {
      area.classList.remove('drop-over');
      dropPaths(p.paths || [], p.position);
    }
  }

  // ---------- 読み込み・設定
  function applyLoaded(raw) {
    const s = sanitize(raw);
    state.cols = s.cols;
    state.gravity = s.gravity;
    state.tiles = s.gravity ? compact(s.tiles) : s.tiles;
    state.nextId = state.tiles.reduce((m, t) => Math.max(m, t.id), 0) + 1;
  }

  window.initTiles = async () => {
    try {
      const s = await invoke('load_tiles');
      if (s) applyLoaded(JSON.parse(s));
    } catch (e) {
      logMsg('タイルの読み込みに失敗: ' + e);
    }
    const colsSel = document.getElementById('tileCols');
    const gravChk = document.getElementById('tileGravity');
    colsSel.value = String(state.cols);
    gravChk.checked = state.gravity;
    colsSel.addEventListener('change', () => {
      state.cols = +colsSel.value;
      const s = settle(state.tiles, state.cols);
      state.tiles = state.gravity ? compact(s) : s;
      commit();
    });
    gravChk.addEventListener('change', () => {
      state.gravity = gravChk.checked;
      if (state.gravity) state.tiles = compact(state.tiles);
      commit();
    });
    render();
    try {
      await window.__TAURI__.webview.getCurrentWebview().onDragDropEvent((e) => onDragDrop(e.payload));
    } catch (e) {
      logMsg('ドロップの受け付けを開始できなかった: ' + e);
    }
  };
})();
