use egui::PointerButton;

const PANEL3D_SIZE: f32 = 256.0;

#[derive(Clone, Copy)]
pub struct Panel3dViewConf {
    /// camera x and y orbit angles
    pub orbit: [f32; 2],
    /// camera x and y pan distances
    pub pan: [f32; 2],
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

/// the "3d preview" widgets and the square the scene camera renders into; the camera itself
/// lives in `preview3d`
pub struct Panel3dView {
    size: f32,
    conf: Panel3dViewConf,
}

impl Default for Panel3dView {
    fn default() -> Self {
        Self {
            size: PANEL3D_SIZE,
            conf: Panel3dViewConf {
                pan: [0.0, 0.0],
                orbit: [std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_4],
                zoom: 60.0,
                hscale: 100.0,
                water_level: 40.0,
                show_water: true,
                show_skybox: true,
            },
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
    /// canvas size in pixels
    pub fn set_size(&mut self, size: f32) {
        self.size = size;
    }
    /// the camera and scene settings; nothing renders them until the 3D scene exists
    #[allow(dead_code)]
    pub fn conf(&self) -> Panel3dViewConf {
        self.conf
    }
    /// draws the widgets and returns the rect of the 3D square, in egui points
    pub fn render(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        ui.vertical(|ui| {
            // transparent so the scene camera's output shows through the square
            let rect = egui::Frame::dark_canvas(ui.style())
                .fill(egui::Color32::TRANSPARENT)
                .show(ui, |ui| self.render_3dview(ui))
                .inner;
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
                ui.checkbox(&mut self.conf.show_water, "");
                ui.label("Water height");
                ui.add_enabled(
                    self.conf.show_water,
                    egui::DragValue::new(&mut self.conf.water_level)
                        .speed(0.1)
                        .range(std::ops::RangeInclusive::new(0.0, 100.0)),
                );
                ui.label("Show skybox");
                ui.checkbox(&mut self.conf.show_skybox, "");
            });
            rect
        })
        .inner
    }

    /// allocates the square and turns drags into orbit (left), pan (right) and zoom (middle)
    fn render_3dview(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        let (rect, response) =
            ui.allocate_exact_size(egui::Vec2::splat(self.size), egui::Sense::drag());
        let lbutton = ui
            .ctx()
            .input(|i| i.pointer.button_down(PointerButton::Primary));
        let rbutton = ui
            .ctx()
            .input(|i| i.pointer.button_down(PointerButton::Secondary));
        let mbutton = ui
            .ctx()
            .input(|i| i.pointer.button_down(PointerButton::Middle));
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
        rect
    }
}
