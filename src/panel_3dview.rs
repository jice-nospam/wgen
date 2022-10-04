use std::{rc::Rc, sync::Arc};

use eframe::egui::{self, PointerButton};
use image::EncodableLayout;
use three_d::{
    degrees, radians, rotation_matrix_from_dir_to_dir, vec2, vec3, AmbientLight, Camera, Color,
    ColorMaterial, CpuMaterial, CpuMesh, CpuTexture, Cull, DirectionalLight, Gm, Indices,
    InnerSpace, Instance, InstancedMesh, InstancedModel, Mat4, Model, Object, PhysicalMaterial,
    Positions, TextureData, Vec3, Viewport,
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
    pub orbit: three_d::Vec2,
    pub pan: three_d::Vec2,
    pub zoom: f32,
    pub hscale: f32,
    pub water_level: f32,
    pub show_water: bool,
    pub show_grid: bool,
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
                show_grid: false,
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
                        .clamp_range(std::ops::RangeInclusive::new(10.0, 200.0)),
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
                        .clamp_range(std::ops::RangeInclusive::new(0.0, 100.0)),
                );
                if old_water_level != self.conf.water_level {
                    self.update_water_level(false, old_water_level);
                    self.update_water_level(true, self.conf.water_level);
                }
                ui.label("Show grid");
                ui.checkbox(&mut self.conf.show_grid, "");
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
            indices: Some(three_d::Indices::U32(self.mesh_data.indices.clone())),
            ..Default::default()
        };
        cpu_mesh.compute_normals();
        self.mesh_data.normals = cpu_mesh.normals.take().unwrap();
        self.mesh_updated = true;
    }

    fn render_3dview(&mut self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(egui::Vec2::splat(self.size), egui::Sense::drag());
        let lbutton = ui.input().pointer.button_down(PointerButton::Primary);
        let rbutton = ui.input().pointer.button_down(PointerButton::Secondary);
        let mbutton = ui.input().pointer.button_down(PointerButton::Middle);
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
                    renderer.render(three_d, &info, conf);
                });
            })),
        };
        ui.painter().add(callback);
        self.mesh_updated = false;
    }
}

fn with_three_d_context<R>(
    gl: &std::sync::Arc<glow::Context>,
    f: impl FnOnce(&three_d::Context, &mut Renderer) -> R,
) -> R {
    use std::cell::RefCell;
    thread_local! {
        pub static THREE_D: RefCell<Option<(three_d::Context,Renderer)>> = RefCell::new(None);
    }
    #[allow(unsafe_code)]
    unsafe {
        use glow::HasContext as _;
        gl.enable(glow::DEPTH_TEST);
        if !cfg!(target_arch = "wasm32") {
            gl.disable(glow::FRAMEBUFFER_SRGB);
        }
        gl.clear(glow::DEPTH_BUFFER_BIT);
        gl.clear_depth_f32(1.0);
        gl.depth_func(glow::LESS);
    }
    THREE_D.with(|context| {
        let mut context = context.borrow_mut();
        let (three_d, renderer) = context.get_or_insert_with(|| unsafe {
            let three_d =
                three_d::Context::from_gl_context(Rc::from_raw(Arc::into_raw(gl.clone()))).unwrap();
            let renderer = Renderer::new(&three_d);
            (three_d, renderer)
        });

        f(three_d, renderer)
    })
}
pub struct Renderer {
    terrain_mesh: CpuMesh,
    terrain_model: Model<PhysicalMaterial>,
    terrain_material: PhysicalMaterial,
    water_model: Model<PhysicalMaterial>,
    directional: DirectionalLight,
    ambient: AmbientLight,
    sky: Model<PhysicalMaterial>,
    wireframe: Option<Gm<InstancedMesh, ColorMaterial>>,
}

