use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::generators::{
    gen_fbm, gen_fluvial_erosion, gen_hills, gen_island, gen_landmass, gen_mid_point, gen_mudslide,
    gen_normalize, gen_plateau, gen_ridged, gen_thermal_erosion, gen_water_erosion, render_fbm,
    render_fluvial_erosion, render_hills, render_island, render_landmass, render_mid_point,
    render_mudslide, render_plateau, render_ridged, render_thermal_erosion, render_water_erosion,
    FbmConf, FluvialErosionConf, HillsConf, IslandConf, LandMassConf, MidPointConf, MudSlideConf,
    NormalizeConf, PlateauConf, Progress, RidgedConf, ThermalErosionConf, WaterErosionConf,
};
use crate::gpu::{
    fbm::gen_fbm_gpu, fluvial_erosion::gen_fluvial_erosion_gpu, plateau::gen_plateau_gpu,
    ridged::gen_ridged_gpu, thermal_erosion::gen_thermal_erosion_gpu, Backend,
};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
/// Each value contains its own configuration
pub enum StepType {
    Hills(HillsConf),
    Fbm(FbmConf),
    Normalize(NormalizeConf),
    LandMass(LandMassConf),
    MudSlide(MudSlideConf),
    ThermalErosion(ThermalErosionConf),
    WaterErosion(WaterErosionConf),
    FluvialErosion(FluvialErosionConf),
    Island(IslandConf),
    MidPoint(MidPointConf),
    Ridged(RidgedConf),
    Plateau(PlateauConf),
}

