// One sweep of the implicit stream-power incision: `dst = (h0 + uplift + c * h_recv) /
// (1 + c)` with `h_recv` the receiver's height from the previous sweep, `c` the incision
// coefficient of the cell's drainage area (`fluvial_erosion::incision_coef`). Repeated sweeps
// converge on the CPU's solve, which walks the cells in drainage order instead. Sea cells are left as they are, border
// land drains to the water level one cell away, and a cell without a receiver (a pit the fill
// did not reach) is left as it is.

struct Band { width: u32, height: u32, first_row: u32, rows: u32 }
struct Params { k: f32, uplift: f32, water_level: f32, n_cells: f32, area_floor: f32, eps: f32, slot: u32, cold: u32, freeze: u32, pad0: u32, pad1: u32, pad2: u32 }
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> h0: array<f32>;
@group(0) @binding(3) var<storage, read> h_src: array<f32>;
@group(0) @binding(4) var<storage, read_write> h_dst: array<f32>;
@group(0) @binding(5) var<storage, read> recv: array<u32>;
@group(0) @binding(6) var<storage, read> area: array<u32>;

const SINK: u32 = 0xFFFFFFFFu;
const SQRT2: f32 = 1.4142135;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= band.width || gid.y >= band.rows) {
        return;
    }
    let x = i32(gid.x);
    let y = i32(band.first_row + gid.y);
    let i = u32(x) + u32(y) * band.width;
    let h = h0[i];
    if (h <= params.water_level) {
        h_dst[i] = h;
        return;
    }
    let r = recv[i];
    var h_recv = params.water_level;
    var dist = 1.0;
    if (r == SINK) {
        let border = x == 0 || y == 0 || x == i32(band.width) - 1 || y == i32(band.height) - 1;
        if (!border) {
            h_dst[i] = h;
            return;
        }
    } else {
        h_recv = max(h_src[r], params.water_level);
        let dx = x - i32(r % band.width);
        let dy = y - i32(r / band.width);
        dist = select(1.0, SQRT2, dx != 0 && dy != 0);
    }
    let a = max(f32(area[i]), params.area_floor) / params.n_cells;
    let c = params.k * sqrt(a) / dist;
    h_dst[i] = (h + params.uplift + c * h_recv) / (1.0 + c);
}
