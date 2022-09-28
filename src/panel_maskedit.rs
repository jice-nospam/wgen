use std::{rc::Rc, sync::Arc};

use eframe::egui::{self, PointerButton};
use three_d::{
    core::Color, vec3, Camera, ColorMaterial, CpuMaterial, CpuMesh, Cull, DepthTest, Indices,
    Model, Object, Positions, Viewport,
};

pub enum PanelMaskEditAction {}
pub struct PanelMaskEdit {
    image_size: usize,
    preview_size: usize,
    mask: Option<Vec<f32>>,
    brush_value: f32,
    brush_size: f32,
    brush_falloff: f32,
    mesh_updated: bool,
}

impl PanelMaskEdit {
    pub fn new(image_size: usize, preview_size: u32) -> Self {
        PanelMaskEdit {
            image_size,
            preview_size: preview_size as usize,
            mask: None,
            brush_value: 1.0,
            brush_size: 8.0,
            brush_falloff: 0.0,
            mesh_updated: false,
        }
    }
    pub fn display_mask(&mut self, image_size: usize, preview_size: u32, mask: Option<Vec<f32>>) {
        self.image_size = image_size;
        self.preview_size = preview_size as usize;
        self.mesh_updated = true;
        self.mask = mask.or_else(|| Some(vec![1.0; (preview_size * preview_size) as usize]));
    }
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<PanelMaskEditAction> {
        ui.vertical(|ui| {
            // if let Some(img) = &self.img {
            //     img.show(ui);
            // }
            egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                self.render_3dview(ui);
            });
            ui.label("Use left and right mouse buttons to edit the mask");
        });
        None
    }
    fn render_3dview(&mut self, ui: &mut egui::Ui) {
        let (rect, _response) = ui.allocate_exact_size(
            egui::Vec2::splat(self.image_size as f32),
            egui::Sense::drag(),
        );
        let _lbutton = ui.input().pointer.button_down(PointerButton::Primary);
        let _rbutton = ui.input().pointer.button_down(PointerButton::Secondary);
        let _mbutton = ui.input().pointer.button_down(PointerButton::Middle);
        let mesh_updated = self.mesh_updated;
        let mask = if mesh_updated {
            self.mask.clone()
        } else {
            None
        };
        let mask_size = self.preview_size;
        // TODO handle mouse clicks
        let callback = egui::PaintCallback {
            rect,
            callback: std::sync::Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                with_three_d_context(painter.gl(), |three_d, renderer| {
                    if mesh_updated {
                        renderer.update_model(three_d, mask_size, &mask);
                    }
                    renderer.render(three_d, &info);
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
        gl.disable(glow::DEPTH_TEST);
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
    model: Option<Model<ColorMaterial>>,
}

impl Renderer {
    pub fn new(three_d: &three_d::Context) -> Self {
        Self { model: None }
    }
    pub fn update_model(
        &mut self,
        three_d: &three_d::Context,
        mask_size: usize,
        mask: &Option<Vec<f32>>,
    ) {
        if let Some(mask) = mask {
            // TODO only update colors if the model is already created
            let mut vertices = Vec::with_capacity(mask_size * mask_size);
            let mut indices = Vec::with_capacity(6 * (mask_size - 1) * (mask_size - 1));
            let mut colors = Vec::with_capacity(mask_size * mask_size);
            for y in 0..mask_size {
                let vy = y as f32 / (mask_size - 1) as f32 * 10.0 - 5.0;
                let yoff = y * mask_size;
                for x in 0..mask_size {
                    let vx = x as f32 / (mask_size - 1) as f32 * 10.0 - 5.0;
                    let rgb_val = (mask[yoff + x] * 255.0).clamp(0.0, 255.0) as u8;
                    vertices.push(three_d::vec3(vx, vy, 0.0));
                    colors.push(Color::new_opaque(rgb_val, rgb_val, rgb_val));
                }
            }
            for y in 0..mask_size - 1 {
                let y_offset = y * mask_size;
                for x in 0..mask_size - 1 {
                    let off = x + y_offset;
                    indices.push((off) as u32);
                    indices.push((off + mask_size) as u32);
                    indices.push((off + 1) as u32);
                    indices.push((off + mask_size) as u32);
                    indices.push((off + mask_size + 1) as u32);
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
            self.model = Some(Model::new_with_material(three_d, &cpu_mesh, material).unwrap());
        }
    }
    pub fn render(&mut self, three_d: &three_d::Context, info: &egui::PaintCallbackInfo) {
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

        if let Some(ref mut model) = self.model {
            model.render(&camera, &[]).unwrap();
        }
    }
}
