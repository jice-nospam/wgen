// Plateau kernel: the `gen_plateau` cell expression, one invocation per cell of the band.
// Appended after noise.wgsl (binding 3, the jitter noise's tables) and terrace.wgsl.

struct Band { width: u32, height: u32, first_row: u32, rows: u32 }

struct PlateauParams {
    min: f32,
    range: f32,
    levels: f32,
    flat: f32,
    rounding: f32,
    jitter: f32,
    jitter_coef: f32,
    jitter_scale: f32,
    width_f: f32,
    height_f: f32,
    octaves: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: PlateauParams;
@group(0) @binding(2) var<storage, read_write> map: array<f32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= band.width || gid.y >= band.rows) { return; }
    let y = band.first_row + gid.y;
    let i = gid.x + gid.y * band.width;
    let u = f32(gid.x) * 512.0 / params.width_f;
    let v = f32(y) * 512.0 / params.height_f;
    let noise = fbm2_from(vec2<f32>(u, v) * params.jitter_coef, params.octaves, 0u);
    let j = params.jitter * noise * params.jitter_scale;
    let frac = (map[i] - params.min) / params.range;
    let s = frac * params.levels + j;
    let k = floor(s);
    let f = s - k;
    map[i] = params.min + (k + terrace(f, params.flat, params.rounding)) / params.levels * params.range;
}
