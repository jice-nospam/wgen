use std::{rc::Rc, sync::Arc};

use eframe::{
    egui::{self, PointerButton},
    emath,
};
use epaint::{Pos2, Rect};
use three_d::{
    core::Color, vec3, Blend, Camera, ColorMaterial, CpuMaterial, CpuMesh, Cull, DepthTest,
    Indices, Mat4, Model, Object, Positions, Viewport,
};

use crate::MASK_SIZE;

pub enum PanelMaskEditAction {}

/// maximum size of the brush relative to the canvas
const MAX_BRUSH_SIZE: f32 = 0.25;

#[derive(Clone, Copy)]
pub struct BrushConfig {
    pub brush_value: f32,
    pub brush_size: f32,
    pub brush_falloff: f32,
}
pub struct PanelMaskEdit {
    image_size: usize,
    mask: Option<Vec<f32>>,
    conf: BrushConfig,
    mesh_updated: bool,
    brush_updated: bool,
    prev_frame_time: f64,
}

impl PanelMaskEdit {
    pub fn new(image_size: usize) -> Self {
        PanelMaskEdit {
            image_size,
            mask: None,
            conf: BrushConfig {
                brush_value: 0.5,
                brush_size: 0.5,
                brush_falloff: 0.5,
            },
            mesh_updated: false,
            brush_updated: false,
            prev_frame_time: -1.0,
        }
    }
    pub fn get_mask(&self) -> Option<Vec<f32>> {
        self.mask.clone()
    }
    pub fn display_mask(&mut self, image_size: usize, mask: Option<Vec<f32>>) {
        self.image_size = image_size;
        self.mesh_updated = true;
        self.mask = mask.or_else(|| Some(vec![1.0; MASK_SIZE * MASK_SIZE]));
    }
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<PanelMaskEditAction> {
        ui.vertical(|ui| {
            egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                self.render_3dview(ui);
            });
            ui.horizontal(|ui| {
                ui.label("mouse buttons : left increase, right decrease, middle set brush value");
                if ui.input().pointer.button_down(PointerButton::Primary)
                    || ui.input().pointer.button_down(PointerButton::Secondary)
                    || ui.input().pointer.button_down(PointerButton::Middle)
                {
                    ui.add(egui::Spinner::new());
                } else {
                    self.prev_frame_time = -1.0;
                }
            });
            ui.horizontal(|ui| {
                ui.label("brush size");
                ui.add(
                    egui::DragValue::new(&mut self.conf.brush_size)
                        .speed(0.01)
                        .clamp_range(1.0 / (MASK_SIZE as f32)..=1.0),
                );
                ui.label("falloff");
                let old_falloff = self.conf.brush_falloff;
                ui.add(
                    egui::DragValue::new(&mut self.conf.brush_falloff)
                        .speed(0.01)
                        .clamp_range(0.0..=1.0),
                );
                ui.label("value");
                ui.add(
                    egui::DragValue::new(&mut self.conf.brush_value)
                        .speed(0.01)
                        .clamp_range(0.0..=1.0),
                );
                self.brush_updated = old_falloff != self.conf.brush_falloff;
            });
        });
        None
    }
    fn render_3dview(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(
            egui::Vec2::splat(self.image_size as f32),
            egui::Sense::drag(),
        );
        let lbutton = ui.input().pointer.button_down(PointerButton::Primary);
        let rbutton = ui.input().pointer.button_down(PointerButton::Secondary);
        let mbutton = ui.input().pointer.button_down(PointerButton::Middle);
        let mut mouse_pos = ui.input().pointer.hover_pos();
        let to_screen = emath::RectTransform::from_to(
            Rect::from_min_size(Pos2::ZERO, response.rect.square_proportions()),
            response.rect,
        );
        let from_screen = to_screen.inverse();
        let mut mesh_updated = self.mesh_updated;
        let brush_updated = self.brush_updated;
        let brush_config = self.conf;
        let time = if self.prev_frame_time == -1.0 {
            self.prev_frame_time = ui.input().time;
            0.0
        } else {
            let t = ui.input().time;
            let elapsed = t - self.prev_frame_time;
            self.prev_frame_time = t;
            elapsed
        };
        if let Some(pos) = mouse_pos {
            // mouse position in canvas from 0.0,0.0 (bottom left) to 1.0,1.0 (top right)
            let canvas_pos = from_screen * pos;
            mouse_pos = Some(canvas_pos);
            if (lbutton || rbutton || mbutton) && in_canvas(canvas_pos) && time > 0.0 {
                self.update_mask(canvas_pos, lbutton, rbutton, brush_config, time as f32);
                mesh_updated = true;
            }
        }
        let mask = if mesh_updated {
            self.mask.clone()
        } else {
            None
        };
        let callback = egui::PaintCallback {
            rect,
            callback: std::sync::Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                with_three_d_context(painter.gl(), |three_d, renderer| {
                    if brush_updated {
                        renderer.update_brush(three_d, brush_config);
                    }
                    if mesh_updated {
                        renderer.update_model(three_d, &mask);
                    }
                    renderer.render(three_d, &info, mouse_pos, brush_config);
                });
            })),
        };
        ui.painter().add(callback);
        self.mesh_updated = false;
    }

    fn update_mask(
        &mut self,
        canvas_pos: Pos2,
        lbutton: bool,
        rbutton: bool,
        brush_config: BrushConfig,
        time: f32,
    ) {
        if let Some(ref mut mask) = self.mask {
            let mx = canvas_pos.x * MASK_SIZE as f32;
            let my = canvas_pos.y * MASK_SIZE as f32;
            let brush_radius = brush_config.brush_size * MASK_SIZE as f32 * MAX_BRUSH_SIZE;
            let falloff_dist = (1.0 - brush_config.brush_falloff) * brush_radius;
            let minx = (mx - brush_radius).max(0.0) as usize;
            let maxx = ((mx + brush_radius) as usize).min(MASK_SIZE);
            let miny = (my - brush_radius).max(0.0) as usize;
            let maxy = ((my + brush_radius) as usize).min(MASK_SIZE);
            let (target_value, time_coef) = if lbutton {
                (0.0, 10.0)
            } else if rbutton {
                // for some unknown reason, white color is faster than black!
                (1.0, 3.0)
            } else {
                // mbutton
                (brush_config.brush_value, 5.0)
            };
            let brush_coef = 1.0 / (brush_radius - falloff_dist);
            for y in miny..maxy {
                let dy = y as f32 - my;
                let yoff = y * MASK_SIZE;
                for x in minx..maxx {
                    let dx = x as f32 - mx;
                    // distance from brush center
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist >= brush_radius {
                        // out of the brush
                        continue;
                    }
                    let alpha = if dist < falloff_dist {
                        1.0
                    } else {
                        1.0 - (dist - falloff_dist) * brush_coef
                    };
                    let current_value = mask[x + yoff];
                    mask[x + yoff] =
                        current_value + time * time_coef * alpha * (target_value - current_value);
                }
            }
        }
    }
}

