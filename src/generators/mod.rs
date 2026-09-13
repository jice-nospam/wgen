#[cfg(test)]
mod calib;
mod fbm;
mod flow;
mod fluvial_erosion;
mod hills;
mod island;
mod landmass;
mod mid_point;
mod mudslide;
mod normalize;
mod resample;
mod thermal_erosion;
mod water_erosion;

use std::sync::mpsc::Sender;

pub use fbm::{gen_fbm, render_fbm, FbmConf};
pub use flow::{receiver_distance, FlowNet};
pub use fluvial_erosion::{gen_fluvial_erosion, render_fluvial_erosion, FluvialErosionConf};
pub use hills::{gen_hills, render_hills, HillsConf};
pub use island::{gen_island, render_island, IslandConf};
pub use landmass::{gen_landmass, render_landmass, LandMassConf};
pub use mid_point::{gen_mid_point, render_mid_point, MidPointConf};
pub use mudslide::{gen_mudslide, render_mudslide, MudSlideConf};
pub use normalize::{gen_normalize, NormalizeConf};
pub use resample::{add_upsampled, bilinear, downsample, work_size};
pub use thermal_erosion::{gen_thermal_erosion, render_thermal_erosion, ThermalErosionConf};
pub use water_erosion::{gen_water_erosion, render_water_erosion, WaterErosionConf};

use crate::ThreadMessage;

const DIRX: [i32; 9] = [0, -1, 0, 1, -1, 1, -1, 0, 1];
const DIRY: [i32; 9] = [0, -1, -1, -1, 0, 0, 1, 1, 1];

/// (min, max) of a map; (0, 0) for an empty one
pub fn get_min_max(v: &[f32]) -> (f32, f32) {
    let Some(&first) = v.first() else {
        return (0.0, 0.0);
    };
    let mut min = first;
    let mut max = first;
    for val in v.iter().skip(1) {
        if *val > max {
            max = *val;
        } else if *val < min {
            min = *val;
        }
    }
    (min, max)
}

pub fn normalize(v: &mut [f32], target_min: f32, target_max: f32) {
    let (min, max) = get_min_max(v);
    let invmax = if min == max {
        0.0
    } else {
        (target_max - target_min) / (max - min)
    };
    for val in v {
        *val = target_min + (*val - min) * invmax;
    }
}

/// progress reporting and cancellation for one step; one instance per step execution
pub struct Progress {
    tx: Sender<ThreadMessage>,
    /// selects ExporterStepProgress over GeneratorStepProgress
    export: bool,
    /// send only when the progress advanced by this much since the last message
    min_step: f32,
    last_sent: f32,
    /// staleness is asked at most once per 1 % of progress
    last_checked: f32,
    stale: Box<dyn Fn() -> bool + Send>,
    /// latched once `stale` returned true
    cancelled: bool,
}

impl Progress {
    fn new(
        tx: Sender<ThreadMessage>,
        export: bool,
        min_step: f32,
        stale: Box<dyn Fn() -> bool + Send>,
    ) -> Self {
        Self {
            tx,
            export,
            min_step,
            last_sent: 0.0,
            last_checked: -1.0,
            stale,
            cancelled: false,
        }
    }
    /// preview path: `stale` tells whether the step being run has been invalidated by a newer regen
    pub fn preview(
        tx: Sender<ThreadMessage>,
        min_step: f32,
        stale: impl Fn() -> bool + Send + 'static,
    ) -> Self {
        Self::new(tx, false, min_step, Box::new(stale))
    }
    /// export path: never cancelled
    pub fn export(tx: Sender<ThreadMessage>, min_step: f32) -> Self {
        Self::new(tx, true, min_step, Box::new(|| false))
    }
    /// tests: dropped receiver, never cancelled
    #[cfg(test)]
    pub fn headless() -> Self {
        let (tx, _) = std::sync::mpsc::channel();
        Self::new(tx, false, 1.0, Box::new(|| false))
    }
    /// `p` in 0..1 within this step. Returns false once the step is cancelled: the generator must return.
    /// A closed channel (main thread gone, or a headless test) is not an error.
    pub fn report(&mut self, p: f32) -> bool {
        if self.cancelled {
            return false;
        }
        if p - self.last_checked >= 0.01 {
            self.last_checked = p;
            if (self.stale)() {
                self.cancelled = true;
                return false;
            }
        }
        if p - self.last_sent >= self.min_step {
            self.last_sent = p;
            let _ = self.tx.send(if self.export {
                ThreadMessage::ExporterStepProgress(p)
            } else {
                ThreadMessage::GeneratorStepProgress(p)
            });
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_max_of_empty_map_is_zero() {
        assert_eq!(get_min_max(&[]), (0.0, 0.0));
        assert_eq!(get_min_max(&[2.0, -1.0, 0.5]), (-1.0, 2.0));
    }

    #[test]
    fn progress_throttles_and_stops_when_stale() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut progress = Progress::preview(tx, 0.5, || false);
        assert!(progress.report(0.2));
        assert!(progress.report(0.6));
        assert!(progress.report(1.0));
        match rx.try_recv() {
            Ok(ThreadMessage::GeneratorStepProgress(p)) => assert_eq!(p, 0.6),
            _ => panic!("expected one GeneratorStepProgress(0.6)"),
        }
        assert!(rx.try_recv().is_err(), "more than one message sent");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut stale = Progress::preview(tx, 0.5, || true);
        assert!(!stale.report(0.0));
        assert!(rx.try_recv().is_err(), "a cancelled step sent a message");
    }
}
