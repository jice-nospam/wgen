// One relaxation dispatch of the depression fill: `w = h` where `h` is strictly above the
// lowest neighbour's `w`, else that minimum plus `eps` (Planchon & Darboux with the epsilon on
// flats only), converging from above towards the filled surface where every land cell has a
// strictly lower neighbour and every cell that drains freely keeps its own height. A workgroup loads a 32x32 tile plus its 1-cell halo
// into shared memory and runs `ROUNDS` rounds of four Gauss-Seidel line sweeps (left, right,
// down, up), one thread per line. Before each sweep the whole workgroup precomputes every
// cell's minimum over the two lines beside it (`side`), so a sweeping line reads only itself:
// one neighbour from shared memory, the other carried in a register. The shared arrays are
// padded to odd strides so the 32 sweeping threads never share a memory bank. The tile then
// goes to `w_dst`; a tile with a cell that moved by more than half an `eps` sets
// `flags[params.slot]`.

struct Band { width: u32, height: u32, first_row: u32, rows: u32 }
struct Params { k: f32, uplift: f32, water_level: f32, n_cells: f32, area_floor: f32, eps: f32, slot: u32, cold: u32, freeze: u32, pad0: u32, pad1: u32, pad2: u32 }
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> h_buf: array<f32>;
@group(0) @binding(3) var<storage, read> w_src: array<f32>;
@group(0) @binding(4) var<storage, read_write> w_dst: array<f32>;
@group(0) @binding(5) var<storage, read_write> flags: array<atomic<u32>>;
@group(0) @binding(6) var<storage, read> lake: array<u32>;

const T: u32 = 32u;
const HALO: u32 = 34u;
// row strides of the shared arrays, odd so that rows land in distinct banks
const TS: u32 = 35u;
const HS: u32 = 33u;
const ROUNDS: u32 = 1u;
const THREADS: u32 = 256u;
// a height no map reaches; `INF + eps == INF` in f32, so an unresolved cell stays unresolved
const INF: f32 = 1.0e30;

// the tile and its halo, `[lx + ly * TS]` for `lx`, `ly` in 0..HALO
var<workgroup> tile: array<f32, 1190>;
// the tile's heights, `[x + y * HS]` for `x`, `y` in 0..T
var<workgroup> hgt: array<f32, 1056>;
// bit `x + y * T` set: that cell is fixed (base, or a frozen lake cell), never relaxed
var<workgroup> base_bits: array<atomic<u32>, 32>;
// for the 32 lines being swept, `[along + line * HS]`: the minimum over the 6 cells of the two
// lines beside each cell, taken before the sweep (Jacobi across lines, Gauss-Seidel along them)
var<workgroup> side: array<f32, 1056>;
var<workgroup> changed: atomic<u32>;

fn in_map(x: i32, y: i32) -> bool {
    return x >= 0 && y >= 0 && x < i32(band.width) && y < i32(band.height);
}

fn is_base(x: u32, y: u32) -> bool {
    let bit = x + y * T;
    return ((atomicLoad(&base_bits[bit >> 5u]) >> (bit & 31u)) & 1u) == 1u;
}

// `side` of every row: the minimum over the row above and the row below
fn row_sides(li: u32) {
    for (var e = li; e < T * T; e += THREADS) {
        let x = e % T;
        let y = e / T;
        let c = (x + 1u) + (y + 1u) * TS;
        var m = min(tile[c - TS - 1u], min(tile[c - TS], tile[c - TS + 1u]));
        m = min(m, min(tile[c + TS - 1u], min(tile[c + TS], tile[c + TS + 1u])));
        side[x + y * HS] = m;
    }
}

// `side` of every column: the minimum over the columns left and right
fn column_sides(li: u32) {
    for (var e = li; e < T * T; e += THREADS) {
        let y = e % T;
        let x = e / T;
        let c = (x + 1u) + (y + 1u) * TS;
        var m = min(tile[c - 1u - TS], min(tile[c - 1u], tile[c - 1u + TS]));
        m = min(m, min(tile[c + 1u - TS], min(tile[c + 1u], tile[c + 1u + TS])));
        side[y + x * HS] = m;
    }
}

