use eframe::egui::{self, PointerButton};
use image::EncodableLayout;
use three_d::{
    core::prelude::Srgba, degrees, radians, vec2, vec3, AmbientLight, Camera, ClearState,
    CpuMaterial, CpuMesh, CpuTexture, Cull, DirectionalLight, Gm, Indices, InnerSpace, Mat3, Mat4,
    Mesh, PhysicalMaterial, Positions, TextureData, Vec3,
};

use crate::worldgen::ExportMap;

const ZSCALE: f32 = 200.0;
const XY_SCALE: f32 = 500.0;
const PANEL3D_SIZE: f32 = 256.0;
const WATER_LEVEL_DELTA: f32 = 3.0;

#[derive(Default, Clone)]
pub struct MeshData {
    size: (usize, usize),
    vertices: Vec<three_d::Vec3>,
    indices: Vec<u32>,
    normals: Vec<three_d::Vec3>,
    uv: Vec<three_d::Vec2>,
}

#[derive(Clone, Copy)]
pub struct Panel3dViewConf {
    /// camera x and y orbit angles
    pub orbit: three_d::Vec2,
    /// camera x and y pan distances
    pub pan: three_d::Vec2,
    /// camera zoom in degrees (y field of view is 90 - zoom)
    pub zoom: f32,
    /// vertical scale to apply to the heightmap
    pub hscale: f32,
    /// water plane z position
    pub water_level: f32,
    /// do we display the water plane ?
    pub show_water: bool,
    /// do we display the skybox ?
    pub show_skybox: bool,
}

pub struct Panel3dView {
    size: f32,
    conf: Panel3dViewConf,
    mesh_data: MeshData,
    mesh_updated: bool,
}

impl Default for Panel3dView {
    fn default() -> Self {
        Self {
            size: PANEL3D_SIZE,
            conf: Panel3dViewConf {
                pan: three_d::Vec2::new(0.0, 0.0),
                orbit: three_d::Vec2::new(std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_4),
                zoom: 60.0,
                hscale: 100.0,
                water_level: 40.0,
                show_water: true,
                show_skybox: true,
            },
            mesh_data: Default::default(),
            mesh_updated: false,
        }
    }
}

