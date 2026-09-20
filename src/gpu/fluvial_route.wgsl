// D8 receivers on the filled surface: every land cell drains into the neighbour with the
// steepest descent of `w` (strictly lower; ties go to the first in row-major order, like the
// CPU router). Base cells, and cells the fill did not reach, get `SINK`. Also seeds the
// drainage accumulation (`p` = receiver, `s` = 1 cell) and marks the cells the next warm fill
// leaves frozen: those under a lake (`w > h`) and those on a nearly flat river (less than an
// `eps` above their receiver), so that a slide's deposit on a river does not turn the stretch
// behind it into a sink until the next full fill: frozen, the routing still crosses it.

struct Band { width: u32, height: u32, first_row: u32, rows: u32 }
struct Params { k: f32, uplift: f32, water_level: f32, n_cells: f32, area_floor: f32, eps: f32, slot: u32, cold: u32, freeze: u32, pad0: u32, pad1: u32, pad2: u32 }
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> h_buf: array<f32>;
@group(0) @binding(3) var<storage, read> w: array<f32>;
@group(0) @binding(4) var<storage, read_write> recv: array<u32>;
@group(0) @binding(5) var<storage, read_write> p_out: array<u32>;
@group(0) @binding(6) var<storage, read_write> s_out: array<u32>;
@group(0) @binding(7) var<storage, read_write> lake: array<u32>;

const SINK: u32 = 0xFFFFFFFFu;
const INF: f32 = 1.0e30;
const SQRT2: f32 = 1.4142135;

fn in_map(p: vec2<i32>) -> bool {
    return p.x >= 0 && p.y >= 0 && p.x < i32(band.width) && p.y < i32(band.height);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= band.width || gid.y >= band.rows) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(band.first_row + gid.y));
    let i = u32(p.x) + u32(p.y) * band.width;
    let hi = h_buf[i];
    let wi = w[i];
    let border = p.x == 0 || p.y == 0 || p.x == i32(band.width) - 1 || p.y == i32(band.height) - 1;
    var best = SINK;
    var best_slope = 0.0;
    var best_w = wi;
    if (!(border || hi <= params.water_level) && wi < INF) {
        for (var k = 0u; k < 9u; k++) {
            if (k == 4u) {
                continue;
            }
            let d = vec2<i32>(i32(k % 3u) - 1, i32(k / 3u) - 1);
            let n = p + d;
            if (!in_map(n)) {
                continue;
            }
            let ni = u32(n.x) + u32(n.y) * band.width;
            let wn = w[ni];
            if (wn >= wi) {
                continue;
            }
            let dist = select(1.0, SQRT2, d.x != 0 && d.y != 0);
            let slope = (wi - wn) / dist;
            if (slope > best_slope) {
                best_slope = slope;
                best = ni;
                best_w = wn;
            }
        }
    }
    recv[i] = best;
    p_out[i] = best;
    s_out[i] = 1u;
    let frozen = wi < INF && (wi > hi || (best != SINK && wi - best_w < params.eps));
    lake[i] = select(0u, 1u, frozen);
}