impl Renderer {
    pub fn new(three_d: &three_d::Context) -> Self {
        let terrain_mesh = CpuMesh::square();
        let mut terrain_material = PhysicalMaterial::new_opaque(
            three_d,
            &CpuMaterial {
                roughness: 1.0,
                metallic: 0.0,
                albedo: Color::new_opaque(45, 30, 25),
                ..Default::default()
            },
        )
        .unwrap();
        terrain_material.render_states.cull = Cull::Back;
        let terrain_model =
            Model::new_with_material(three_d, &terrain_mesh, terrain_material.clone()).unwrap();
        let water_model = build_water_plane(three_d);
        Self {
            terrain_mesh,
            terrain_model,
            terrain_material,
            water_model,
            wireframe: None,
            sky: build_sky(three_d),
            directional: DirectionalLight::new(
                three_d,
                1.5,
                Color::new_opaque(255, 222, 180),
                &vec3(-0.5, 0.5, -0.5).normalize(),
            )
            .unwrap(),
            ambient: AmbientLight::new(three_d, 0.5, Color::WHITE).unwrap(),
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
                self.terrain_mesh.indices = Some(Indices::U32(mesh_data.indices.clone()));
                self.terrain_mesh.normals = Some(mesh_data.normals.clone());
                self.terrain_mesh.uvs = Some(mesh_data.uv.clone());
                self.terrain_mesh.tangents = None;
            }
            self.wireframe = Some(build_wireframe(three_d, mesh_data, 16));
            self.terrain_model = Model::new_with_material(
                three_d,
                &self.terrain_mesh,
                self.terrain_material.clone(),
            )
            .unwrap();
        }
    }
    pub fn render(
        &mut self,
        three_d: &three_d::Context,
        info: &egui::PaintCallbackInfo,
        conf: Panel3dViewConf,
    ) {
        // Set where to paint
        let viewport = info.viewport_in_pixels();
        let viewport = Viewport {
            x: viewport.left_px.round() as _,
            y: viewport.from_bottom_px.round() as _,
            width: viewport.width_px.round() as _,
            height: viewport.height_px.round() as _,
        };

        let target = vec3(0.0, 0.0, 0.0);
        let campos = vec3(XY_SCALE * 2.0, 0.0, 0.0);

        let mut camera = Camera::new_perspective(
            three_d,
            viewport,
            campos,
            target,
            vec3(0.0, 0.0, 1.0),
            degrees((90.0 - conf.zoom * 0.8).clamp(1.0, 90.0)),
            0.1,
            XY_SCALE * 10.0,
        )
        .unwrap();

        camera
            .rotate_around_with_fixed_up(
                &target,
                conf.orbit[0] * XY_SCALE * 2.0,
                conf.orbit[1] * XY_SCALE * 2.0,
            )
            .unwrap();

        camera
            .translate(&(conf.pan[1] * camera.up() - conf.pan[0] * camera.right_direction()))
            .unwrap();
        let camz = camera.position().z;
        if camz < conf.water_level + 10.0 {
            camera
                .translate(&vec3(0.0, 0.0, conf.water_level + 10.0 - camz))
                .unwrap();
        }

        let mut transfo = Mat4::from_angle_z(radians(0.0));
        transfo.z[2] = conf.hscale / 100.0;

        self.terrain_model.set_transformation(transfo);
        self.directional
            .generate_shadow_map(1024, &[&self.terrain_model])
            .unwrap();
        self.terrain_model
            .render(&camera, &[&self.ambient, &self.directional])
            .unwrap();

        if conf.show_grid {
            if let Some(ref mut wireframe) = self.wireframe {
                wireframe.set_transformation(transfo);
                wireframe
                    .render(&camera, &[&self.ambient, &self.directional])
                    .unwrap();
            }
        }
        if conf.show_water {
            let mut water_transfo = Mat4::from_translation(Vec3::new(0.0, 0.0, conf.water_level));
            water_transfo.x[0] = XY_SCALE * 10.0;
            water_transfo.y[1] = XY_SCALE * 10.0;
            self.water_model.set_transformation(transfo * water_transfo);

            self.water_model
                .render(&camera, &[&self.ambient, &self.directional])
                .unwrap();
        }
        if conf.show_skybox {
            self.sky.render(&camera, &[]).unwrap();
        }
    }
}

