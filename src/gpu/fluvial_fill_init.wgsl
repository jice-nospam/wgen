// The starting surface of the depression fill. Base cells (the map border and the sea) keep
// their height; on a cold start every other cell starts at `INF`. On a warm start a cell that
// was under a lake at the previous iteration (the `lake` mask the routing kernel wrote) keeps
// the previous surface raised by the uplift, which the relaxation then leaves frozen; a cell strictly above the
// lowest of its neighbours on that mixed surface drains freely and is exact at its height;
// a cell with no lower neighbour is a new pit and starts at `INF`, so its ramp converges from
// above in a few sweeps instead of creeping up.

struct Band { width: u32, height: u32, first_row: u32, rows: u32 }
struct Params { k: f32, uplift: f32, water_level: f32, n_cells: f32, area_floor: f32, eps: f32, slot: u32, cold: u32, freeze: u32, pad0: u32, pad1: u32, pad2: u32 }
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> h_buf: array<f32>;
@group(0) @binding(3) var<storage, read> lake: array<u32>;
@group(0) @binding(4) var<storage, read> w_prev: array<f32>;
@group(0) @binding(5) var<storage, read_write> w_out: array<f32>;

const INF: f32 = 1.0e30;

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
    let h = h_buf[i];
    let border = p.x == 0 || p.y == 0 || p.x == i32(band.width) - 1 || p.y == i32(band.height) - 1;
    if (border || h <= params.water_level) {
        w_out[i] = h;
        return;
    }
    if (params.cold == 1u) {
        w_out[i] = INF;
        return;
    }
    if (lake[i] == 1u) {
        // the land rises by the uplift each iteration, the lake surface with its spill
        w_out[i] = w_prev[i] + params.uplift;
        return;
    }
    var m = INF;
    for (var k = 0u; k < 9u; k++) {
        if (k == 4u) {
            continue;
        }
        let n = p + vec2<i32>(i32(k % 3u) - 1, i32(k / 3u) - 1);
        if (!in_map(n)) {
            continue;
        }
        let ni = u32(n.x) + u32(n.y) * band.width;
        let hn = h_buf[ni];
        let nborder = n.x == 0 || n.y == 0 || n.x == i32(band.width) - 1 || n.y == i32(band.height) - 1;
        let wn = select(select(hn, w_prev[ni] + params.uplift, lake[ni] == 1u), hn, nborder || hn <= params.water_level);
        m = min(m, wn);
    }
    w_out[i] = select(INF, h, h > m);
}
