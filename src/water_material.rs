//! The water's material: `StandardMaterial` extended by `water.wgsl`, which colours every
//! fragment from the depth of the terrain below it (read from a heightmap texture, since a
//! deferred material cannot read the depth prepass it is writing) and tilts its normal with
//! a tileable ripple normal map scrolled by time. `apply_water_conf` keeps the shader's
//! uniform in step with the "3d preview" panel.
use crate::panel_3dview::Panel3dViewConf;
use crate::preview3d::{PreviewViewport, SceneDirty, Water, ZSCALE};
use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use rand::{Rng, SeedableRng};
use std::f32::consts::TAU;

/// the shader, embedded in the binary; the crate is `worldgen` and `src` is trimmed
const SHADER_PATH: &str = "embedded://worldgen/water.wgsl";
/// side of the ripple normal map, in pixels
const RIPPLE_MAP_SIZE: usize = 256;
/// slope per unit of `ripple_gradient`: the wave sum has a peak slope near 40 per tile, real
/// ripples tilt by a few tenths, and a nearly horizontal normal field is a binary ±1 one
/// whose mean is far from vertical
const RIPPLE_SLOPE: f32 = 0.04;
/// integer wave vectors of the ripple map's six sine waves: integers make it exactly tileable
const RIPPLE_WAVES: [(f32, f32); 6] = [
    (3.0, 1.0),
    (-2.0, 4.0),
    (5.0, -3.0),
    (1.0, 6.0),
    (-6.0, -2.0),
    (4.0, 4.0),
];

pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExtension>;

/// the extension part of the material: bindings 100..103 (0..99 belong to the base)
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct WaterExtension {
    #[uniform(100)]
    pub settings: WaterSettings,
    /// `h01` of the current terrain, `R32Float`, read with `textureLoad` (float32 is not
    /// filterable)
    #[texture(101, sample_type = "float", filterable = false)]
    pub heightmap: Handle<Image>,
    /// tileable ripple normal map, `Rgba8Unorm`, repeat + linear sampler
    #[texture(102)]
    #[sampler(103)]
    pub ripples: Handle<Image>,
}

/// what the shader needs from the panel; heights are normalised (`h01`: 0 = map minimum,
/// 1 = map maximum). Mirrors `struct WaterSettings` in `water.wgsl`
#[derive(ShaderType, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct WaterSettings {
    /// water plane height as h01 (`conf.water_level / ZSCALE`)
    pub water_level: f32,
    /// depth in h01 over which the colour goes from shallow to deep
    pub depth_fade: f32,
    /// ripple normal-map tiles across the terrain
    pub ripple_scale: f32,
    /// ripple scroll speed, in tiles per second
    pub ripple_speed: f32,
    /// how far the ripples tilt the normal, 0 = flat mirror
    pub ripple_strength: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

impl MaterialExtension for WaterExtension {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }
}

/// registers the embedded shader and the material pipeline
pub struct WaterMaterialPlugin;

impl Plugin for WaterMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "water.wgsl");
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default());
    }
}

/// the shader uniform for the panel's settings
pub fn water_settings(conf: &Panel3dViewConf) -> WaterSettings {
    WaterSettings {
        water_level: conf.water_level / ZSCALE,
        depth_fade: 0.05,
        ripple_scale: 60.0,
        ripple_speed: 0.03,
        ripple_strength: 0.25,
        _pad0: 0.0,
        _pad1: 0.0,
        _pad2: 0.0,
    }
}

