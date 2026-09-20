use egui::PointerButton;

const PANEL3D_SIZE: f32 = 256.0;

#[derive(Clone, Copy, PartialEq)]
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
    /// sun elevation above the horizon, in degrees; the azimuth is fixed
    pub sun_elevation: f32,
    /// camera exposure in EV100
    pub exposure: f32,
}

/// the "3d preview" widgets and the square the scene camera renders into; the camera itself
/// lives in `preview3d`
pub struct Panel3dView {
    size: f32,
    conf: Panel3dViewConf,
}

impl Default for Panel3dViewConf {
    fn default() -> Self {
        Self {
            pan: [0.0, 0.0],
            orbit: [std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_4],
            zoom: 60.0,
            hscale: 100.0,
            water_level: 40.0,
            show_water: true,
            show_skybox: true,
            sun_elevation: 35.0,
            exposure: 13.0,
        }
    }
}

impl Default for Panel3dView {
    fn default() -> Self {
        Self {
            size: PANEL3D_SIZE,
            conf: Panel3dViewConf::default(),
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
    /// the camera and scene settings, applied to the scene by `preview3d::apply_view_conf`
    pub fn conf(&self) -> Panel3dViewConf {
        self.conf
    }
    /// draws the widgets, paints the scene image `texture` in the 3D square and returns the
    /// square's rect, in egui points
    pub fn render(&mut self, ui: &mut egui::Ui, texture: egui::TextureId) -> egui::Rect {
        ui.vertical(|ui| {
            let rect = egui::Frame::dark_canvas(ui.style())
                .show(ui, |ui| self.render_3dview(ui, texture))
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
            ui.horizontal(|ui| {
                ui.label("Sun elevation °");
                ui.add(
                    egui::DragValue::new(&mut self.conf.sun_elevation)
                        .speed(1.0)
                        .range(std::ops::RangeInclusive::new(2.0, 89.0)),
                );
                ui.label("Exposure EV");
                ui.add(
                    egui::DragValue::new(&mut self.conf.exposure)
                        .speed(0.1)
                        .range(std::ops::RangeInclusive::new(6.0, 20.0)),
                );
            });
            rect
        })
        .inner
    }

    /// allocates the square, paints the scene image over it and turns drags into orbit
    /// (left), pan (right) and zoom (middle)
    fn render_3dview(&mut self, ui: &mut egui::Ui, texture: egui::TextureId) -> egui::Rect {
        let (rect, response) =
            ui.allocate_exact_size(egui::Vec2::splat(self.size), egui::Sense::drag());
        ui.painter().image(
            texture,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
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
