//! The 3D preview scene: the two Bevy cameras behind the egui panels (one carries the egui
//! context and clears the window, one draws the scene into an offscreen image that the
//! "3d preview" square paints), the terrain mesh, the sun, and the systems that apply
//! `Panel3dViewConf` to them every frame.
//!
//! Bevy is Y-up: a heightmap cell `(x, y)` with height `h` is the vertex `[vx, h, vy]`, the
//! grid is centred on the origin and spans `XY_SCALE`, heights are normalised to `0..ZSCALE`.
use crate::panel_3dview::Panel3dViewConf;
use crate::terrain_material::{terrain_settings, TerrainExtension, TerrainMaterial};
use crate::water_material::{
    heightmap_image, water_normal_map, water_settings, WaterExtension, WaterMaterial,
};
use crate::MyApp;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Exposure, Hdr, RenderTarget};
use bevy::core_pipeline::prepass::{DeferredPrepass, DepthPrepass};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{
    Atmosphere, AtmosphereEnvironmentMapLight, CascadeShadowConfigBuilder,
    DirectionalLightShadowMap, SunDisk,
};
use bevy::material::OpaqueRendererMethod;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{AtmosphereSettings, ExtendedMaterial, ScreenSpaceReflections};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureFormat};
use bevy_egui::{EguiGlobalSettings, EguiTextureHandle, EguiUserTextures, PrimaryEguiContext};

/// height of the highest vertex of the terrain mesh, in scene units
pub const ZSCALE: f32 = 200.0;
/// side of the terrain mesh, in scene units; the camera orbits at twice this distance
pub const XY_SCALE: f32 = 500.0;
/// metres per scene unit for the atmosphere: the map is 2.5 km wide, `ZSCALE` 1 km of relief
pub const UNIT_M: f32 = 5.0;
/// ambient light when the sky is off, in cd/m²: a stop below the sun's `RAW_SUNLIGHT`
/// after the fixed exposure, so shadows read dark grey rather than black
pub const AMBIENT_NO_SKY: f32 = 5000.0;

/// marker of the camera that renders the 3D scene into the preview square
#[derive(Component)]
pub struct SceneCamera;
/// marker of the terrain entity; spawned without a mesh, given one by the first heightmap and
/// a new one by every next one
#[derive(Component)]
pub struct Terrain;
/// marker of the directional light
#[derive(Component)]
pub struct Sun;
/// marker of the water plane, a child of the terrain so it follows its spin and height scale
#[derive(Component)]
pub struct Water;
/// marker of the skirt: the four vertical walls from the terrain's border down to the map
/// minimum, a child of the terrain like the water plane, so the preview reads as a block of
/// land rather than a sheet
#[derive(Component)]
pub struct Skirt;
/// seed of the ripple normal map's wave phases
const RIPPLE_SEED: u64 = 7;
/// the skirt's dirt colour
const SKIRT_COLOR: Color = Color::srgb(0.40, 0.28, 0.17);

/// the image the scene camera renders into, and its egui texture id: the 3D panel paints it
/// over its square. The handle never changes (the asset is resized in place), so the id is
/// stable for the whole session
#[derive(Resource)]
pub struct SceneTarget {
    pub image: Handle<Image>,
    pub texture: egui::TextureId,
}

/// what the UI decided for the 3D square this frame: its size on screen and how the scene
/// is viewed; written by the UI, read by `apply_view_conf` and `apply_target`
#[derive(Resource, Default)]
pub struct PreviewViewport {
    /// the square in egui points, used for its size only; `None` when the "3d preview"
    /// header is collapsed
    pub rect: Option<egui::Rect>,
    /// egui points to physical pixels
    pub pixels_per_point: f32,
    /// camera and scene settings of the "3d preview" panel
    pub conf: Panel3dViewConf,
}

