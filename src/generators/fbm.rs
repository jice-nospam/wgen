use eframe::egui;
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use serde::{Deserialize, Serialize};

use super::Progress;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct FbmConf {
    pub mulx: f32,
    pub muly: f32,
    pub addx: f32,
    pub addy: f32,
    pub octaves: f32,
    pub delta: f32,
    pub scale: f32,
}

impl Default for FbmConf {
    fn default() -> Self {
        Self {
            mulx: 2.20,
            muly: 2.20,
            addx: 0.0,
            addy: 0.0,
            octaves: 6.0,
            delta: 0.0,
            scale: 2.05,
        }
    }
}

pub fn render_fbm(ui: &mut egui::Ui, conf: &mut FbmConf) {
    ui.horizontal(|ui| {
        ui.label("scale x").on_hover_text(
            "Horizontal zoom of the noise: higher packs more, smaller features across the map",
        );
        ui.add(
            egui::DragValue::new(&mut conf.mulx)
                .speed(0.1)
                .range(0.0..=100.0),
        );
        ui.label("y").on_hover_text(
            "Vertical zoom of the noise: higher packs more, smaller features across the map",
        );
        ui.add(
            egui::DragValue::new(&mut conf.muly)
                .speed(0.1)
                .range(0.0..=100.0),
        );
        ui.label("octaves")
            .on_hover_text("Layers of ever finer detail: more = richer but slower");
        ui.add(
            egui::DragValue::new(&mut conf.octaves)
                .speed(0.5)
                .range(1.0..=Fbm::<Perlin>::MAX_OCTAVES as f32),
        );
    });
    ui.horizontal(|ui| {
        ui.label("offset x")
            .on_hover_text("Slides the noise sideways to look at another part of it");
        ui.add(
            egui::DragValue::new(&mut conf.addx)
                .speed(0.1)
                .range(0.0..=200.0),
        );
        ui.label("y")
            .on_hover_text("Slides the noise up or down to look at another part of it");
        ui.add(
            egui::DragValue::new(&mut conf.addy)
                .speed(0.1)
                .range(0.0..=200.0),
        );
        ui.label("scale").on_hover_text("Height of the bumps");
        ui.add(
            egui::DragValue::new(&mut conf.scale)
                .speed(0.01)
                .range(0.01..=10.0),
        );
    });
}

pub fn gen_fbm(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &FbmConf,
    progress: &mut Progress,
) {
    let xcoef = conf.mulx / 400.0;
    let ycoef = conf.muly / 400.0;
    let num_threads = num_cpus::get();
    // only chunk 0 reports progress; on cancel it stops while the other chunks finish their rows
    let mut progress = Some(progress);
    std::thread::scope(|s| {
        // at least one row per job : a small preview on a many-core machine must not get a zero-sized chunk
        let size_per_job = (size.1 / num_threads).max(1);
        for (i, chunk) in hmap.chunks_mut(size_per_job * size.0).enumerate() {
            let fbm = Fbm::<Perlin>::new(seed as u32).set_octaves(conf.octaves as usize);
            let mut chunk_progress = if i == 0 { progress.take() } else { None };
            s.spawn(move || {
                let yoffset = i * size_per_job;
                let lasty = size_per_job.min(size.1 - yoffset);
                for y in 0..lasty {
                    let f1 = ((y + yoffset) as f32 * 512.0 / size.1 as f32 + conf.addy) * ycoef;
                    let mut offset = y * size.0;
                    for x in 0..size.0 {
                        let f0 = (x as f32 * 512.0 / size.0 as f32 + conf.addx) * xcoef;
                        let value =
                            conf.delta + fbm.get([f0 as f64, f1 as f64]) as f32 * conf.scale;
                        chunk[offset] += value;
                        offset += 1;
                    }
                    if let Some(progress) = chunk_progress.as_mut() {
                        if !progress.report((y + 1) as f32 / size_per_job as f32) {
                            break;
                        }
                    }
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fbm_is_deterministic_and_handles_tiny_maps() {
        let conf = FbmConf::default();
        let mut a = vec![0.0; 8 * 2];
        let mut b = vec![0.0; 8 * 2];
        gen_fbm(7, (8, 2), &mut a, &conf, &mut Progress::headless());
        gen_fbm(7, (8, 2), &mut b, &conf, &mut Progress::headless());
        assert_eq!(a, b);
        assert!(a.iter().all(|v| v.is_finite()));
    }
}