/// the heightmap as an `R32Float` texture of its own size: pixel `(x, y)` is `h01`, the
/// height normalised to the map's range as `terrain_mesh` does (a flat map is all zeros)
pub fn heightmap_image(size: (usize, usize), h: &[f32]) -> Image {
    let (min, max) = crate::generators::get_min_max(h);
    let coef = if max - min > f32::EPSILON {
        1.0 / (max - min)
    } else {
        0.0
    };
    let mut data = Vec::with_capacity(size.0 * size.1 * 4);
    for v in &h[..size.0 * size.1] {
        data.extend_from_slice(&((v - min) * coef).to_le_bytes());
    }
    Image::new(
        Extent3d {
            width: size.0 as u32,
            height: size.1 as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// height of the ripple surface at `(u, v)`: six sine waves with the integer wave vectors
/// `RIPPLE_WAVES` and the given phases, so the field has period 1 in both `u` and `v`.
/// The map is built from its analytic gradient; this is the definition the tests check it
/// against
#[cfg(test)]
pub fn ripple_height(u: f32, v: f32, phases: &[f32; 6]) -> f32 {
    RIPPLE_WAVES
        .iter()
        .zip(phases)
        .enumerate()
        .map(|(k, (&(i, j), &phi))| (0.6 / (k + 1) as f32) * (TAU * (i * u + j * v) + phi).sin())
        .sum()
}

/// the gradient `(∂f/∂u, ∂f/∂v)` of `ripple_height`, analytically
fn ripple_gradient(u: f32, v: f32, phases: &[f32; 6]) -> (f32, f32) {
    RIPPLE_WAVES.iter().zip(phases).enumerate().fold(
        (0.0, 0.0),
        |(du, dv), (k, (&(i, j), &phi))| {
            let c = (0.6 / (k + 1) as f32) * TAU * (TAU * (i * u + j * v) + phi).cos();
            (du + c * i, dv + c * j)
        },
    )
}

/// the tileable ripple normal map: `RIPPLE_MAP_SIZE`², `Rgba8Unorm` (not sRGB), the normal
/// `normalize(-∂f/∂u, -∂f/∂v, 1)` at `RIPPLE_SLOPE` per unit of gradient, encoded as
/// `n * 0.5 + 0.5`, with a full mip chain so the
/// ripples average out to a flat surface at a distance instead of aliasing, sampled
/// repeating and trilinear. The six wave phases come from `seed`; the same seed gives the
/// same map
pub fn water_normal_map(seed: u64) -> Image {
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let phases: [f32; 6] = std::array::from_fn(|_| rng.random_range(0.0..TAU));
    let n = RIPPLE_MAP_SIZE;
    let mut level: Vec<Vec3> = (0..n * n)
        .map(|i| {
            let (u, v) = ((i % n) as f32 / n as f32, (i / n) as f32 / n as f32);
            let (du, dv) = ripple_gradient(u, v, &phases);
            Vec3::new(-du * RIPPLE_SLOPE, -dv * RIPPLE_SLOPE, 1.0).normalize()
        })
        .collect();
    let mut data = Vec::with_capacity(n * n * 4 * 4 / 3);
    let mut side = n;
    let mut mip_level_count = 0;
    loop {
        data.extend(level.iter().flat_map(encode_normal));
        mip_level_count += 1;
        if side == 1 {
            break;
        }
        level = downsample_normals(&level, side);
        side /= 2;
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: n as u32,
            height: n as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = mip_level_count;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// a normal as RGBA8, `normalize(n) * 0.5 + 0.5`, opaque
fn encode_normal(normal: &Vec3) -> [u8; 4] {
    let e = (normal.normalize() * 0.5 + 0.5) * 255.0;
    [e.x.round() as u8, e.y.round() as u8, e.z.round() as u8, 255]
}

/// the next mip level of a square normal map: every 2×2 block's mean. Levels stay
/// unnormalised so each one is the plain mean of the base level's unit normals over its
/// block (a true box filter); `encode_normal` normalises
fn downsample_normals(level: &[Vec3], side: usize) -> Vec<Vec3> {
    let half = side / 2;
    (0..half * half)
        .map(|i| {
            let (x, y) = (2 * (i % half), 2 * (i / half));
            let at = |dx: usize, dy: usize| level[x + dx + (y + dy) * side];
            0.25 * (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1))
        })
        .collect()
}

/// writes the panel's settings into the water material, only when they changed, so the
/// bind group is not re-prepared every frame
pub fn apply_water_conf(
    vp: Res<PreviewViewport>,
    mut dirty: ResMut<SceneDirty>,
    water: Option<Single<&MeshMaterial3d<WaterMaterial>, With<Water>>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
) {
    let Some(water) = water else {
        return;
    };
    let want = water_settings(&vp.conf);
    let Some(current) = materials.get(&water.0) else {
        return;
    };
    if current.extension.settings == want {
        return;
    }
    if let Some(mut material) = materials.get_mut(&water.0) {
        material.extension.settings = want;
        dirty.mark();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the image's pixels as row-major little-endian `f32`s
    fn pixels(image: &Image) -> Vec<f32> {
        image
            .data
            .as_ref()
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    #[test]
    fn heightmap_image_is_h01_row_major() {
        let ramp: Vec<f32> = (0..12).map(|v| v as f32 * 3.0 + 5.0).collect();
        let image = heightmap_image((4, 3), &ramp);
        assert_eq!(image.texture_descriptor.format, TextureFormat::R32Float);
        assert_eq!(image.width(), 4);
        assert_eq!(image.height(), 3);
        let p = pixels(&image);
        assert_eq!(p.len(), 12);
        assert_eq!(p[0], 0.0);
        assert_eq!(p[3 + 2 * 4], 1.0);
        assert!(p.windows(2).all(|w| w[0] < w[1]));
        let flat = heightmap_image((4, 3), &[7.0; 12]);
        assert!(pixels(&flat).iter().all(|&v| v == 0.0));
    }

    #[test]
    fn water_level_is_normalised_to_zscale() {
        let conf = Panel3dViewConf {
            water_level: 40.0,
            ..Default::default()
        };
        assert_eq!(water_settings(&conf).water_level, 0.2);
    }

    #[test]
    fn ripple_height_tiles() {
        let phases = [0.3, 1.1, 2.9, 4.2, 5.0, 0.7];
        for t in [0.13, 0.5, 0.87] {
            assert!((ripple_height(0.0, t, &phases) - ripple_height(1.0, t, &phases)).abs() < 1e-4);
            assert!((ripple_height(t, 0.0, &phases) - ripple_height(t, 1.0, &phases)).abs() < 1e-4);
        }
    }

    #[test]
    fn ripple_gradient_matches_finite_differences() {
        let phases = [0.3, 1.1, 2.9, 4.2, 5.0, 0.7];
        let (u, v, e) = (0.31, 0.72, 1e-4);
        let (du, dv) = ripple_gradient(u, v, &phases);
        let fd_u =
            (ripple_height(u + e, v, &phases) - ripple_height(u - e, v, &phases)) / (2.0 * e);
        let fd_v =
            (ripple_height(u, v + e, &phases) - ripple_height(u, v - e, &phases)) / (2.0 * e);
        assert!((du - fd_u).abs() < 1e-2, "{du} vs {fd_u}");
        assert!((dv - fd_v).abs() < 1e-2, "{dv} vs {fd_v}");
    }

    /// the normal encoded in one RGBA8 pixel
    fn decode(px: &[u8]) -> Vec3 {
        Vec3::new(px[0] as f32, px[1] as f32, px[2] as f32) / 255.0 * 2.0 - 1.0
    }

    #[test]
    fn water_normal_map_is_unit_normals_and_seeded() {
        let a = water_normal_map(7);
        assert_eq!(a.texture_descriptor.format, TextureFormat::Rgba8Unorm);
        assert_eq!(a.width() as usize, RIPPLE_MAP_SIZE);
        let data = a.data.as_ref().unwrap();
        // a full mip chain down to 1×1, mip 0 first
        let levels = RIPPLE_MAP_SIZE.trailing_zeros() + 1;
        assert_eq!(a.texture_descriptor.mip_level_count, levels);
        let texels: usize = (0..levels).map(|l| (RIPPLE_MAP_SIZE >> l).pow(2)).sum();
        assert_eq!(data.len(), texels * 4);
        for px in data.chunks_exact(4) {
            let n = decode(px);
            assert!((n.length() - 1.0).abs() < 0.02, "{n:?}");
            assert!(n.z > 0.0);
            assert_eq!(px[3], 255);
        }
        // mip 1 texel (0, 0) is the renormalised mean of mip 0's top-left 2×2 block
        let n = RIPPLE_MAP_SIZE;
        let at = |x: usize, y: usize| decode(&data[(x + y * n) * 4..][..4]);
        let mean = (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)).normalize();
        let mip1 = decode(&data[n * n * 4..][..4]);
        assert!(mean.distance(mip1) < 0.02, "{mean:?} vs {mip1:?}");
        // the ripples average out: the last level is nearly flat
        let last = decode(&data[data.len() - 4..]);
        assert!(last.z > 0.98, "{last:?}");
        assert!(matches!(a.sampler, ImageSampler::Descriptor(ref d)
            if d.address_mode_u == ImageAddressMode::Repeat
                && d.address_mode_v == ImageAddressMode::Repeat
                && d.mipmap_filter == ImageFilterMode::Linear));
        let b = water_normal_map(7);
        assert_eq!(a.data, b.data);
        let c = water_normal_map(8);
        assert_ne!(a.data, c.data);
    }
}