/// egui camera (order 0): clears the whole window to the panel colour and draws only egui.
/// scene camera (order -1): renders into the `SceneTarget` image before egui paints it,
/// deferred (gbuffer through the depth and deferred prepasses, no MSAA).
pub fn spawn_cameras(
    mut commands: Commands,
    mut settings: ResMut<EguiGlobalSettings>,
    mut images: ResMut<Assets<Image>>,
    mut egui_textures: ResMut<EguiUserTextures>,
) {
    // otherwise bevy_egui attaches its context to the first camera it sees
    settings.auto_create_primary_context = false;
    // 1×1 until the first frame tells `apply_target` the square's size
    let image = images.add(Image::new_target_texture(
        1,
        1,
        TextureFormat::Bgra8UnormSrgb,
        None,
    ));
    let texture = egui_textures.add_image(EguiTextureHandle::Strong(image.clone()));
    commands.insert_resource(SceneTarget { image: image.clone(), texture });
    commands.spawn((
        PrimaryEguiContext,
        Camera2d,
        RenderLayers::none(),
        Camera {
            order: 0,
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(27, 27, 27)),
            ..default()
        },
    ));
    commands.spawn((
        SceneCamera,
        Camera3d::default(),
        Camera {
            order: -1,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(10, 10, 10)),
            ..default()
        },
        RenderTarget::from(image),
        Hdr,
        Tonemapping::TonyMcMapface,
        Exposure {
            ev100: Panel3dViewConf::default().exposure,
        },
        Msaa::Off,
        DepthPrepass,
        DeferredPrepass,
        // the fade-in range starts at 0 so the near-mirror water (roughness 0.05) reflects;
        // thickness and steps are in scene units of a 500-wide terrain
        ScreenSpaceReflections {
            min_perceptual_roughness: 0.0..0.0,
            thickness: 2.0,
            linear_steps: 16,
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// an Earth atmosphere placed so that the scene origin sits on the planet's surface, in
/// `UNIT_M` metres per scene unit; `apply_sky` decides whether the camera renders it
pub fn spawn_sky(mut commands: Commands, mut media: ResMut<Assets<ScatteringMedium>>) {
    let atmosphere = Atmosphere::earth(media.add(ScatteringMedium::earth(256, 256)));
    let transform = Transform::from_translation(-Vec3::Y * (atmosphere.inner_radius / UNIT_M))
        .with_scale(Vec3::splat(1.0 / UNIT_M));
    commands.spawn((atmosphere, transform));
}

/// turns the atmosphere on and off on the scene camera with "Show skybox": on, the sky is
/// rendered, lights the scene through its environment map and the flat ambient goes to 0;
/// off, the ambient is `AMBIENT_NO_SKY`. Nothing is written while the state matches
pub fn apply_sky(
    vp: Res<PreviewViewport>,
    mut commands: Commands,
    mut ambient: ResMut<GlobalAmbientLight>,
    cam: Single<(Entity, Option<&AtmosphereSettings>), With<SceneCamera>>,
) {
    let (entity, settings) = *cam;
    match (vp.conf.show_skybox, settings.is_some()) {
        (true, false) => {
            commands.entity(entity).insert((
                AtmosphereSettings::default(),
                AtmosphereEnvironmentMapLight::default(),
            ));
            ambient.brightness = 0.0;
        }
        (false, true) => {
            commands
                .entity(entity)
                .remove::<(AtmosphereSettings, AtmosphereEnvironmentMapLight)>();
            ambient.brightness = AMBIENT_NO_SKY;
        }
        _ => {}
    }
}

/// the sun, the ambient light, and the terrain entity with its material but no mesh (it draws
/// nothing until `update_terrain` hands it the first heightmap), with the water plane and the
/// skirt as its children
pub fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut terrain_materials: ResMut<Assets<TerrainMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Sun,
        DirectionalLight {
            color: Color::srgb_u8(255, 222, 180),
            // unfiltered sunlight, what the atmosphere expects to attenuate
            illuminance: light_consts::lux::RAW_SUNLIGHT,
            shadow_maps_enabled: true,
            ..default()
        },
        SunDisk::EARTH,
        // the camera sits 2 * XY_SCALE from the centre and the terrain spans XY_SCALE / 2
        // each way, so the visible depth is about 1.5 .. 3.5 * XY_SCALE: the cascade splits
        // spread over 0.6 .. 6 * XY_SCALE
        CascadeShadowConfigBuilder {
            num_cascades: 4,
            minimum_distance: 0.1,
            maximum_distance: XY_SCALE * 6.0,
            first_cascade_far_bound: XY_SCALE * 0.6,
            overlap_proportion: 0.2,
        }
        .build(),
        Transform::default(),
    ));
    commands.insert_resource(DirectionalLightShadowMap { size: 2048 });
    // Bevy's default ambient is night-dark next to a 130 000 lx sun
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: AMBIENT_NO_SKY,
        ..default()
    });
    // a +Y plane 10 terrain sides wide, centred like the terrain, so its uv maps to the
    // terrain's by one affine step in `water.wgsl`; `apply_view_conf` sets its local height
    // and visibility. Until the first terrain the heightmap is one zero texel: all deep
    let plane = Plane3d::default()
        .mesh()
        .size(XY_SCALE * 10.0, XY_SCALE * 10.0);
    let water = (
        Water,
        Mesh3d(meshes.add(plane)),
        MeshMaterial3d(water_materials.add(ExtendedMaterial {
            base: StandardMaterial {
                base_color: Color::BLACK,
                perceptual_roughness: 0.05,
                metallic: 0.0,
                reflectance: 0.5,
                opaque_render_method: OpaqueRendererMethod::Auto,
                ..default()
            },
            extension: WaterExtension {
                settings: water_settings(&Panel3dViewConf::default()),
                heightmap: images.add(heightmap_image((1, 1), &[0.0])),
                ripples: images.add(water_normal_map(RIPPLE_SEED)),
            },
        })),
        Transform::default(),
        Visibility::Hidden,
    );
    // like the terrain, meshless until the first heightmap
    let skirt = (
        Skirt,
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: SKIRT_COLOR,
            perceptual_roughness: 1.0,
            metallic: 0.0,
            opaque_render_method: OpaqueRendererMethod::Auto,
            ..default()
        })),
        Transform::default(),
        Visibility::default(),
    );
    commands
        .spawn((
            Terrain,
            Transform::default(),
            Visibility::default(),
            MeshMaterial3d(terrain_materials.add(ExtendedMaterial {
                base: StandardMaterial {
                    base_color: Color::WHITE,
                    perceptual_roughness: 1.0,
                    metallic: 0.0,
                    opaque_render_method: OpaqueRendererMethod::Auto,
                    ..default()
                },
                extension: TerrainExtension {
                    settings: terrain_settings(&Panel3dViewConf::default()),
                },
            })),
        ))
        .with_children(|parent| {
            parent.spawn(water);
            parent.spawn(skirt);
        });
}