impl Panel3dView {
    pub fn new(size: f32) -> Self {
        Self {
            size,
            ..Default::default()
        }
    }
    pub fn render(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                self.render_3dview(ui);
            });
            ui.horizontal(|ui| {
                ui.label("Height scale %");
                ui.add(
                    egui::DragValue::new(&mut self.conf.hscale)
                        .speed(1.0)
                        .range(std::ops::RangeInclusive::new(10.0, 200.0)),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Show water plane");
                let old_show_water = self.conf.show_water;
                ui.checkbox(&mut self.conf.show_water, "");
                if old_show_water != self.conf.show_water {
                    self.update_water_level(self.conf.show_water, self.conf.water_level);
                }
                ui.label("Water height");
                let old_water_level = self.conf.water_level;
                ui.add_enabled(
                    self.conf.show_water,
                    egui::DragValue::new(&mut self.conf.water_level)
                        .speed(0.1)
                        .range(std::ops::RangeInclusive::new(0.0, 100.0)),
                );
                if old_water_level != self.conf.water_level {
                    self.update_water_level(false, old_water_level);
                    self.update_water_level(true, self.conf.water_level);
                }
                ui.label("Show skybox");
                ui.checkbox(&mut self.conf.show_skybox, "");
            });
        });
    }

    pub fn update_water_level(&mut self, show: bool, level: f32) {
        let sign = if show { 1.0 } else { -1.0 };
        for v in self.mesh_data.vertices.iter_mut() {
            let delta = v.z - level;
            if delta > 0.0 {
                v.z += sign * WATER_LEVEL_DELTA;
            } else {
                v.z -= sign * WATER_LEVEL_DELTA;
            }
        }
        self.mesh_updated = true;
    }

    pub fn update_mesh(&mut self, hmap: &ExportMap) {
        let size = hmap.get_size();
        self.mesh_data.size = size;
        self.mesh_data.vertices = Vec::with_capacity(size.0 * size.1);
        let grid_size = (XY_SCALE / size.0 as f32, XY_SCALE / size.1 as f32);
        let off_x = -0.5 * grid_size.0 * size.0 as f32;
        let off_y = -0.5 * grid_size.1 * size.1 as f32;
        let (min, max) = hmap.get_min_max();
        let coef = ZSCALE
            * if max - min > std::f32::EPSILON {
                1.0 / (max - min)
            } else {
                1.0
            };
        let ucoef = 1.0 / size.0 as f32;
        let vcoef = 1.0 / size.1 as f32;
        for y in 0..size.1 {
            let vy = y as f32 * grid_size.1 + off_y;
            for x in 0..size.0 {
                let vx = x as f32 * grid_size.0 + off_x;
                let mut vz = hmap.height(x, y);
                vz = (vz - min) * coef;
                self.mesh_data.vertices.push(three_d::vec3(vx, -vy, vz));
                self.mesh_data
                    .uv
                    .push(three_d::vec2(x as f32 * ucoef, y as f32 * vcoef));
            }
        }
        if self.conf.show_water {
            self.update_water_level(true, self.conf.water_level);
        }
        self.mesh_data.indices = Vec::with_capacity(6 * (size.1 - 1) * (size.0 - 1));
        for y in 0..size.1 - 1 {
            let y_offset = y * size.0;
            for x in 0..size.0 - 1 {
                let off = x + y_offset;
                self.mesh_data.indices.push((off) as u32);
                self.mesh_data.indices.push((off + size.0) as u32);
                self.mesh_data.indices.push((off + 1) as u32);
                self.mesh_data.indices.push((off + size.0) as u32);
                self.mesh_data.indices.push((off + size.0 + 1) as u32);
                self.mesh_data.indices.push((off + 1) as u32);
            }
        }
        let mut cpu_mesh = three_d::CpuMesh {
            positions: three_d::Positions::F32(self.mesh_data.vertices.clone()),
            indices: three_d::Indices::U32(self.mesh_data.indices.clone()),
            ..Default::default()
        };
        cpu_mesh.compute_normals();
        self.mesh_data.normals = cpu_mesh.normals.take().unwrap();
        self.mesh_updated = true;
    }

    fn render_3dview(&mut self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(egui::Vec2::splat(self.size), egui::Sense::drag());
        let lbutton = ui.input(|i| i.pointer.button_down(PointerButton::Primary));
        let rbutton = ui.input(|i| i.pointer.button_down(PointerButton::Secondary));
        let mbutton = ui.input(|i| i.pointer.button_down(PointerButton::Middle));
        if lbutton {
            self.conf.orbit[0] += response.drag_delta().x * 0.01;
            self.conf.orbit[1] += response.drag_delta().y * 0.01;
            self.conf.orbit[1] = self.conf.orbit[1].clamp(0.15, std::f32::consts::FRAC_PI_2 - 0.05);
        } else if rbutton {
            self.conf.pan[0] += response.drag_delta().x * 0.5;
            self.conf.pan[1] += response.drag_delta().y * 0.5;
            self.conf.pan[1] = self.conf.pan[1].clamp(0.0, 140.0);
        } else if mbutton {
            self.conf.zoom += response.drag_delta().y * 0.15;
        }

        // Clone locals so we can move them into the paint callback:
        let conf = self.conf;
        let mesh_updated = self.mesh_updated;
        let mesh_data: Option<MeshData> = if mesh_updated {
            Some(self.mesh_data.clone())
        } else {
            None
        };

        let callback = egui::PaintCallback {
            rect,
            callback: std::sync::Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                with_three_d_context(painter.gl(), |three_d, renderer| {
                    if mesh_updated {
                        renderer.update_model(three_d, &mesh_data);
                    }
                    renderer.render(
                        three_d,
                        &info,
                        conf,
                        FrameInput::new(&three_d, &info, painter),
                    );
                });
            })),
        };
        ui.painter().add(callback);
        self.mesh_updated = false;
    }
}
///
/// Translates from egui input to three-d input
///
pub struct FrameInput<'a> {
    screen: three_d::RenderTarget<'a>,
    viewport: three_d::Viewport,
    scissor_box: three_d::ScissorBox,
}

