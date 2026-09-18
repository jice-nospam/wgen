// The terrain's fragment shader, for both the deferred prepass (gbuffer) and the forward
// pass: the standard PBR inputs with base colour and roughness splatted from sand, grass,
// rock and seabed by slope, height (`uv_b.x`), curvature (`uv_b.y`) and a value noise.
// Layout of Bevy's `examples/shader/extended_material.rs`.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

// mirrors `terrain_material::TerrainSettings`
struct TerrainSettings {
    water_level: f32,
    snow_line: f32,
    detail: f32,
    _pad: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> terrain: TerrainSettings;

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

// value noise: the 4 integer corners of `p` blended with smoothstep weights
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let w = smoothstep(vec2<f32>(0.0), vec2<f32>(1.0), f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

fn fbm2(p: vec2<f32>) -> f32 {
    return 0.65 * vnoise(p) + 0.35 * vnoise(p * 2.3 + 17.0);
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    // every uniform field is read, so the struct cannot drift from the Rust side unnoticed
    _ = terrain.snow_line + terrain.detail;

    let nrm = normalize(in.world_normal);
    // 0 on flat ground, 1 on a vertical face
    let slope = 1.0 - nrm.y;
    // normalised height and cell curvature (-1 ridge .. 1 valley), from the mesh
    let h01 = in.uv_b.x;
    let curv = in.uv_b.y * 2.0 - 1.0;
    // terrain-local coordinates in scene units: the pattern turns with the terrain and is
    // the same at every preview size; one noise feature is about 8 units
    let p = in.uv * 500.0;
    let n = fbm2(p / 8.0);
    let w = terrain.water_level;

    let grass = mix(vec3<f32>(0.13, 0.30, 0.08), vec3<f32>(0.24, 0.36, 0.10), n);
    let rock = mix(vec3<f32>(0.32, 0.29, 0.26), vec3<f32>(0.20, 0.18, 0.17), n);
    let sand = vec3<f32>(0.60, 0.55, 0.40);
    let seabed = vec3<f32>(0.25, 0.28, 0.22);
    // steep faces and convex ridges are rock; the shore and the shallows are sand, unless
    // steep; below the water the seabed takes over. Top layer wins
    let rock_w = smoothstep(0.30, 0.55, slope - 0.15 * curv + 0.08 * (n - 0.5));
    let sand_w = (1.0 - smoothstep(w, w + 0.02, h01)) * (1.0 - smoothstep(0.30, 0.50, slope));
    let seabed_w = 1.0 - smoothstep(w - 0.04, w - 0.02, h01);

    let color = mix(mix(mix(grass, rock, rock_w), sand, sand_w), seabed, seabed_w);
    let roughness = mix(mix(mix(0.95, 0.85, rock_w), 0.90, sand_w), 0.95, seabed_w);
    pbr_input.material.base_color = vec4<f32>(color, 1.0);
    pbr_input.material.perceptual_roughness = roughness;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