/// the mapping from heightmap cells to scene positions shared by the terrain and skirt
/// meshes: the grid centred on the origin over `XY_SCALE`, heights rescaled to `0..ZSCALE`
struct Grid {
    step: (f32, f32),
    off: (f32, f32),
    min: f32,
    coef: f32,
}

impl Grid {
    fn new(size: (usize, usize), h: &[f32]) -> Self {
        let step = (XY_SCALE / size.0 as f32, XY_SCALE / size.1 as f32);
        let (min, max) = crate::generators::get_min_max(h);
        let coef = ZSCALE
            * if max - min > f32::EPSILON {
                1.0 / (max - min)
            } else {
                1.0
            };
        Self {
            step,
            off: (-0.5 * step.0 * size.0 as f32, -0.5 * step.1 * size.1 as f32),
            min,
            coef,
        }
    }

    /// the cell's `[vx, vy]` on the scene's ground plane
    fn xy(&self, (x, y): (usize, usize)) -> [f32; 2] {
        [
            x as f32 * self.step.0 + self.off.0,
            y as f32 * self.step.1 + self.off.1,
        ]
    }

    /// a raw height in scene units
    fn height(&self, h: f32) -> f32 {
        (h - self.min) * self.coef
    }
}

/// the heightmap as a lit triangle mesh: one vertex per cell, `[vx, h, vy]`, the grid centred
/// on the origin over `XY_SCALE`, heights rescaled to `0..ZSCALE`, smooth normals, and
/// `UV_1 = [h01, curv01]` for the terrain shader (normalised height, cell curvature)
pub fn terrain_mesh(size: (usize, usize), h: &[f32]) -> Mesh {
    let grid = Grid::new(size, h);
    let mut positions = Vec::with_capacity(size.0 * size.1);
    let mut uvs = Vec::with_capacity(size.0 * size.1);
    let mut uvs_b = Vec::with_capacity(size.0 * size.1);
    for y in 0..size.1 {
        for x in 0..size.0 {
            let [vx, vy] = grid.xy((x, y));
            let hv = grid.height(h[x + y * size.0]);
            positions.push([vx, hv, vy]);
            uvs.push([x as f32 / size.0 as f32, y as f32 / size.1 as f32]);
            uvs_b.push([hv / ZSCALE, curvature01(size, h, (x, y), grid.coef)]);
        }
    }
    let mut indices = Vec::with_capacity(6 * (size.0 - 1) * (size.1 - 1));
    for y in 0..size.1 - 1 {
        for x in 0..size.0 - 1 {
            let off = (x + y * size.0) as u32;
            let w = size.0 as u32;
            indices.extend_from_slice(&[off, off + w, off + 1, off + w, off + w + 1, off + 1]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, uvs_b)
    .with_inserted_indices(Indices::U32(indices));
    mesh.compute_smooth_normals();
    mesh
}

/// the four vertical walls closing the terrain into a block: from every border vertex of
/// `terrain_mesh` straight down to height 0 (the map minimum, the water plane's lowest
/// position), one flat outward normal per wall, no UVs
pub fn skirt_mesh(size: (usize, usize), h: &[f32]) -> Mesh {
    let grid = Grid::new(size, h);
    let (w, d) = size;
    let mut positions = Vec::with_capacity(4 * (w + d));
    let mut normals = Vec::with_capacity(4 * (w + d));
    let mut indices = Vec::with_capacity(12 * (w + d - 2));
    let sides: [(Vec<(usize, usize)>, Vec3); 4] = [
        ((0..d).map(|y| (0, y)).collect(), Vec3::NEG_X),
        ((0..d).map(|y| (w - 1, y)).collect(), Vec3::X),
        ((0..w).map(|x| (x, 0)).collect(), Vec3::NEG_Z),
        ((0..w).map(|x| (x, d - 1)).collect(), Vec3::Z),
    ];
    for (edge, outward) in sides {
        // vertex 2i is the top of border cell i, 2i + 1 its foot; the quad order
        // top_i, top_i+1, foot_i+1, foot_i faces along `edge × down`, flipped when that is
        // not the outward side (Bevy's front face is counter-clockwise)
        let base = positions.len() as u32;
        for &cell in &edge {
            let [vx, vy] = grid.xy(cell);
            positions.push([vx, grid.height(h[cell.0 + cell.1 * w]), vy]);
            positions.push([vx, 0.0, vy]);
            normals.push(outward.to_array());
            normals.push(outward.to_array());
        }
        let [x0, y0] = grid.xy(edge[0]);
        let [x1, y1] = grid.xy(edge[1]);
        let along = Vec3::new(x1 - x0, 0.0, y1 - y0);
        let flip = along.cross(Vec3::NEG_Y).dot(outward) < 0.0;
        for i in 0..edge.len() as u32 - 1 {
            let (t0, f0, t1, f1) = (base + 2 * i, base + 2 * i + 1, base + 2 * i + 2, base + 2 * i + 3);
            let quad = if flip {
                [t0, f1, t1, t0, f0, f1]
            } else {
                [t0, t1, f1, t0, f1, f0]
            };
            indices.extend_from_slice(&quad);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

/// the cell's curvature mapped to `0..1`: `0.5 + 0.5 * clamp(c, -1, 1)` with `c` the mean of
/// the 4 edge neighbours minus the cell, in scene units (`coef`), per `XY_SCALE / 512` — so
/// `< 0.5` on a ridge, `> 0.5` in a valley, `0.5` on a plane or a constant slope, and one
/// 512-cell of drop per cell on both flanks saturates. Border cells use their clamped neighbour
fn curvature01(size: (usize, usize), h: &[f32], (x, y): (usize, usize), coef: f32) -> f32 {
    let at = |x: usize, y: usize| h[x + y * size.0];
    let left = at(x.saturating_sub(1), y);
    let right = at((x + 1).min(size.0 - 1), y);
    let up = at(x, y.saturating_sub(1));
    let down = at(x, (y + 1).min(size.1 - 1));
    let mean = 0.25 * (left + right + up + down);
    let c = (mean - at(x, y)) * coef / (XY_SCALE / 512.0);
    0.5 + 0.5 * c.clamp(-1.0, 1.0)
}

/// turns the heightmap the UI left in `MyApp::pending_terrain` into the terrain's and the
/// skirt's meshes — inserted on the first one, replaced afterwards (the previous asset is
/// freed with its handle) — and into the water material's heightmap texture (a new handle
/// each time, which marks the material modified so its bind group is rebuilt)
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn update_terrain(
    mut app: ResMut<MyApp>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    terrain: Single<(Entity, Option<&mut Mesh3d>), (With<Terrain>, Without<Skirt>)>,
    skirt: Single<(Entity, Option<&mut Mesh3d>), (With<Skirt>, Without<Terrain>)>,
    water: Single<&MeshMaterial3d<WaterMaterial>, With<Water>>,
) {
    let Some(hmap) = app.pending_terrain.take() else {
        return;
    };
    let size = hmap.get_size();
    let mut set_mesh = |(entity, mesh): (Entity, Option<Mut<Mesh3d>>), handle| match mesh {
        Some(mut mesh) => mesh.0 = handle,
        None => {
            commands.entity(entity).insert(Mesh3d(handle));
        }
    };
    set_mesh(terrain.into_inner(), meshes.add(terrain_mesh(size, hmap.borrow())));
    set_mesh(skirt.into_inner(), meshes.add(skirt_mesh(size, hmap.borrow())));
    if let Some(mut material) = water_materials.get_mut(&water.0) {
        material.extension.heightmap = images.add(heightmap_image(size, hmap.borrow()));
    }
}

/// applies the panel's settings to the scene: camera orbit / pan / zoom and exposure, terrain
/// rotation and height scale, sun direction, water height and visibility.
/// Component writes only: the mesh is never rebuilt for a settings change. The `Without`
/// filters make the four `&mut Transform` queries disjoint, which Bevy checks at startup
#[allow(clippy::type_complexity)]
pub fn apply_view_conf(
    vp: Res<PreviewViewport>,
    cam: Single<
        (&mut Transform, &mut Projection, &mut Camera, &mut Exposure),
        With<SceneCamera>,
    >,
    terrain: Option<Single<&mut Transform, (With<Terrain>, Without<SceneCamera>)>>,
    sun: Single<&mut Transform, (With<Sun>, Without<SceneCamera>, Without<Terrain>)>,
    water: Single<
        (&mut Transform, &mut Visibility),
        (
            With<Water>,
            Without<SceneCamera>,
            Without<Terrain>,
            Without<Sun>,
        ),
    >,
) {
    let conf = &vp.conf;
    let (mut cam_transform, mut projection, mut camera, mut exposure) = cam.into_inner();
    *cam_transform = camera_transform(conf);
    let want = perspective(conf);
    // written only on change, so `camera_system` recomputes the aspect ratio only when needed
    if !matches!(&*projection, Projection::Perspective(p) if p.fov == want.fov && p.far == want.far)
    {
        *projection = Projection::Perspective(want);
    }
    if exposure.ev100 != conf.exposure {
        exposure.ev100 = conf.exposure;
    }
    // near-black background; with the sky on, `render_sky` paints over it
    camera.clear_color = ClearColorConfig::Custom(Color::srgb_u8(10, 10, 10));
    let spin = Quat::from_rotation_y(conf.orbit[0] * 2.0);
    if let Some(terrain) = terrain {
        *terrain.into_inner() = Transform {
            rotation: spin,
            scale: Vec3::new(1.0, conf.hscale / 100.0, 1.0),
            translation: Vec3::ZERO,
        };
    }
    // a `DirectionalLight` shines along its -Z
    *sun.into_inner() = Transform::default().looking_to(sun_direction(conf), Vec3::Y);
    // the plane is the terrain's child: its local height is the contour `water_level`, the
    // parent's scale and spin place it in the world
    let (mut water_transform, mut water_visibility) = water.into_inner();
    *water_transform = Transform::from_xyz(0.0, conf.water_level, 0.0);
    *water_visibility = if conf.show_water {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
}

/// the direction the sun shines along: azimuth fixed at 225° in the terrain's frame,
/// `sun_elevation` degrees above the horizon, turned with the terrain by `orbit[0]`
fn sun_direction(conf: &Panel3dViewConf) -> Vec3 {
    let (sin, cos) = conf.sun_elevation.to_radians().sin_cos();
    let spin = Quat::from_rotation_y(conf.orbit[0] * 2.0);
    spin * Vec3::new(
        -cos * std::f32::consts::FRAC_1_SQRT_2,
        -sin,
        -cos * std::f32::consts::FRAC_1_SQRT_2,
    )
}

/// the camera at `2 * XY_SCALE` from the origin, raised by `orbit[1]`, panned in its own
/// plane, and never below `water_level + 10`
fn camera_transform(conf: &Panel3dViewConf) -> Transform {
    let d = 2.0 * XY_SCALE;
    let e = conf.orbit[1];
    let mut t = Transform::from_translation(Vec3::new(d * e.cos(), d * e.sin(), 0.0))
        .looking_at(Vec3::ZERO, Vec3::Y);
    t.translation += conf.pan[1] * *t.up() - conf.pan[0] * *t.right();
    if t.translation.y < conf.water_level + 10.0 {
        t.translation.y = conf.water_level + 10.0;
    }
    t
}

/// vertical fov `90 - zoom * 0.8` degrees, clamped to `1..90`; `aspect_ratio` is left to
/// `camera_system`, which sets it from the viewport
fn perspective(conf: &Panel3dViewConf) -> PerspectiveProjection {
    PerspectiveProjection {
        fov: (90.0 - conf.zoom * 0.8).clamp(1.0, 90.0).to_radians(),
        near: 0.1,
        far: XY_SCALE * 10.0,
        ..default()
    }
}

/// sizes the scene camera's image to the preview square, in physical pixels: the asset is
/// resized in place on a change (same handle, so the egui texture id stays valid); the camera
/// is inactive while the square is hidden or zero-sized, so the target is never empty
pub fn apply_target(
    vp: Res<PreviewViewport>,
    target: Res<SceneTarget>,
    mut images: ResMut<Assets<Image>>,
    mut cam: Single<&mut Camera, With<SceneCamera>>,
) {
    let Some(size) = target_size(vp.rect, vp.pixels_per_point) else {
        cam.is_active = false;
        return;
    };
    cam.is_active = true;
    let Some(image) = images.get(&target.image) else {
        return;
    };
    if image.size() != size {
        // `get_mut` alone marks the asset modified: taken only when the size really changed
        if let Some(mut image) = images.get_mut(&target.image) {
            image.resize(Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            });
        }
    }
}

/// the square's size in physical pixels; `None` when it is absent or a side rounds to 0
fn target_size(rect: Option<egui::Rect>, pixels_per_point: f32) -> Option<UVec2> {
    let rect = rect?;
    let size = (rect.size() * pixels_per_point).round();
    if size.x < 1.0 || size.y < 1.0 {
        None
    } else {
        Some(UVec2::new(size.x as u32, size.y as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_mesh_layout() {
        let size = (4, 3);
        let h: Vec<f32> = (0..12).map(|v| v as f32).collect();
        let mesh = terrain_mesh(size, &h);
        let pos = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .unwrap();
        assert_eq!(pos.len(), 12);
        assert_eq!(uv1(&mesh).len(), 12);
        assert_eq!(mesh.indices().unwrap().len(), 36);
        assert!(pos.iter().all(|p| (0.0..=ZSCALE).contains(&p[1])));
        assert_eq!(pos[11][1], ZSCALE);
        assert_eq!(pos[0][0], -0.5 * XY_SCALE);
        // the grid is centred on the origin: mean x is off + 1.5 * g
        let mean_x = pos.iter().map(|p| p[0]).sum::<f32>() / pos.len() as f32;
        let expected = -0.5 * XY_SCALE + 1.5 * (XY_SCALE / 4.0);
        assert!((mean_x - expected).abs() < 1e-3, "{mean_x} vs {expected}");
    }

    /// `[h01, curv01]` of every vertex
    fn uv1(mesh: &Mesh) -> Vec<[f32; 2]> {
        match mesh.attribute(Mesh::ATTRIBUTE_UV_1).unwrap() {
            bevy::mesh::VertexAttributeValues::Float32x2(v) => v.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn terrain_mesh_curvature() {
        let n = 9usize;
        let at = |x: usize, y: usize| x + y * n;
        let build = |f: &dyn Fn(usize) -> f32| {
            let h: Vec<f32> = (0..n * n).map(|i| f(i % n)).collect();
            uv1(&terrain_mesh((n, n), &h))
        };
        // a V valley along y: concave at the floor, convex where the profile is inverted
        let valley = build(&|x| (x as f32 - 4.0).abs());
        assert!(valley[at(4, 4)][1] > 0.5, "{:?}", valley[at(4, 4)]);
        let ridge = build(&|x| -(x as f32 - 4.0).abs());
        assert!(ridge[at(4, 4)][1] < 0.5, "{:?}", ridge[at(4, 4)]);
        // a plane and a constant ramp have no curvature (interior vertices only: the border
        // sees a clamped neighbour)
        let flat = build(&|_| 3.0);
        let ramp = build(&|x| x as f32);
        for y in 1..n - 1 {
            for x in 1..n - 1 {
                assert_eq!(flat[at(x, y)][1], 0.5, "flat {x} {y}");
                assert!((ramp[at(x, y)][1] - 0.5).abs() < 1e-5, "ramp {x} {y}");
            }
        }
        // h01 spans the map's range; a flat map is 0
        assert_eq!(ramp[at(0, 4)][0], 0.0);
        assert_eq!(ramp[at(8, 4)][0], 1.0);
        assert_eq!(flat[at(4, 4)][0], 0.0);
    }

    #[test]
    fn terrain_mesh_flat_map_has_unit_normals() {
        let mesh = terrain_mesh((5, 5), &[3.0; 25]);
        let normals = mesh
            .attribute(Mesh::ATTRIBUTE_NORMAL)
            .and_then(|a| a.as_float3())
            .unwrap();
        assert_eq!(normals.len(), 25);
        for n in normals {
            assert!(
                (n[0]).abs() < 1e-5 && (n[1] - 1.0).abs() < 1e-5 && n[2].abs() < 1e-5,
                "{n:?}"
            );
        }
    }

    #[test]
    fn skirt_mesh_walls_face_outward() {
        let size = (4, 3);
        // the minimum sits inside, so no wall triangle is degenerate
        let h: Vec<f32> = (0..12).map(|v| if v == 5 { 0.0 } else { v as f32 + 1.0 }).collect();
        let mesh = skirt_mesh(size, &h);
        let pos = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .unwrap();
        let normals = mesh
            .attribute(Mesh::ATTRIBUTE_NORMAL)
            .and_then(|a| a.as_float3())
            .unwrap();
        let Indices::U32(idx) = mesh.indices().unwrap() else {
            panic!("u16 indices");
        };
        assert_eq!(pos.len(), 4 * (4 + 3));
        assert_eq!(normals.len(), pos.len());
        assert_eq!(idx.len(), 12 * (4 + 3 - 2));
        // every foot is at the map minimum, every top on the terrain's border
        let terrain = terrain_mesh(size, &h);
        let tpos = terrain
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .unwrap();
        for pair in pos.chunks(2) {
            assert_eq!(pair[1][1], 0.0);
            assert_eq!(pair[0][0], pair[1][0]);
            assert_eq!(pair[0][2], pair[1][2]);
            assert!(tpos.contains(&pair[0]), "{:?}", pair[0]);
        }
        // the counter-clockwise normal of every triangle agrees with its stored normal
        for tri in idx.chunks(3) {
            let v = |i: u32| Vec3::from_array(pos[i as usize]);
            let (a, b, c) = (v(tri[0]), v(tri[1]), v(tri[2]));
            let face = (b - a).cross(c - a);
            let stored = Vec3::from_array(normals[tri[0] as usize]);
            assert!(face.dot(stored) > 0.0, "{tri:?}: {face:?} vs {stored:?}");
            assert!(stored.y == 0.0 && stored.length() == 1.0);
        }
    }

    /// Bevy panics at system initialisation when two `&mut` queries of one system may alias
    /// (error B0001); an empty world is enough to trigger that check
    #[test]
    fn view_systems_have_disjoint_queries() {
        let mut app = App::new();
        app.init_resource::<PreviewViewport>()
            .insert_resource(Assets::<Image>::default())
            .insert_resource(Assets::<TerrainMaterial>::default())
            .insert_resource(Assets::<WaterMaterial>::default())
            .insert_resource(SceneTarget {
                image: Handle::default(),
                texture: egui::TextureId::default(),
            })
            .insert_resource(GlobalAmbientLight::default())
            .add_systems(
                Update,
                (
                    apply_view_conf,
                    apply_sky,
                    apply_target,
                    crate::terrain_material::apply_terrain_conf,
                    crate::water_material::apply_water_conf,
                )
                    .chain(),
            );
        app.update();
    }

    /// the elevation control reproduces the former fixed `(-0.5, -0.5, -0.5)` sun at
    /// `atan(1 / sqrt 2)`, and points straight down near 90°
    #[test]
    fn sun_direction_matches_the_old_vector_and_elevation() {
        let mut conf = Panel3dViewConf {
            orbit: [0.0, 0.0],
            sun_elevation: std::f32::consts::FRAC_1_SQRT_2.atan().to_degrees(),
            ..default()
        };
        let want = Vec3::new(-0.5, -0.5, -0.5).normalize();
        assert!(sun_direction(&conf).distance(want) < 1e-4);
        conf.sun_elevation = 89.0;
        assert!(sun_direction(&conf).y < -0.999);
        // the spin turns the sun with the terrain, about Y
        conf.orbit[0] = std::f32::consts::FRAC_PI_2;
        let d = sun_direction(&conf);
        assert!(d.y < -0.999 && d.length() > 0.999 && d.length() < 1.001);
    }

    #[test]
    fn target_size_is_the_square_in_pixels() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(300.0, 300.0));
        assert_eq!(target_size(Some(rect), 2.0), Some(UVec2::new(600, 600)));
        // no square, or a side that rounds to nothing: no target
        assert_eq!(target_size(None, 1.0), None);
        let sliver = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(0.2, 300.0));
        assert_eq!(target_size(Some(sliver), 1.0), None);
    }
}
