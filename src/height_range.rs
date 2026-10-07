//! The project's height window: which heights, in meters, map to 0..1 in the 2D view, to `0..ZSCALE` in
//! the 3D mesh and to 0..1 in exported files.
use serde::{Deserialize, Serialize};

use crate::generators::get_min_max;

/// meters per generator unit: generators work in units (about 0..1), every stored height is in
/// meters. A power of two, so the conversion is exact in f32
pub const GEN_UNIT_M: f32 = 4096.0;

/// a `DragValue` showing a height stored in generator units as meters
pub fn drag_meters(ui: &mut egui::Ui, units: &mut f32, speed_m: f32) -> egui::Response {
    let mut m = *units * GEN_UNIT_M;
    let response = ui.add(egui::DragValue::new(&mut m).speed(speed_m).suffix(" m"));
    if response.changed() {
        *units = m / GEN_UNIT_M;
    }
    response
}

/// `auto`: the window is the map's own min..max; otherwise `min..max`, heights outside clamped
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeightRange {
    pub auto: bool,
    pub min: f32,
    pub max: f32,
}

impl Default for HeightRange {
    fn default() -> Self {
        Self {
            auto: true,
            min: -200.0,
            max: 4096.0,
        }
    }
}

impl HeightRange {
    /// the raw heights mapped to 0 and 1 for the map `h`
    pub fn bounds(&self, h: &[f32]) -> (f32, f32) {
        if self.auto {
            get_min_max(h)
        } else {
            (self.min, self.max)
        }
    }

    /// `(lo, coef)` such that `(h - lo) * coef` is in 0..1; `coef` is 0 for an empty window
    pub fn unit(&self, h: &[f32]) -> (f32, f32) {
        let (lo, hi) = self.bounds(h);
        let coef = if hi - lo > f32::EPSILON {
            1.0 / (hi - lo)
        } else {
            0.0
        };
        (lo, coef)
    }

    /// a raw height in 0..1 through `unit`'s result, clamped
    pub fn to01(lo: f32, coef: f32, h: f32) -> f32 {
        ((h - lo) * coef).clamp(0.0, 1.0)
    }

    /// cells outside the manual window; 0 in auto mode
    pub fn count_outside(&self, h: &[f32]) -> usize {
        if self.auto {
            return 0;
        }
        h.iter().filter(|&&v| v < self.min || v > self.max).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANUAL: HeightRange = HeightRange {
        auto: false,
        min: 0.0,
        max: 1.0,
    };

    #[test]
    fn default_window_is_minus_200_to_4096() {
        let r = HeightRange::default();
        assert!(r.auto);
        assert_eq!((r.min, r.max), (-200.0, 4096.0));
    }

    #[test]
    fn auto_bounds_are_the_map_range() {
        assert_eq!(
            HeightRange::default().bounds(&[3.0, -1.0, 2.0]),
            (-1.0, 3.0)
        );
    }

    #[test]
    fn manual_bounds_ignore_the_map() {
        assert_eq!(MANUAL.bounds(&[3.0, -1.0, 2.0]), (0.0, 1.0));
        assert_eq!(MANUAL.count_outside(&[3.0, -1.0, 0.5]), 2);
    }

    #[test]
    fn flat_map_unit_coef_is_zero() {
        assert_eq!(HeightRange::default().unit(&[2.0, 2.0]), (2.0, 0.0));
    }

    #[test]
    fn to01_clamps() {
        let (lo, coef) = MANUAL.unit(&[]);
        assert_eq!(HeightRange::to01(lo, coef, -1.0), 0.0);
        assert_eq!(HeightRange::to01(lo, coef, 0.25), 0.25);
        assert_eq!(HeightRange::to01(lo, coef, 2.0), 1.0);
    }

    #[test]
    fn count_outside_is_zero_in_auto() {
        assert_eq!(HeightRange::default().count_outside(&[-5.0, 5.0]), 0);
    }
}