impl FrameInput<'_> {
    pub fn new(
        context: &three_d::Context,
        info: &egui::PaintCallbackInfo,
        painter: &egui_glow::Painter,
    ) -> Self {
        use three_d::*;

        // Disable sRGB textures for three-d
        #[cfg(not(target_arch = "wasm32"))]
        #[allow(unsafe_code)]
        unsafe {
            use egui_glow::glow::HasContext as _;
            context.disable(egui_glow::glow::FRAMEBUFFER_SRGB);
        }

        // Constructs a screen render target to render the final image to
        let screen = painter.intermediate_fbo().map_or_else(
            || {
                RenderTarget::screen(
                    context,
                    info.viewport.width() as u32,
                    info.viewport.height() as u32,
                )
            },
            |fbo| {
                RenderTarget::from_framebuffer(
                    context,
                    info.viewport.width() as u32,
                    info.viewport.height() as u32,
                    fbo,
                )
            },
        );

        // Set where to paint
        let viewport = info.viewport_in_pixels();
        let viewport = Viewport {
            x: viewport.left_px,
            y: viewport.from_bottom_px,
            width: viewport.width_px as u32,
            height: viewport.height_px as u32,
        };

        // Respect the egui clip region (e.g. if we are inside an `egui::ScrollArea`).
        let clip_rect = info.clip_rect_in_pixels();
        let scissor_box = ScissorBox {
            x: clip_rect.left_px,
            y: clip_rect.from_bottom_px,
            width: clip_rect.width_px as u32,
            height: clip_rect.height_px as u32,
        };
        Self {
            screen,
            scissor_box,
            viewport,
        }
    }
}

fn with_three_d_context<R>(
    gl: &std::sync::Arc<egui_glow::glow::Context>,
    f: impl FnOnce(&three_d::Context, &mut Renderer) -> R,
) -> R {
    use std::cell::RefCell;
    thread_local! {
        pub static THREE_D: RefCell<Option<(three_d::Context,Renderer)>> = RefCell::new(None);
    }
    #[allow(unsafe_code)]
    unsafe {
        use egui_glow::glow::HasContext as _;
        gl.enable(egui_glow::glow::DEPTH_TEST);
        if !cfg!(target_arch = "wasm32") {
            gl.disable(egui_glow::glow::FRAMEBUFFER_SRGB);
        }
        gl.clear(egui_glow::glow::DEPTH_BUFFER_BIT);
        gl.clear_depth_f32(1.0);
        gl.depth_func(egui_glow::glow::LESS);
    }
    THREE_D.with(|context| {
        let mut context = context.borrow_mut();
        let (three_d, renderer) = context.get_or_insert_with(|| {
            let three_d = three_d::Context::from_gl_context(gl.clone()).unwrap();
            let renderer = Renderer::new(&three_d);
            (three_d, renderer)
        });

        f(three_d, renderer)
    })
}
pub struct Renderer {
    terrain_mesh: CpuMesh,
    terrain_model: Gm<Mesh, PhysicalMaterial>,
    terrain_material: PhysicalMaterial,
    water_model: Gm<Mesh, PhysicalMaterial>,
    directional: DirectionalLight,
    ambient: AmbientLight,
    sky: Gm<Mesh, PhysicalMaterial>,
}

