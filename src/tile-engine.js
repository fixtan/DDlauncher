// タイル配置エンジン（docs/prototype/tile-prototype.html から切り出し。ロジックは同一）。
// テスト: node docs/prototype/test-engine.js src/tile-engine.js
// ENGINE-START
const SIZES = { S:{w:1,h:1}, M:{w:2,h:2}, W:{w:4,h:2}, L:{w:4,h:4} };
const SNAP  = { S:1, M:2, W:2, L:2 };          // 位置のスナップ単位（Win10実機との差は要確認）
const SIZE_LABEL = { S:'小', M:'中', W:'横長', L:'大' };

function overlaps(a,b){ return a.x < b.x+b.w && b.x < a.x+a.w && a.y < b.y+b.h && b.y < a.y+a.h; }
function snapTo(v,step){ return Math.round(v/step)*step; }
function snapUp(v,step){ return Math.ceil(v/step)*step; }
function clone(tiles){ return tiles.map(t => ({...t})); }
function applySize(t){ const s = SIZES[t.size]; t.w = s.w; t.h = s.h; }
function byPos(a,b){ return a.y-b.y || a.x-b.x; }

// 位置をスナップして、領域内に収める
function fit(t,cols){
  const st = SNAP[t.size];
  const maxX = Math.floor((cols - t.w) / st) * st;
  t.x = Math.max(0, Math.min(snapTo(t.x, st), maxX));
  t.y = Math.max(0, snapTo(t.y, st));
}

// 動かしたタイル(movedId)は固定、重なった他のタイルを下へ押し下げる
function resolve(tiles, movedId){
  const queue = [tiles.find(t => t.id === movedId)];
  let guard = 0;
  while(queue.length){
    const m = queue.shift();
    for(const c of [...tiles].sort(byPos)){
      if(c === m || c.id === movedId) continue;
      if(overlaps(m,c)){
        c.y = snapUp(m.y + m.h, SNAP[c.size]);
        queue.push(c);
      }
    }
    if(++guard > 5000) throw new Error('resolve loop');
  }
}

// 任意の状態を「重ならない・はみ出さない」状態に正規化
function settle(tiles, cols){
  const out = clone(tiles);
  out.forEach(t => { applySize(t); fit(t, cols); });
  out.sort(byPos);
  const placed = [];
  for(const t of out){
    while(placed.some(p => overlaps(t,p))) t.y += SNAP[t.size];
    placed.push(t);
  }
  return out;
}

// 上詰め（各タイルをできるだけ上へ）
function compact(tiles){
  const out = clone(tiles).sort(byPos);
  const placed = [];
  for(const t of out){
    const st = SNAP[t.size];
    let y = 0;
    for(;;){
      t.y = y;
      if(!placed.some(p => overlaps(t,p))) break;
      y += st;
    }
    placed.push(t);
  }
  return out;
}

// 空いている最初の場所
function findFree(tiles, size, cols){
  const s = SIZES[size], st = SNAP[size];
  const probe = { w:s.w, h:s.h, x:0, y:0 };
  for(let y = 0; ; y += st){
    for(let x = 0; x <= cols - s.w; x += st){
      probe.x = x; probe.y = y;
      if(!tiles.some(t => overlaps(probe,t))) return { x, y };
    }
  }
}

function moveTile(tiles, id, x, y, cols, gravity){
  const out = clone(tiles);
  const t = out.find(t => t.id === id);
  t.x = x; t.y = y; fit(t, cols);
  resolve(out, id);
  return gravity ? compact(out) : out;
}

function resizeTile(tiles, id, size, cols, gravity){
  const out = clone(tiles);
  const t = out.find(t => t.id === id);
  t.size = size; applySize(t); fit(t, cols);
  resolve(out, id);
  return gravity ? compact(out) : out;
}

// 読み込んだJSONの検証＋正規化
function sanitize(raw){
  const cols = raw && raw.cols === 8 ? 8 : 6;
  const gravity = !!(raw && raw.gravity);
  const seen = new Set();
  const tiles = [];
  for(const r of (raw && Array.isArray(raw.tiles) ? raw.tiles : [])){
    if(!r || typeof r !== 'object') continue;
    if(!Number.isInteger(r.id) || seen.has(r.id) || !SIZES[r.size]) continue;
    if(!Number.isFinite(r.x) || !Number.isFinite(r.y)) continue;
    seen.add(r.id);
    tiles.push({ id:r.id, name:String(r.name ?? '').slice(0,100), path:String(r.path ?? '').slice(0,500),
                 size:r.size, x:r.x, y:r.y, w:0, h:0 });
  }
  return { cols, gravity, tiles: settle(tiles, cols) };
}
// ENGINE-END