impl StepType {
    /// the generators the dropdown offers, with their default configuration, in dropdown
    /// order. `MudSlide` and `WaterErosion` are legacy: they only exist in old `.wgen` files
    /// and still load and run, but a new step cannot be one
    pub fn all() -> [StepType; 10] {
        [
            StepType::Hills(HillsConf::default()),
            StepType::Fbm(FbmConf::default()),
            StepType::MidPoint(MidPointConf::default()),
            StepType::Ridged(RidgedConf::default()),
            StepType::Normalize(NormalizeConf::default()),
            StepType::LandMass(LandMassConf::default()),
            StepType::Plateau(PlateauConf::default()),
            StepType::ThermalErosion(ThermalErosionConf::default()),
            StepType::FluvialErosion(FluvialErosionConf::default()),
            StepType::Island(IslandConf::default()),
        ]
    }
    /// the variant name: the step label, and what `Step`'s `Display` prints
    pub fn name(&self) -> &'static str {
        match self {
            StepType::Hills(_) => "Hills",
            StepType::Fbm(_) => "Fbm",
            StepType::MidPoint(_) => "MidPoint",
            StepType::Ridged(_) => "Ridged",
            StepType::Plateau(_) => "Plateau",
            StepType::Normalize(_) => "Normalize",
            StepType::LandMass(_) => "LandMass",
            StepType::MudSlide(_) => "MudSlide",
            StepType::ThermalErosion(_) => "ThermalErosion",
            StepType::WaterErosion(_) => "WaterErosion",
            StepType::FluvialErosion(_) => "FluvialErosion",
            StepType::Island(_) => "Island",
        }
    }
    /// the dropdown hover text
    pub fn description(&self) -> &'static str {
        match self {
            StepType::Hills(_) => "Add round hills to generate a smooth land",
            StepType::Fbm(_) => "Add fractional brownian motion to generate a mountainous land",
            StepType::MidPoint(_) => "Use mid point displacement to generate a mountainous land",
            StepType::Ridged(_) => {
                "Add ridged multifractal noise: sharp crests and alpine peaks, optionally folded"
            }
            StepType::Plateau(_) => {
                "Cut the terrain into flat stepped levels: mesas, buttes, tablelands"
            }
            StepType::Normalize(_) => "Scale the terrain back to the 0.0-1.0 range",
            StepType::LandMass(_) => {
                "Scale the terrain so that only a proportion of land is above water level"
            }
            StepType::MudSlide(_) => "Legacy: smooth the terrain (superseded by ThermalErosion)",
            StepType::ThermalErosion(_) => "Crumble slopes steeper than the talus into scree",
            StepType::WaterErosion(_) => {
                "Legacy: simulate rain drops carving rivers (superseded by FluvialErosion)"
            }
            StepType::FluvialErosion(_) => {
                "Carve a dendritic river network with the stream-power law (grid based, pairs with ThermalErosion)"
            }
            StepType::Island(_) => "Lower height on the map borders",
        }
    }
    /// the parameter widgets of this step (nothing for Normalize)
    pub fn render(&mut self, ui: &mut egui::Ui) {
        match self {
            StepType::Hills(conf) => render_hills(ui, conf),
            StepType::Fbm(conf) => render_fbm(ui, conf),
            StepType::MidPoint(conf) => render_mid_point(ui, conf),
            StepType::Ridged(conf) => render_ridged(ui, conf),
            StepType::Plateau(conf) => render_plateau(ui, conf),
            StepType::Normalize(_) => (),
            StepType::LandMass(conf) => render_landmass(ui, conf),
            StepType::MudSlide(conf) => render_mudslide(ui, conf),
            StepType::ThermalErosion(conf) => render_thermal_erosion(ui, conf),
            StepType::WaterErosion(conf) => render_water_erosion(ui, conf),
            StepType::FluvialErosion(conf) => render_fluvial_erosion(ui, conf),
            StepType::Island(conf) => render_island(ui, conf),
        }
    }
    /// runs the generator on `h`, which holds the previous step's output; a cancelled
    /// `progress` makes the generator return early with `h` in an unspecified state.
    /// A generator with a GPU twin runs it when `backend` offers a context, and falls back to
    /// the CPU generator when the twin fails before writing anything
    pub fn run(
        &self,
        seed: u64,
        size: (usize, usize),
        h: &mut [f32],
        progress: &mut Progress,
        backend: &Backend,
    ) {
        match self {
            StepType::Hills(conf) => gen_hills(seed, size, h, conf, progress),
            StepType::Fbm(conf) => match backend.gpu() {
                Some(gpu) => {
                    if let Err(e) = gen_fbm_gpu(gpu, seed, size, h, conf, progress) {
                        crate::log(&format!("gpu=>fbm fell back to the CPU: {}", e.0));
                        gen_fbm(seed, size, h, conf, progress)
                    }
                }
                None => gen_fbm(seed, size, h, conf, progress),
            },
            StepType::MidPoint(conf) => gen_mid_point(seed, size, h, conf, progress),
            StepType::Ridged(conf) => match backend.gpu() {
                Some(gpu) => {
                    if let Err(e) = gen_ridged_gpu(gpu, seed, size, h, conf, progress) {
                        crate::log(&format!("gpu=>ridged fell back to the CPU: {}", e.0));
                        gen_ridged(seed, size, h, conf, progress)
                    }
                }
                None => gen_ridged(seed, size, h, conf, progress),
            },
            StepType::Plateau(conf) => match backend.gpu() {
                Some(gpu) => {
                    if let Err(e) = gen_plateau_gpu(gpu, seed, size, h, conf, progress) {
                        crate::log(&format!("gpu=>plateau fell back to the CPU: {}", e.0));
                        gen_plateau(seed, size, h, conf, progress)
                    }
                }
                None => gen_plateau(seed, size, h, conf, progress),
            },
            StepType::Normalize(conf) => gen_normalize(h, conf),
            StepType::LandMass(conf) => gen_landmass(size, h, conf, progress),
            StepType::MudSlide(conf) => gen_mudslide(size, h, conf, progress),
            StepType::ThermalErosion(conf) => match backend.gpu() {
                Some(gpu) => {
                    if let Err(e) = gen_thermal_erosion_gpu(gpu, size, h, conf, progress) {
                        crate::log(&format!("gpu=>thermal fell back to the CPU: {}", e.0));
                        gen_thermal_erosion(size, h, conf, progress)
                    }
                }
                None => gen_thermal_erosion(size, h, conf, progress),
            },
            StepType::WaterErosion(conf) => gen_water_erosion(seed, size, h, conf, progress),
            StepType::FluvialErosion(conf) => match backend.gpu() {
                Some(gpu) => {
                    if let Err(e) = gen_fluvial_erosion_gpu(gpu, size, h, conf, progress) {
                        crate::log(&format!("gpu=>fluvial fell back to the CPU: {}", e.0));
                        gen_fluvial_erosion(size, h, conf, progress)
                    }
                }
                None => gen_fluvial_erosion(size, h, conf, progress),
            },
            StepType::Island(conf) => gen_island(size, h, conf, progress),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Step {
    /// should we skip this step when computing the heightmap ?
    pub disabled: bool,
    /// this step mask
    pub mask: Option<Vec<f32>>,
    /// how far the mask's edges are softened, 0.0 (hard edge) to 1.0 (a ramp over a quarter
    /// of the map side); `mask::feather_mask` applies it
    #[serde(default)]
    pub mask_feather: f32,
    /// step type with its configuration
    pub typ: StepType,
}

impl Default for Step {
    fn default() -> Self {
        Self {
            disabled: false,
            mask: None,
            mask_feather: 0.0,
            typ: StepType::Normalize(NormalizeConf::default()),
        }
    }
}

impl Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.typ.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_debug_variant_names() {
        for typ in StepType::all() {
            let debug = format!("{:?}", typ);
            let variant = debug.split('(').next().unwrap();
            assert_eq!(typ.name(), variant);
        }
    }

    #[test]
    fn fbm_step_agrees_across_backends() {
        let Some(gpu) = crate::gpu::test_context() else {
            return;
        };
        let step = StepType::Fbm(FbmConf::default());
        let mut cpu = vec![0.0; 64 * 64];
        let mut on_gpu = vec![0.0; 64 * 64];
        step.run(
            9,
            (64, 64),
            &mut cpu,
            &mut Progress::headless(),
            &Backend::Cpu,
        );
        step.run(
            9,
            (64, 64),
            &mut on_gpu,
            &mut Progress::headless(),
            &Backend::Gpu(gpu),
        );
        let diff = cpu
            .iter()
            .zip(&on_gpu)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(diff <= 2e-3, "max |cpu - gpu| = {diff}");
    }

    #[test]
    fn thermal_step_agrees_across_backends() {
        let Some(gpu) = crate::gpu::test_context() else {
            return;
        };
        let step = StepType::ThermalErosion(ThermalErosionConf::default());
        let input = crate::generators::calib::stock_map(9, (64, 64));
        let mut cpu = input.clone();
        let mut on_gpu = input.clone();
        step.run(
            9,
            (64, 64),
            &mut cpu,
            &mut Progress::headless(),
            &Backend::Cpu,
        );
        step.run(
            9,
            (64, 64),
            &mut on_gpu,
            &mut Progress::headless(),
            &Backend::Gpu(gpu),
        );
        assert!(cpu != input, "the step did nothing");
        let diff = cpu
            .iter()
            .zip(&on_gpu)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(diff <= 1e-4, "max |cpu - gpu| = {diff}");
    }

    #[test]
    fn fluvial_step_agrees_across_backends() {
        let Some(gpu) = crate::gpu::test_context() else {
            return;
        };
        let step = StepType::FluvialErosion(FluvialErosionConf::default());
        let input = crate::generators::calib::stock_map(9, (64, 64));
        let mut cpu = input.clone();
        let mut on_gpu = input.clone();
        step.run(
            9,
            (64, 64),
            &mut cpu,
            &mut Progress::headless(),
            &Backend::Cpu,
        );
        step.run(
            9,
            (64, 64),
            &mut on_gpu,
            &mut Progress::headless(),
            &Backend::Gpu(gpu),
        );
        assert!(cpu != input, "the step did nothing");
        assert!(on_gpu != input, "the twin did nothing");
        // the twin is a different algorithm (F-020): the same landscape, not the same heights
        let n = cpu.len() as f32;
        let (ma, mb) = (cpu.iter().sum::<f32>() / n, on_gpu.iter().sum::<f32>() / n);
        let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
        for (a, b) in cpu.iter().zip(&on_gpu) {
            sab += (a - ma) * (b - mb);
            saa += (a - ma) * (a - ma);
            sbb += (b - mb) * (b - mb);
        }
        let r = sab / (saa * sbb).sqrt();
        assert!(r > 0.99, "correlation cpu / gpu = {r}");
    }

    #[test]
    fn all_is_in_dropdown_order() {
        let names: Vec<&str> = StepType::all().iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            [
                "Hills",
                "Fbm",
                "MidPoint",
                "Ridged",
                "Normalize",
                "LandMass",
                "Plateau",
                "ThermalErosion",
                "FluvialErosion",
                "Island"
            ]
        );
    }

    #[test]
    fn legacy_generators_are_not_offered() {
        let names: Vec<&str> = StepType::all().iter().map(|t| t.name()).collect();
        assert!(!names.contains(&"MudSlide"));
        assert!(!names.contains(&"WaterErosion"));
    }

    #[test]
    fn legacy_generators_still_load() {
        let old = r#"MudSlide((iterations:5.0,max_erosion_alt:0.9,strength:0.4,water_level:0.12))"#;
        assert!(matches!(
            ron::from_str::<StepType>(old),
            Ok(StepType::MudSlide(_))
        ));
        let old = ron::to_string(&StepType::WaterErosion(WaterErosionConf::default())).unwrap();
        assert!(matches!(
            ron::from_str::<StepType>(&old),
            Ok(StepType::WaterErosion(_))
        ));
    }
}