impl Renderer {
    pub fn new(three_d: &three_d::Context) -> Self {
        let terrain_mesh = CpuMesh::square();
        let mut terrain_material = PhysicalMaterial::new_opaque(
            three_d,
            &CpuMaterial {
                roughness: 1.0,
                metallic: 0.0,
                albedo: Srgba::new_opaque(45, 30, 25),
                ..Default::default()
            },
        );
        terrain_material.render_states.cull = Cull::Back;
        let terrain_model = Gm::new(Mesh::new(three_d, &terrain_mesh), terrain_material.clone());
        let water_model = build_water_plane(three_d);
        Self {
            terrain_mesh,
            terrain_model,
            terrain_material,
            water_model,
            sky: build_sky(three_d),
            directional: DirectionalLight::new(
                three_d,
                1.5,
                Srgba::new_opaque(255, 222, 180),
                vec3(-0.5, 0.5, -0.5).normalize(),
            ),
            ambient: AmbientLight::new(&three_d, 0.5, Srgba::WHITE),
        }
    }
    pub fn update_model(&mut self, three_d: &three_d::Context, mesh_data: &Option<MeshData>) {
        if let Some(mesh_data) = mesh_data {
            let mut rebuild = false;
            if let Positions::F32(ref mut vertices) = self.terrain_mesh.positions {
                rebuild = vertices.len() != mesh_data.vertices.len();
                *vertices = mesh_data.vertices.clone();
            }
            if rebuild {
                self.terrain_mesh.indices = Indices::U32(mesh_data.indices.clone());
                self.terrain_mesh.normals = Some(mesh_data.normals.clone());
                self.terrain_mesh.uvs = Some(mesh_data.uv.clone());
                self.terrain_mesh.tangents = None;
            }
            self.terrain_model = Gm::new(
                Mesh::new(three_d, &self.terrain_mesh),
                self.terrain_material.clone(),
            );
        }
    }
    pub fn render(
        &mut self,
        _three_d: &three_d::Context,
        _info: &egui::PaintCallbackInfo,
        conf: Panel3dViewConf,
        frame_input: FrameInput<'_>,
    ) {
        // Set where to paint
        let viewport = frame_input.viewport;

        let target = vec3(0.0, 0.0, 0.0);
        let campos = vec3(XY_SCALE * 2.0, 0.0, 0.0);

        let mut camera = Camera::new_perspective(
            viewport,
            campos,
            target,
            vec3(0.0, 0.0, 1.0),
            degrees((90.0 - conf.zoom * 0.8).clamp(1.0, 90.0)),
            0.1,
            XY_SCALE * 10.0,
        );

        camera.rotate_around_with_fixed_up(target, 0.0, conf.orbit[1]);

        let up = camera.up();
        let right_direction = camera.right_direction();
        camera.translate(conf.pan[1] * up - conf.pan[0] * right_direction);
        let camz = camera.position().z;
        if camz < conf.water_level + 10.0 {
            camera.translate(vec3(0.0, 0.0, conf.water_level + 10.0 - camz));
        }

        let mut transfo = Mat4::from_angle_z(radians(conf.orbit[0] * 2.0));
        transfo.z[2] = conf.hscale / 100.0;
        self.terrain_model.set_transformation(transfo);

        let light_transfo = Mat3::from_angle_z(radians(conf.orbit[0] * 2.0));
        self.directional.direction = light_transfo * vec3(-0.5, 0.5, -0.5);
        self.directional
            .generate_shadow_map(1024, &[&self.terrain_model]);
        // Get the screen render target to be able to render something on the screen
        frame_input
            .screen
            // Clear the color and depth of the screen render target
            .clear_partially(frame_input.scissor_box, ClearState::depth(1.0));
        frame_input.screen.render_partially(
            frame_input.scissor_box,
            &camera,
            &[&self.terrain_model],
            &[&self.ambient, &self.directional],
        );

        if conf.show_water {
            let mut water_transfo =
                Mat4::from_translation(Vec3::new(0.0, 0.0, conf.water_level * conf.hscale * 0.01));
            water_transfo.x[0] = XY_SCALE * 10.0;
            water_transfo.y[1] = XY_SCALE * 10.0;
            self.water_model.set_transformation(water_transfo);

            frame_input.screen.render_partially(
                frame_input.scissor_box,
                &camera,
                &[&self.water_model],
                &[&self.ambient, &self.directional],
            );
        }
        if conf.show_skybox {
            let transfo = Mat4::from_angle_z(radians(conf.orbit[0] * 2.0));
            self.sky.set_transformation(transfo);
            frame_input.screen.render_partially(
                frame_input.scissor_box,
                &camera,
                &[&self.sky],
                &[],
            );
        }

        frame_input.screen.into_framebuffer(); // Take back the screen fbo, we will continue to use it.
    }
}

