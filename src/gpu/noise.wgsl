// Port of the `noise` crate's `core::perlin::perlin_2d`, `Fbm::get` (persistence 0.5,
// lacunarity 2π/3, frequency 1) and `RidgedMulti::get` (persistence 1, attenuation 2, the same
// lacunarity). `table` holds 256-entry permutation tables, one per octave of each noise
// stream, built on the CPU by `gpu::fbm::perm_tables` / `stream_tables` so the GPU hash equals
// the crate's; a stream's octave `i` reads table `first + i`.

@group(0) @binding(3) var<storage, read> table: array<u32>;

// the crate's `PermutationTable::hash(&[cx, cy]) & 3` on table `index`
fn hash2(cx: i32, cy: i32, index: u32) -> u32 {
    let base = index * 256u;
    let a = table[base + u32(cx & 255)];
    return table[base + (a ^ u32(cy & 255))] & 3u;
}

// gradient (±1, ±1) selected by `h`, dotted with the corner offset `p`
fn grad2(h: u32, p: vec2<f32>) -> f32 {
    switch h {
        case 0u: { return p.x + p.y; }
        case 1u: { return -p.x + p.y; }
        case 2u: { return p.x - p.y; }
        default: { return -p.x - p.y; }
    }
}

fn perlin2(p: vec2<f32>, index: u32) -> f32 {
    let fl = floor(p);
    let c = vec2<i32>(fl);
    let d = p - fl;
    let g00 = grad2(hash2(c.x, c.y, index), d);
    let g10 = grad2(hash2(c.x + 1, c.y, index), d - vec2<f32>(1.0, 0.0));
    let g01 = grad2(hash2(c.x, c.y + 1, index), d - vec2<f32>(0.0, 1.0));
    let g11 = grad2(hash2(c.x + 1, c.y + 1, index), d - vec2<f32>(1.0, 1.0));
    let t = d * d * d * (d * (d * 6.0 - 15.0) + 10.0);
    let r = mix(mix(g00, g01, t.y), mix(g10, g11, t.y), t.x) * 1.4142135;
    return clamp(r, -1.0, 1.0);
}

// the raw Fbm octave sum over tables `first .. first + octaves`; the caller applies the
// crate's `scale_factor`
fn fbm2_from(p: vec2<f32>, octaves: u32, first: u32) -> f32 {
    var q = p;
    var result = 0.0;
    var attenuation = 0.5;
    for (var i = 0u; i < octaves; i++) {
        result += perlin2(q, first + i) * attenuation;
        attenuation *= 0.5;
        q *= 2.0943951;
    }
    return result;
}

// the Fbm stream whose tables start at 0
fn fbm2(p: vec2<f32>, octaves: u32) -> f32 {
    return fbm2_from(p, octaves, 0u);
}

// `RidgedMulti::get` before its scaling: the caller multiplies by the crate's `scale_factor`
// and subtracts 1. Persistence 1 drops the amplitude term
fn ridged2(p: vec2<f32>, octaves: u32, first: u32) -> f32 {
    var q = p;
    var result = 0.0;
    var weight = 1.0;
    for (var i = 0u; i < octaves; i++) {
        var s = 1.0 - abs(perlin2(q, first + i));
        s *= s;
        s *= weight;
        weight = clamp(s / 2.0, 0.0, 1.0);
        result += s;
        q *= 2.0943951;
    }
    return result;
}

// the twin of `noise_field::Warp::apply`: `p` displaced by `amount` times two Fbm streams
// (tables from `first_a` and `first_b`, `fbm_scale` their crate `scale_factor`) sampled at
// `p * coef`
fn warp2(p: vec2<f32>, amount: f32, coef: f32, octaves: u32, first_a: u32, first_b: u32, fbm_scale: f32) -> vec2<f32> {
    let q = p * coef;
    let d = vec2<f32>(fbm2_from(q, octaves, first_a), fbm2_from(q, octaves, first_b));
    return p + amount * d * fbm_scale;
}
