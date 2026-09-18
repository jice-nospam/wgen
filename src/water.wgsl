// The water's fragment shader, for both the deferred prepass (gbuffer) and the forward
// pass: the standard PBR inputs with the colour picked from the depth of the terrain below
// (turquoise in the shallows, dark blue offshore, a pale line at the shore) and the normal
// tilted by a scrolling ripple normal map. Layout of `terrain.wgsl`.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#import bevy_render::globals::Globals
// the prepass view layout binds the globals (time) at 1 and declares no symbol for them
@group(0) @binding(1) var<uniform> globals: Globals;
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::globals,
}
#endif

// mirrors `water_material::WaterSettings`
struct WaterSettings {
    water_level: f32,
    depth_fade: f32,
    ripple_scale: f32,
    ripple_speed: f32,
    ripple_strength: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> water: WaterSettings;
// h01 of the terrain, one texel per heightmap cell
@group(#{MATERIAL_BIND_GROUP}) @binding(101)
var heightmap: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102)
var ripples: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103)
var ripples_sampler: sampler;

const SHALLOW: vec3<f32> = vec3<f32>(0.10, 0.45, 0.45);
const DEEP: vec3<f32> = vec3<f32>(0.01, 0.06, 0.20);
const FOAM: vec3<f32> = vec3<f32>(0.70, 0.75, 0.75);

// the terrain's h01 at terrain uv `uv`, bilinear over the four nearest texels
// (float32 textures cannot be filtered by a sampler)
fn height_at(uv: vec2<f32>) -> f32 {
    let dims = vec2<f32>(textureDimensions(heightmap));
    let p = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * (dims - 1.0);
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let i1 = min(i + 1, vec2<i32>(dims) - 1);
    let a = textureLoad(heightmap, i, 0).r;
    let b = textureLoad(heightmap, vec2<i32>(i1.x, i.y), 0).r;
    let c = textureLoad(heightmap, vec2<i32>(i.x, i1.y), 0).r;
    let d = textureLoad(heightmap, i1, 0).r;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // the plane is 10 terrain sides wide and centred like the terrain, so its uv maps to
    // the terrain's uv by this affine step; outside the map the water is deep
    let tuv = (in.uv - 0.5) * 10.0 + 0.5;
    let inside = all(tuv >= vec2<f32>(0.0)) && all(tuv <= vec2<f32>(1.0));
    let h = select(0.0, height_at(tuv), inside);
    // depth of the terrain below the surface, in h01
    let depth = water.water_level - h;
    let t = saturate(depth / water.depth_fade);
    var color = mix(SHALLOW, DEEP, t);
    // a thin pale line where the terrain meets the surface
    let foam = 1.0 - smoothstep(0.0, 0.004, depth);
    color = mix(color, FOAM, foam * 0.6);

    // two scrolling copies of the ripple map, sampled unconditionally; time only advances
    // while frames are rendered
    let time = globals.time * water.ripple_speed;
    let uv1 = tuv * water.ripple_scale + vec2<f32>(time, 0.37 * time);
    let uv2 = tuv * water.ripple_scale * 2.7 + vec2<f32>(-0.61 * time, 0.23 * time);
    let n1 = textureSample(ripples, ripples_sampler, uv1).xyz * 2.0 - 1.0;
    let n2 = textureSample(ripples, ripples_sampler, uv2).xyz * 2.0 - 1.0;
    // map x, y are the tangent-plane slopes along the plane's u (world X) and v (world Z)
    let r = n1 + 0.5 * n2;
    pbr_input.N = normalize(pbr_input.N + vec3<f32>(r.x, 0.0, r.y) * water.ripple_strength);

    pbr_input.material.base_color = vec4<f32>(color, 1.0);
    pbr_input.material.perceptual_roughness = mix(0.05, 0.6, foam);
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
