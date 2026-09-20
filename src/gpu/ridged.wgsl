// Ridged kernel: the `gen_ridged` cell expression, one invocation per cell of the band.
// Appended after noise.wgsl, which declares binding 3 (the tables: the ridge stream's
// `octaves` tables first, then the fold's two streams of `fold_octaves` each).

struct Band { width: u32, height: u32, first_row: u32, rows: u32 }

struct RidgedParams {
    coef: f32,
    offset_x: f32,
    offset_y: f32,
    scale: f32,
    scale_factor: f32,
    fold_px: f32,
    fold_coef: f32,
    fold_scale: f32,
    width_f: f32,
    height_f: f32,
    octaves: u32,
    fold_octaves: u32,
    fold_first_a: u32,
    fold_first_b: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: RidgedParams;
@group(0) @binding(2) var<storage, read_write> map: array<f32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= band.width || gid.y >= band.rows) { return; }
    let y = band.first_row + gid.y;
    let i = gid.x + gid.y * band.width;
    let u = f32(gid.x) * 512.0 / params.width_f + params.offset_x;
    let v = f32(y) * 512.0 / params.height_f + params.offset_y;
    let p = vec2<f32>(u, v);
    var q = p;
    if (params.fold_px > 0.0) {
        q = warp2(p, params.fold_px, params.fold_coef, params.fold_octaves,
                  params.fold_first_a, params.fold_first_b, params.fold_scale);
    }
    let r = ridged2(q * params.coef, params.octaves, 0u) * params.scale_factor - 1.0;
    map[i] = map[i] + 0.5 * (r + 1.0) * params.scale;
}
