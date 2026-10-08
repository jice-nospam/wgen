use egui::PointerButton;
use serde::{Deserialize, Serialize};

const PANEL3D_SIZE: f32 = 256.0;
/// zoom at which the vertical fov `90 - zoom * 0.8` reaches its 1 degree minimum
const ZOOM_MAX: f32 = 111.25;
/// zoom units per scroll point (one wheel notch is 40 points)
const WHEEL_ZOOM_SPEED: f32 = 0.1;

/// saved in the `.wgen` file; fields a file lacks take their default
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Panel3dViewConf {
    /// camera x and y orbit angles
    pub orbit: [f32; 2],
    /// camera x and y pan distances
    pub pan: [f32; 2],
    /// camera zoom in degrees (y field of view is 90 - zoom)
    pub zoom: f32,
    /// vertical scale to apply to the heightmap
    pub hscale: f32,
    /// water plane height in scene units (`water_level × ZSCALE`), written by `MyApp` from the
    /// project's water level; not saved
    #[serde(skip)]
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
            orbit: [std::f32::consts::FRAC_PI_4, std::f32::consts::FRAC_PI_4],
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
    /// restores the settings loaded from a project; `water_level` is left to `MyApp`
    pub fn set_conf(&mut self, conf: Panel3dViewConf) {
        self.conf = Panel3dViewConf {
            water_level: self.conf.water_level,
            ..conf
        };
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
    /// (left), pan (right) and zoom (middle drag, wheel)
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
            self.conf.zoom = zoom_by(self.conf.zoom, response.drag_delta().y * 0.15);
        }
        if response.hovered() {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y != 0.0 {
                self.conf.zoom = zoom_by(self.conf.zoom, scroll_y * WHEEL_ZOOM_SPEED);
            }
        }
        rect
    }
}

/// adds `delta` to `zoom`, kept within the range where the fov formula is not clamped
fn zoom_by(zoom: f32, delta: f32) -> f32 {
    (zoom + delta).clamp(0.0, ZOOM_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_by_adds_delta() {
        assert_eq!(zoom_by(60.0, 4.0), 64.0);
    }

    #[test]
    fn zoom_by_clamps_to_range() {
        assert_eq!(zoom_by(60.0, -1000.0), 0.0);
        assert_eq!(zoom_by(60.0, 1000.0), ZOOM_MAX);
    }

    #[test]
    fn zoom_max_hits_min_fov() {
        assert_eq!(90.0 - ZOOM_MAX * 0.8, 1.0);
    }
}
