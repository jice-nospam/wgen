// One thermal slide pass as a gather: `dst[c] = src[c] - outflow(c) + sum of the inflows from
// the 8 neighbours`, each neighbour's share recomputed from its own neighbourhood. The terms are
// accumulated in the CPU scatter's order (`thermal_erosion::slide_pass`): the neighbours in
// row-major order, the cell's own outflow in the middle.

struct Band { width: u32, height: u32, first_row: u32, rows: u32 }
struct ThermalParams { threshold: f32, diag_threshold: f32, strength: f32, water_level: f32 }
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: ThermalParams;
@group(0) @binding(2) var<storage, read> src: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst: array<f32>;

// the 3x3 neighbourhood in row-major order; k = 4 is the cell itself
fn dir(k: u32) -> vec2<i32> {
    return vec2<i32>(i32(k % 3u) - 1, i32(k / 3u) - 1);
}

fn in_map(p: vec2<i32>) -> bool {
    return p.x >= 0 && p.y >= 0 && p.x < i32(band.width) && p.y < i32(band.height);
}

fn at(p: vec2<i32>) -> f32 {
    return src[u32(p.x) + u32(p.y) * band.width];
}

fn threshold(d: vec2<i32>) -> f32 {
    if (d.x != 0 && d.y != 0) {
        return params.diag_threshold;
    }
    return params.threshold;
}

// (moved, d_total) of the cell at p: what it sheds this pass and the excess it is shared over;
// (0, 0) below the water level or without a neighbour past the threshold
fn outflow(p: vec2<i32>) -> vec2<f32> {
    let h = at(p);
    if (h < params.water_level) {
        return vec2<f32>(0.0, 0.0);
    }
    var d_total = 0.0;
    var d_max = 0.0;
    for (var k = 0u; k < 9u; k++) {
        if (k == 4u) {
            continue;
        }
        let d = dir(k);
        let n = p + d;
        if (!in_map(n)) {
            continue;
        }
        let e = (h - at(n)) - threshold(d);
        if (e > 0.0) {
            d_total += e;
            d_max = max(d_max, e);
        }
    }
    if (d_total <= 0.0) {
        return vec2<f32>(0.0, 0.0);
    }
    return vec2<f32>(params.strength * d_max, d_total);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= band.width || gid.y >= band.rows) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(band.first_row + gid.y));
    let h = at(p);
    var out = h;
    for (var k = 0u; k < 9u; k++) {
        if (k == 4u) {
            out -= outflow(p).x;
            continue;
        }
        let d = dir(k);
        let n = p + d;
        if (!in_map(n)) {
            continue;
        }
        let flow = outflow(n);
        if (flow.y <= 0.0) {
            continue;
        }
        let e = (at(n) - h) - threshold(d);
        if (e > 0.0) {
            out += flow.x * e / flow.y;
        }
    }
    dst[u32(p.x) + u32(p.y) * band.width] = out;
}
