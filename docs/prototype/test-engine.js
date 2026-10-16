// 使い方: node test-engine.js tile-prototype.html
const fs = require('fs');
const html = fs.readFileSync(process.argv[2] || 'tile-prototype.html', 'utf8');
const m = html.match(/\/\/ ENGINE-START([\s\S]*?)\/\/ ENGINE-END/);
if (!m) throw new Error('engine markers not found');
const E = new Function(m[1] + '; return {SIZES,SNAP,overlaps,clone,settle,compact,findFree,moveTile,resizeTile,sanitize};')();

let passed = 0;
function assert(cond, msg) { if (!cond) throw new Error('FAIL: ' + msg); passed++; }
function mk(id, size, x, y) { const s = E.SIZES[size]; return { id, name: 'n' + id, path: '', size, x, y, w: s.w, h: s.h }; }
function get(tiles, id) { return tiles.find(t => t.id === id); }

function invariants(tiles, cols, label) {
  const ids = new Set();
  for (const t of tiles) {
    assert(!ids.has(t.id), label + ': duplicate id ' + t.id); ids.add(t.id);
    const s = E.SIZES[t.size], st = E.SNAP[t.size];
    assert(t.w === s.w && t.h === s.h, label + ': size mismatch');
    assert(t.x >= 0 && t.x + t.w <= cols && t.y >= 0, label + ': out of bounds ' + JSON.stringify(t));
    assert(t.x % st === 0 && t.y % st === 0, label + ': snap broken ' + JSON.stringify(t));
  }
  for (let i = 0; i < tiles.length; i++) for (let j = i + 1; j < tiles.length; j++)
    assert(!E.overlaps(tiles[i], tiles[j]), label + ': overlap ' + tiles[i].id + '/' + tiles[j].id);
}

// 1. 中3つの行の真ん中に小を挿入 → 真ん中だけ押し下げ、他は動かない
{
  let t = [mk(1,'M',0,0), mk(2,'M',2,0), mk(3,'M',4,0), mk(4,'S',9,9)];
  t = E.moveTile(t, 4, 2, 0, 6, false);
  assert(get(t,4).x === 2 && get(t,4).y === 0, 'S placed at 2,0');
  assert(get(t,2).y === 2 && get(t,2).x === 2, 'M pushed down to y=2');
  assert(get(t,1).y === 0 && get(t,3).y === 0, 'others stay');
  invariants(t, 6, 'case1');
}
// 2. 連鎖の押し下げ
{
  let t = [mk(1,'M',0,0), mk(2,'M',0,2), mk(3,'M',4,4)];
  t = E.moveTile(t, 3, 0, 0, 6, false);
  assert(get(t,3).y === 0 && get(t,1).y === 2 && get(t,2).y === 4, 'chain push');
  invariants(t, 6, 'case2');
}
// 3. サイズ変更で隣を押し下げ
{
  let t = [mk(1,'M',0,0), mk(2,'M',2,0)];
  t = E.resizeTile(t, 1, 'L', 6, false);
  assert(get(t,1).w === 4 && get(t,1).h === 4, 'resized to L');
  assert(get(t,2).y === 4, 'neighbor pushed below L');
  invariants(t, 6, 'case3');
}
// 4. 隙間の保持と上詰め
{
  let t = [mk(1,'S',0,0), mk(2,'S',5,0), mk(3,'M',0,6)];
  const s = E.settle(t, 6);
  assert(get(s,1).x === 0 && get(s,2).x === 5 && get(s,3).y === 6, 'gaps kept without gravity');
  const c = E.compact(t);
  assert(get(c,3).y === 2 || get(c,3).y === 1 || get(c,3).y === 0, 'compact moves up');
  assert(get(c,3).y === 2, 'M below S row snaps to y=2');
  invariants(c, 6, 'case4');
}
// 5. 範囲外へのドラッグ
{
  let t = [mk(1,'M',0,0)];
  t = E.moveTile(t, 1, 100, -5, 6, false);
  assert(get(t,1).x === 4 && get(t,1).y === 0, 'clamped into area');
  invariants(t, 6, 'case5');
}
// 6. 壊れたJSONの検証
{
  const s = E.sanitize({ cols: 8, tiles: [
    { id:1, name:'a', size:'M', x:0, y:0 }, { id:1, name:'dup', size:'M', x:0, y:0 },
    { id:2, name:'bad', size:'XX', x:0, y:0 }, { id:3, name:'nan', size:'S', x:'a', y:0 },
    { id:4, name:'over', size:'L', x:50, y:-3 }, null, 5 ] });
  assert(s.cols === 8 && s.tiles.length === 2, 'sanitize drops bad entries');
  invariants(s.tiles, 8, 'case6');
  const e = E.sanitize(null);
  assert(e.cols === 6 && e.tiles.length === 0, 'sanitize(null)');
}
// 7. ランダム操作（不変条件が常に成り立つか）
{
  let seed = 12345;
  const rnd = () => { seed |= 0; seed = seed + 0x6D2B79F5 | 0; let t = Math.imul(seed ^ seed >>> 15, 1 | seed);
    t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; };
  const keys = ['S','M','W','L'];
  for (let run = 0; run < 200; run++) {
    const cols = rnd() < 0.5 ? 6 : 8, gravity = rnd() < 0.5;
    let tiles = [], nextId = 1;
    for (let step = 0; step < 60; step++) {
      const r = rnd();
      if (r < 0.35 || tiles.length === 0) {
        const size = keys[Math.floor(rnd()*4)], s = E.SIZES[size];
        const t = { id: nextId++, name:'x', path:'', size, x:0, y:0, w:s.w, h:s.h };
        const pos = rnd() < 0.5 ? E.findFree(tiles, size, cols) : { x: rnd()*cols, y: rnd()*12 };
        tiles = E.moveTile([...tiles, t], t.id, pos.x, pos.y, cols, gravity);
      } else if (r < 0.7) {
        const t = tiles[Math.floor(rnd()*tiles.length)];
        tiles = E.moveTile(tiles, t.id, rnd()*(cols+2)-1, rnd()*14-1, cols, gravity);
      } else if (r < 0.9) {
        const t = tiles[Math.floor(rnd()*tiles.length)];
        tiles = E.resizeTile(tiles, t.id, keys[Math.floor(rnd()*4)], cols, gravity);
      } else {
        const t = tiles[Math.floor(rnd()*tiles.length)];
        const rest = tiles.filter(x => x.id !== t.id);
        tiles = gravity ? E.compact(rest) : rest;
      }
      invariants(tiles, cols, 'random run ' + run + ' step ' + step);
    }
    const back = E.sanitize(JSON.parse(JSON.stringify({ cols, gravity, tiles })));
    assert(back.tiles.length === tiles.length, 'roundtrip count');
    invariants(back.tiles, cols, 'roundtrip run ' + run);
  }
}

console.log('OK: ' + passed + ' assertions passed');
