// Port of the `noise` crate's `core::perlin::perlin_2d` and `Fbm::get` (persistence 0.5,
// lacunarity 2π/3, frequency 1). `table` holds one 256-entry permutation table per octave,
// built on the CPU by `gpu::fbm::perm_tables` so the GPU hash equals the crate's.

@group(0) @binding(3) var<storage, read> table: array<u32>;

// the crate's `PermutationTable::hash(&[cx, cy]) & 3`
fn hash2(cx: i32, cy: i32, octave: u32) -> u32 {
    let base = octave * 256u;
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

fn perlin2(p: vec2<f32>, octave: u32) -> f32 {
    let fl = floor(p);
    let c = vec2<i32>(fl);
    let d = p - fl;
    let g00 = grad2(hash2(c.x, c.y, octave), d);
    let g10 = grad2(hash2(c.x + 1, c.y, octave), d - vec2<f32>(1.0, 0.0));
    let g01 = grad2(hash2(c.x, c.y + 1, octave), d - vec2<f32>(0.0, 1.0));
    let g11 = grad2(hash2(c.x + 1, c.y + 1, octave), d - vec2<f32>(1.0, 1.0));
    let t = d * d * d * (d * (d * 6.0 - 15.0) + 10.0);
    let r = mix(mix(g00, g01, t.y), mix(g10, g11, t.y), t.x) * 1.4142135;
    return clamp(r, -1.0, 1.0);
}

// the raw octave sum; the caller applies the crate's `scale_factor`
fn fbm2(p: vec2<f32>, octaves: u32) -> f32 {
    var q = p;
    var result = 0.0;
    var attenuation = 0.5;
    for (var i = 0u; i < octaves; i++) {
        result += perlin2(q, i) * attenuation;
        attenuation *= 0.5;
        q *= 2.0943951;
    }
    return result;
}