fn in_canvas(canvas_pos: Pos2) -> bool {
    canvas_pos.x >= 0.0 && canvas_pos.x <= 1.0 && canvas_pos.y >= 0.0 && canvas_pos.y <= 1.0
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
        gl.disable(glow::DEPTH_TEST);
        gl.enable(glow::BLEND);
        if !cfg!(target_arch = "wasm32") {
            gl.disable(glow::FRAMEBUFFER_SRGB);
        }
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
    mask_model: Option<Model<ColorMaterial>>,
    brush_model: Model<ColorMaterial>,
}

impl Renderer {
    pub fn new(three_d: &three_d::Context) -> Self {
        Self {
            mask_model: None,
            brush_model: build_brush(three_d, 0.5),
        }
    }
    pub fn update_brush(&mut self, three_d: &three_d::Context, brush_conf: BrushConfig) {
        // TODO only move inner ring vertices instead of recreating everything
        self.brush_model = build_brush(three_d, brush_conf.brush_falloff);
    }
    pub fn update_model(&mut self, three_d: &three_d::Context, mask: &Option<Vec<f32>>) {
        if let Some(mask) = mask {
            // TODO only update colors if the model is already created
            let mut vertices = Vec::with_capacity(MASK_SIZE * MASK_SIZE);
            let mut indices = Vec::with_capacity(6 * (MASK_SIZE - 1) * (MASK_SIZE - 1));
            let mut colors = Vec::with_capacity(MASK_SIZE * MASK_SIZE);
            for y in 0..MASK_SIZE {
                let vy = y as f32 / (MASK_SIZE - 1) as f32 * 10.0 - 5.0;
                let yoff = (MASK_SIZE - 1 - y) * MASK_SIZE;
                for x in 0..MASK_SIZE {
                    let vx = x as f32 / (MASK_SIZE - 1) as f32 * 10.0 - 5.0;
                    let rgb_val = (mask[yoff + x] * 255.0).clamp(0.0, 255.0) as u8;
                    vertices.push(three_d::vec3(vx, vy, 0.0));
                    colors.push(Color::new_opaque(rgb_val, rgb_val, rgb_val));
                }
            }
            for y in 0..MASK_SIZE - 1 {
                let y_offset = y * MASK_SIZE;
                for x in 0..MASK_SIZE - 1 {
                    let off = x + y_offset;
                    indices.push((off) as u32);
                    indices.push((off + MASK_SIZE) as u32);
                    indices.push((off + 1) as u32);
                    indices.push((off + MASK_SIZE) as u32);
                    indices.push((off + MASK_SIZE + 1) as u32);
                    indices.push((off + 1) as u32);
                }
            }
            let cpu_mesh = CpuMesh {
                positions: Positions::F32(vertices),
                indices: Some(Indices::U32(indices)),
                colors: Some(colors),
                ..Default::default()
            };

            let mut material = ColorMaterial::new(
                three_d,
                &CpuMaterial {
                    roughness: 1.0,
                    metallic: 0.0,
                    albedo: Color::WHITE,
                    ..Default::default()
                },
            )
            .unwrap();

            material.render_states.cull = Cull::None;
            material.render_states.depth_test = DepthTest::Always;
            self.mask_model = Some(Model::new_with_material(three_d, &cpu_mesh, material).unwrap());
        }
    }
    pub fn render(
        &mut self,
        three_d: &three_d::Context,
        info: &egui::PaintCallbackInfo,
        mouse_pos: Option<Pos2>,
        brush_conf: BrushConfig,
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
        let campos = vec3(0.0, 0.0, 1.0);

        let camera = Camera::new_orthographic(
            three_d,
            viewport,
            campos,
            target,
            vec3(0.0, 1.0, 0.0),
            10.0,
            0.0,
            1000.0,
        )
        .unwrap();

        if let Some(ref mut model) = self.mask_model {
            model.render(&camera, &[]).unwrap();
        }
        if let Some(mouse_pos) = mouse_pos {
            let transfo = Mat4::from_translation(vec3(
                mouse_pos.x * 10.0 - 5.0,
                5.0 - mouse_pos.y * 10.0,
                0.1,
            ));
            let scale = Mat4::from_scale(brush_conf.brush_size * 10.0 * MAX_BRUSH_SIZE);
            self.brush_model.set_transformation(transfo * scale);
            self.brush_model.render(&camera, &[]).unwrap();
        }
    }
}

