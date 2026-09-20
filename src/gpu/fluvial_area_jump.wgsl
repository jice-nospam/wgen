// One doubling level of the drainage accumulation, first half: `p_dst[i]` becomes the cell
// `2^(k+1)` steps downstream (`SINK` once the path has ended) and `s_dst` starts as a copy of
// `s_src`, the number of cells within `2^k - 1` steps upstream. `fluvial_area_scatter.wgsl`
// then adds the cells between `2^k` and `2^(k+1) - 1` steps. A level that still has a live
// pointer sets `flags[params.slot]`: another level is needed.

struct Band { width: u32, height: u32, first_row: u32, rows: u32 }
struct Params { k: f32, uplift: f32, water_level: f32, n_cells: f32, area_floor: f32, eps: f32, slot: u32, cold: u32, freeze: u32, pad0: u32, pad1: u32, pad2: u32 }
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> p_src: array<u32>;
@group(0) @binding(3) var<storage, read_write> p_dst: array<u32>;
@group(0) @binding(4) var<storage, read> s_src: array<u32>;
@group(0) @binding(5) var<storage, read_write> s_dst: array<u32>;
@group(0) @binding(6) var<storage, read_write> flags: array<atomic<u32>>;

const SINK: u32 = 0xFFFFFFFFu;

var<workgroup> live: atomic<u32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    if (li == 0u) {
        atomicStore(&live, 0u);
    }
    workgroupBarrier();
    if (gid.x < band.width && gid.y < band.rows) {
        let i = gid.x + (band.first_row + gid.y) * band.width;
        let p = p_src[i];
        var q = SINK;
        if (p != SINK) {
            q = p_src[p];
        }
        p_dst[i] = q;
        s_dst[i] = s_src[i];
        if (q != SINK) {
            atomicStore(&live, 1u);
        }
    }
    workgroupBarrier();
    if (li == 0u && atomicLoad(&live) == 1u) {
        atomicMax(&flags[params.slot], 1u);
    }
}