// one Gauss-Seidel sweep along a line of T cells: `c0`/`cstep` walk the tile, `x0`, `y0` and
// `xstep`, `ystep` its cell coordinates, `forward` the direction, `line` the line's row of
// `side`; the value just written is carried in `prev`, so the serial chain reads one neighbour
fn sweep(c0: u32, cstep: u32, x0: u32, y0: u32, xstep: u32, ystep: u32, forward: bool, line: u32) {
    var prev = INF;
    for (var i = 0u; i < T; i++) {
        let k = select(T - 1u - i, i, forward);
        let c = c0 + k * cstep;
        let x = x0 + k * xstep;
        let y = y0 + k * ystep;
        let ahead = select(tile[c - cstep], tile[c + cstep], forward);
        if (i == 0u) {
            prev = select(tile[c + cstep], tile[c - cstep], forward);
        }
        var v = tile[c];
        if (!is_base(x, y)) {
            let m = min(min(prev, ahead), side[k + line * HS]);
            let h = hgt[x + y * HS];
            // a cell with a strictly lower neighbour drains freely and keeps its height; a pit
            // or a flat rises to an eps above its lowest neighbour
            v = select(m + params.eps, h, h > m);
            tile[c] = v;
        }
        prev = v;
    }
}

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let ox = i32(wg.x * T);
    let oy = i32(wg.y * T);
    for (var e = li; e < HALO * HALO; e += THREADS) {
        let lx = e % HALO;
        let ly = e / HALO;
        let gx = ox + i32(lx) - 1;
        let gy = oy + i32(ly) - 1;
        var v = INF;
        if (in_map(gx, gy)) {
            v = w_src[u32(gx) + u32(gy) * band.width];
        }
        tile[lx + ly * TS] = v;
    }
    if (li < 32u) {
        atomicStore(&base_bits[li], 0u);
    }
    if (li == 0u) {
        atomicStore(&changed, 0u);
    }
    workgroupBarrier();
    for (var t = li; t < T * T; t += THREADS) {
        let x = t % T;
        let y = t / T;
        let gx = ox + i32(x);
        let gy = oy + i32(y);
        var h = INF;
        var base = true;
        if (in_map(gx, gy)) {
            h = h_buf[u32(gx) + u32(gy) * band.width];
            let g = u32(gx) + u32(gy) * band.width;
            let border = gx == 0 || gy == 0 || gx == i32(band.width) - 1 || gy == i32(band.height) - 1;
            // a warm iteration leaves last iteration's lakes as they are (`fluvial_fill_init`)
            base = border || h <= params.water_level || (params.freeze == 1u && lake[g] == 1u);
        }
        hgt[x + y * HS] = h;
        if (base) {
            atomicOr(&base_bits[t >> 5u], 1u << (t & 31u));
        }
    }
    workgroupBarrier();
    for (var r = 0u; r < ROUNDS; r++) {
        // rows, left to right then right to left
        for (var dir = 0u; dir < 2u; dir++) {
            row_sides(li);
            workgroupBarrier();
            if (li < T) {
                sweep(1u + (li + 1u) * TS, 1u, 0u, li, 1u, 0u, dir == 0u, li);
            }
            workgroupBarrier();
        }
        // columns, top to bottom then bottom to top
        for (var dir = 0u; dir < 2u; dir++) {
            column_sides(li);
            workgroupBarrier();
            if (li < T) {
                sweep((li + 1u) + TS, TS, li, 0u, 0u, 1u, dir == 0u, li);
            }
            workgroupBarrier();
        }
    }
    for (var t = li; t < T * T; t += THREADS) {
        let x = t % T;
        let y = t / T;
        let gx = ox + i32(x);
        let gy = oy + i32(y);
        if (in_map(gx, gy)) {
            let g = u32(gx) + u32(gy) * band.width;
            let v = tile[(x + 1u) + (y + 1u) * TS];
            // a move under half an `eps` cannot change the routing: not worth another dispatch
            if (abs(v - w_src[g]) > 0.5 * params.eps) {
                atomicStore(&changed, 1u);
            }
            w_dst[g] = v;
        }
    }
    workgroupBarrier();
    if (li == 0u && atomicLoad(&changed) == 1u) {
        atomicMax(&flags[params.slot], 1u);
    }
}