/// build a circular mesh with a double ring : one opaque 32 vertices inner ring and one transparent 64 vertices outer ring
fn build_brush(three_d: &three_d::Context, falloff: f32) -> Model<ColorMaterial> {
    const VERTICES_COUNT: usize = 1 + 32 + 64;
    let mut colors = Vec::with_capacity(VERTICES_COUNT);
    let mut vertices = Vec::with_capacity(VERTICES_COUNT);
    let mut indices = Vec::with_capacity(3 * 32 + 9 * 32);
    vertices.push(vec3(0.0, 0.0, 0.0));
    let inv_fall = 1.0 - falloff;
    // inner opaque ring
    for i in 0..32 {
        let angle = std::f32::consts::PI * 2.0 * (i as f32) / 32.0;
        vertices.push(vec3(angle.cos() * inv_fall, angle.sin() * inv_fall, 0.0));
    }
    // outer transparent ring
    for i in 0..64 {
        let angle = std::f32::consts::PI * 2.0 * (i as f32) / 64.0;
        vertices.push(vec3(angle.cos(), angle.sin(), 0.0));
    }
    for _ in 0..33 {
        colors.push(Color::RED);
    }
    for _ in 0..64 {
        colors.push(Color::new(255, 0, 0, 0));
    }
    // inner ring
    for i in 0..32 {
        indices.push(0);
        indices.push(1 + i);
        indices.push(1 + (1 + i) % 32);
    }
    // outer ring, 32 vertices inside, 64 vertices outside
    for i in 0..32 {
        indices.push(1 + i);
        indices.push(33 + 2 * i);
        indices.push(33 + (2 * i + 1) % 64);

        indices.push(1 + i);
        indices.push(1 + (i + 1) % 32);
        indices.push(33 + (2 * i + 1) % 64);

        indices.push(1 + (i + 1) % 32);
        indices.push(33 + (2 * i + 1) % 64);
        indices.push(33 + (2 * i + 2) % 64);
    }
    let cpu_mesh = CpuMesh {
        name: "brush".to_string(),
        positions: Positions::F32(vertices),
        indices: Some(Indices::U16(indices)),
        colors: Some(colors),
        ..Default::default()
    };

    let mut material = ColorMaterial::new(
        three_d,
        &CpuMaterial {
            roughness: 1.0,
            metallic: 0.0,
            albedo: Color::WHITE,
            ..Default::default()
        },
    )
    .unwrap();

    material.render_states.cull = Cull::None;
    material.render_states.depth_test = DepthTest::Always;
    material.render_states.blend = Blend::TRANSPARENCY;
    Model::new_with_material(three_d, &cpu_mesh, material).unwrap()
}
