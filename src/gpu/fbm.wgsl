// Fbm kernel: the `gen_fbm` cell expression, one invocation per cell of the band.
// Appended after noise.wgsl, which declares binding 3.

struct Band { width: u32, height: u32, first_row: u32, rows: u32 }

struct FbmParams {
    xcoef: f32,
    ycoef: f32,
    addx: f32,
    addy: f32,
    delta: f32,
    scale: f32,
    scale_factor: f32,
    width_f: f32,
    height_f: f32,
    octaves: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: FbmParams;
@group(0) @binding(2) var<storage, read_write> map: array<f32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= band.width || gid.y >= band.rows) { return; }
    let y = band.first_row + gid.y;
    let i = gid.x + gid.y * band.width;
    let f0 = (f32(gid.x) * 512.0 / params.width_f + params.addx) * params.xcoef;
    let f1 = (f32(y) * 512.0 / params.height_f + params.addy) * params.ycoef;
    map[i] = map[i] + params.delta + fbm2(vec2<f32>(f0, f1), params.octaves) * params.scale_factor * params.scale;
}