const SKY_BYTES: &[u8] = include_bytes!("../sky.jpg");

fn build_sky(three_d: &three_d::Context) -> Gm<Mesh, PhysicalMaterial> {
    let img = image::load_from_memory(SKY_BYTES).unwrap();
    let buffer = img.as_rgb8().unwrap().as_bytes();
    let mut data = Vec::new();
    let mut i = 0;
    while i < (img.height() * img.width() * 3) as usize {
        let r = buffer[i];
        let g = buffer[i + 1];
        let b = buffer[i + 2];
        i += 3;
        data.push([r, g, b]);
    }
    const SUBDIV: u32 = 32;
    let mut sky2 = uv_wrapping_cylinder(SUBDIV);
    sky2.transform(Mat4::from_nonuniform_scale(
        ZSCALE * 5.0,
        XY_SCALE * 2.0,
        XY_SCALE * 2.0,
    ))
    .unwrap();
    sky2.transform(Mat4::from_angle_y(degrees(-90.0))).unwrap();
    sky2.transform(Mat4::from_angle_z(degrees(90.0))).unwrap();
    let mut sky_material = PhysicalMaterial::new_opaque(
        three_d,
        &CpuMaterial {
            roughness: 1.0,
            metallic: 0.0,
            emissive: Srgba::WHITE,
            emissive_texture: Some(CpuTexture {
                width: img.width(),
                height: img.height(),
                data: TextureData::RgbU8(data),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    // water_material.render_states.depth_test = DepthTest::Greater;
    sky_material.render_states.cull = Cull::Front;
    Gm::new(Mesh::new(three_d, &sky2), sky_material)
}
fn build_water_plane(three_d: &three_d::Context) -> Gm<Mesh, PhysicalMaterial> {
    let water_mesh = CpuMesh::square();

    let mut water_material = PhysicalMaterial::new_opaque(
        three_d,
        &CpuMaterial {
            roughness: 0.1,
            metallic: 0.2,
            albedo: Srgba::new_opaque(50, 60, 150),
            ..Default::default()
        },
    );
    // water_material.render_states.depth_test = DepthTest::Greater;
    water_material.render_states.cull = Cull::Back;
    Gm::new(Mesh::new(three_d, &water_mesh), water_material)
}
fn uv_wrapping_cylinder(angle_subdivisions: u32) -> CpuMesh {
    let length_subdivisions = 1;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for i in 0..length_subdivisions + 1 {
        let x = i as f32 / length_subdivisions as f32;
        for j in 0..angle_subdivisions + 1 {
            let angle = 2.0 * std::f32::consts::PI * j as f32 / angle_subdivisions as f32;

            positions.push(vec3(x, angle.cos(), angle.sin()));
        }
    }
    for i in 0..length_subdivisions {
        for j in 0..angle_subdivisions {
            indices.push((i * (angle_subdivisions + 1) + j) as u16);
            indices.push((i * (angle_subdivisions + 1) + (j + 1)) as u16);
            indices.push(((i + 1) * (angle_subdivisions + 1) + (j + 1)) as u16);

            indices.push((i * (angle_subdivisions + 1) + j) as u16);
            indices.push(((i + 1) * (angle_subdivisions + 1) + (j + 1)) as u16);
            indices.push(((i + 1) * (angle_subdivisions + 1) + j) as u16);
        }
    }
    let mut uvs = Vec::new();
    for i in 0..angle_subdivisions + 1 {
        let u = i as f32 / angle_subdivisions as f32;
        uvs.push(vec2(u, 1.0));
    }
    for i in 0..angle_subdivisions + 1 {
        let u = i as f32 / angle_subdivisions as f32;
        uvs.push(vec2(u, 0.0));
    }
    let mut mesh = CpuMesh {
        // name: "cylinder".to_string(),
        positions: Positions::F32(positions),
        indices: Indices::U16(indices),
        uvs: Some(uvs),
        ..Default::default()
    };
    mesh.compute_normals();
    mesh
}
