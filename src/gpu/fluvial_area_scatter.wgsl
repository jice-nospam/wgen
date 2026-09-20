// Second half of a doubling level of the drainage accumulation: every cell whose pointer is
// still live adds its own count to the cell exactly `2^k` steps downstream. Integer atomics,
// so the sum does not depend on the order the workgroups run in.

struct Band { width: u32, height: u32, first_row: u32, rows: u32 }
struct Params { k: f32, uplift: f32, water_level: f32, n_cells: f32, area_floor: f32, eps: f32, slot: u32, cold: u32, freeze: u32, pad0: u32, pad1: u32, pad2: u32 }
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> p_src: array<u32>;
@group(0) @binding(3) var<storage, read> s_src: array<u32>;
@group(0) @binding(4) var<storage, read_write> s_dst: array<atomic<u32>>;

const SINK: u32 = 0xFFFFFFFFu;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= band.width || gid.y >= band.rows) {
        return;
    }
    let i = gid.x + (band.first_row + gid.y) * band.width;
    let p = p_src[i];
    if (p != SINK) {
        atomicAdd(&s_dst[p], s_src[i]);
    }
}