fn build_wireframe(
    three_d: &three_d::Context,
    mesh_data: &MeshData,
    grid_size: usize,
) -> Gm<InstancedMesh, ColorMaterial> {
    let mut wireframe_material = ColorMaterial::new(
        three_d,
        &CpuMaterial {
            albedo: Color::new_opaque(220, 50, 50),
            roughness: 0.7,
            metallic: 0.8,
            ..Default::default()
        },
    )
    .unwrap();
    wireframe_material.render_states.cull = Cull::Back;
    let mut cylinder = CpuMesh::cylinder(10);
    cylinder
        .transform(&Mat4::from_nonuniform_scale(1.0, 0.1, 0.1))
        .unwrap();
    let edges = InstancedModel::new_with_material(
        three_d,
        &edge_transformations(grid_size, mesh_data),
        &cylinder,
        wireframe_material,
    )
    .unwrap();
    edges
}
fn edge_transformations(grid_size: usize, mesh_data: &MeshData) -> Vec<Instance> {
    let mut edge_transformations = std::collections::HashMap::new();
    let mut real_size_x = mesh_data.size.0;
    while real_size_x > grid_size {
        real_size_x /= 2;
    }
    real_size_x = mesh_data.size.0 / real_size_x;
    let mut real_size_y = mesh_data.size.1;
    while real_size_y > grid_size {
        real_size_y /= 2;
    }
    real_size_y = mesh_data.size.1 / real_size_y;
    for lx in (0..mesh_data.size.0).step_by(real_size_x) {
        let mut p1 = mesh_data.vertices[lx];
        for ly in 1..mesh_data.size.1 {
            let p2 = mesh_data.vertices[lx + ly * mesh_data.size.0];
            let scale = Mat4::from_nonuniform_scale((p1 - p2).magnitude(), 1.0, 1.0);
            let rotation =
                rotation_matrix_from_dir_to_dir(vec3(1.0, 0.0, 0.0), (p2 - p1).normalize());
            let translation = Mat4::from_translation(p1);
            edge_transformations.insert((lx, ly), translation * rotation * scale);
            p1 = p2;
        }
    }
    for ly in (0..mesh_data.size.1).step_by(real_size_y) {
        let mut p1 = mesh_data.vertices[ly * mesh_data.size.0];
        for lx in 1..mesh_data.size.0 {
            let p2 = mesh_data.vertices[lx + ly * mesh_data.size.0];
            let scale = Mat4::from_nonuniform_scale((p1 - p2).magnitude(), 1.0, 1.0);
            let rotation =
                rotation_matrix_from_dir_to_dir(vec3(1.0, 0.0, 0.0), (p2 - p1).normalize());
            let translation = Mat4::from_translation(p1);
            edge_transformations.insert((lx, ly), translation * rotation * scale);
            p1 = p2;
        }
    }
    edge_transformations
        .drain()
        .map(|(_, v)| Instance {
            geometry_transform: v,
            ..Default::default()
        })
        .collect::<Vec<_>>()
}

const SKY_BYTES: &[u8] = include_bytes!("../sky.jpg");

fn build_sky(three_d: &three_d::Context) -> Model<PhysicalMaterial> {
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
    sky2.transform(&Mat4::from_nonuniform_scale(
        ZSCALE * 5.0,
        XY_SCALE * 2.0,
        XY_SCALE * 2.0,
    ))
    .unwrap();
    sky2.transform(&Mat4::from_angle_y(degrees(-90.0))).unwrap();
    sky2.transform(&Mat4::from_angle_z(degrees(90.0))).unwrap();
    let mut uvs = Vec::new();
    for i in 0..SUBDIV + 1 {
        let u = i as f32 / SUBDIV as f32;
        uvs.push(vec2(u, 1.0));
    }
    for i in 0..SUBDIV + 1 {
        let u = i as f32 / SUBDIV as f32;
        uvs.push(vec2(u, 0.0));
    }
    sky2.uvs = Some(uvs);
    let mut sky_material = PhysicalMaterial::new_opaque(
        three_d,
        &CpuMaterial {
            roughness: 1.0,
            metallic: 0.0,
            emissive: Color::WHITE,
            emissive_texture: Some(CpuTexture {
                width: img.width(),
                height: img.height(),
                data: TextureData::RgbU8(data),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    // water_material.render_states.depth_test = DepthTest::Greater;
    sky_material.render_states.cull = Cull::Front;
    Model::new_with_material(three_d, &sky2, sky_material).unwrap()
}
fn build_water_plane(three_d: &three_d::Context) -> Model<PhysicalMaterial> {
    let water_mesh = CpuMesh::square();

    let mut water_material = PhysicalMaterial::new_opaque(
        three_d,
        &CpuMaterial {
            roughness: 0.1,
            metallic: 0.2,
            albedo: Color::new_opaque(50, 60, 150),
            ..Default::default()
        },
    )
    .unwrap();
    // water_material.render_states.depth_test = DepthTest::Greater;
    water_material.render_states.cull = Cull::Back;
    Model::new_with_material(three_d, &water_mesh, water_material).unwrap()
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
    let mut mesh = CpuMesh {
        name: "cylinder".to_string(),
        positions: Positions::F32(positions),
        indices: Some(Indices::U16(indices)),
        ..Default::default()
    };
    mesh.compute_normals();
    mesh
}
