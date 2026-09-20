//! The terrain's material: `StandardMaterial` extended by `terrain.wgsl`, which picks the
//! colour and roughness of every fragment from the terrain itself (slope for now; height and
//! curvature follow) instead of one base colour. `apply_terrain_conf` keeps the shader's
//! uniform in step with the "3d preview" panel.
use crate::panel_3dview::Panel3dViewConf;
use crate::preview3d::{PreviewViewport, SceneDirty, Terrain, ZSCALE};
use bevy::asset::embedded_asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// the shader, embedded in the binary; the crate is `worldgen` and `src` is trimmed
const SHADER_PATH: &str = "embedded://worldgen/terrain.wgsl";
/// half-height, in h01, of the band around the water plane whose terrain fragments are
/// dropped so the plane wins every depth tie at the shoreline (1 scene unit at `hscale` 100)
pub const SHORE_BAND: f32 = 1.0 / ZSCALE;

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExtension>;

/// the extension part of the material: one uniform at binding 100 (0..99 belong to the base)
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct TerrainExtension {
    #[uniform(100)]
    pub settings: TerrainSettings,
}

/// what the shader needs from the panel; heights are normalised (`h01`: 0 = map minimum,
/// 1 = map maximum). Mirrors `struct TerrainSettings` in `terrain.wgsl`
#[derive(ShaderType, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct TerrainSettings {
    /// water plane height as h01 (`conf.water_level / ZSCALE`)
    pub water_level: f32,
    /// snow line as h01; 2.0 = never
    pub snow_line: f32,
    /// surface detail strength 0..1
    pub detail: f32,
    /// fragments with `|h01 - water_level| < shore_band` are discarded; 0 when the water is hidden
    pub shore_band: f32,
}

impl MaterialExtension for TerrainExtension {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }
}

/// registers the embedded shader and the material pipeline
pub struct TerrainMaterialPlugin;

impl Plugin for TerrainMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "terrain.wgsl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default());
    }
}

/// the shader uniform for the panel's settings
pub fn terrain_settings(conf: &Panel3dViewConf) -> TerrainSettings {
    TerrainSettings {
        water_level: conf.water_level / ZSCALE,
        snow_line: 2.0,
        detail: 0.0,
        shore_band: if conf.show_water { SHORE_BAND } else { 0.0 },
    }
}

/// writes the panel's settings into the terrain material, only when they changed, so the
/// bind group is not re-prepared every frame
pub fn apply_terrain_conf(
    vp: Res<PreviewViewport>,
    mut dirty: ResMut<SceneDirty>,
    terrain: Option<Single<&MeshMaterial3d<TerrainMaterial>, With<Terrain>>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let Some(terrain) = terrain else {
        return;
    };
    let want = terrain_settings(&vp.conf);
    let Some(current) = materials.get(&terrain.0) else {
        return;
    };
    if current.extension.settings == want {
        return;
    }
    if let Some(mut material) = materials.get_mut(&terrain.0) {
        material.extension.settings = want;
        dirty.mark();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_level_is_normalised_to_zscale() {
        let conf = Panel3dViewConf {
            water_level: 40.0,
            ..Default::default()
        };
        let s = terrain_settings(&conf);
        assert_eq!(s.water_level, 0.2);
        assert_eq!(s.snow_line, 2.0);
        assert_eq!(s.detail, 0.0);
    }

    #[test]
    fn shore_band_follows_the_water_toggle() {
        let on = Panel3dViewConf {
            show_water: true,
            ..Default::default()
        };
        let off = Panel3dViewConf {
            show_water: false,
            ..Default::default()
        };
        assert_eq!(terrain_settings(&on).shore_band, SHORE_BAND);
        assert_eq!(terrain_settings(&off).shore_band, 0.0);
    }
}
