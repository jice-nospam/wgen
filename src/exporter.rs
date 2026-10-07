use std::{path::Path, sync::mpsc::Sender};

use crate::{
    gpu::Backend,
    height_range::HeightRange,
    log,
    panel_export::{ExportFileType, PanelExport},
    worldgen::{Step, WorldGenerator},
    ThreadMessage,
};

#[allow(clippy::too_many_arguments)]
pub fn export_heightmap(
    // random number generator's seed to use
    seed: u64,
    // list of generator steps with their configuration and optional masks
    steps: &[Step],
    // size and number of files to export, file name pattern
    export_data: &PanelExport,
    // channel to send feedback messages to the main thread
    tx: Sender<ThreadMessage>,
    // minimum amount of progress to report (below this value, the global %age won't change)
    min_progress_step: f32,
    // where the generators with a GPU twin run
    backend: Backend,
    // raw heights written as 0..1
    height_range: HeightRange,
    // sea level in raw height units
    water_level: f32,
) -> Result<(), String> {
    let file_width = export_data.export_width as usize;
    let file_height = export_data.export_height as usize;
    let mut wgen = WorldGenerator::new(
        seed,
        (
            file_width * export_data.tiles_h as usize,
            file_height * export_data.tiles_v as usize,
        ),
    );
    wgen.set_backend(backend);
    wgen.set_water_level(water_level);
    wgen.generate(steps, tx, min_progress_step);

    let (min, coef) = export_unit(&wgen, &height_range);

    for ty in 0..export_data.tiles_v as usize {
        for tx in 0..export_data.tiles_h as usize {
            let offset_x = if export_data.seamless {
                tx * (file_width - 1)
            } else {
                tx * file_width
            };
            let offset_y = if export_data.seamless {
                ty * (file_height - 1)
            } else {
                ty * file_height
            };
            let path = format!(
                "{}_x{}_y{}.{}",
                export_data.file_path,
                tx,
                ty,
                export_data.file_type.to_string()
            );
            match export_data.file_type {
                ExportFileType::PNG => write_png(
                    file_width,
                    file_height,
                    offset_x,
                    offset_y,
                    &wgen,
                    min,
                    coef,
                    &path,
                )?,
                ExportFileType::EXR => write_exr(
                    file_width,
                    file_height,
                    offset_x,
                    offset_y,
                    &wgen,
                    min,
                    coef,
                    &path,
                )?,
            }
        }
    }
    Ok(())
}

/// `(min, coef)` for the writers, logging the cells a manual range clips
pub(crate) fn export_unit(wgen: &WorldGenerator, height_range: &HeightRange) -> (f32, f32) {
    let h = wgen.final_map();
    let clipped = height_range.count_outside(h);
    if clipped > 0 {
        let (a, b) = wgen.get_min_max();
        log(&format!(
            "export=>clipped {clipped} cells outside {}..{} (map {a:.4}..{b:.4})",
            height_range.min, height_range.max
        ));
    }
    height_range.unit(h)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_png(
    file_width: usize,
    file_height: usize,
    offset_x: usize,
    offset_y: usize,
    wgen: &WorldGenerator,
    min: f32,
    coef: f32,
    path: &str,
) -> Result<(), String> {
    let mut buf = vec![0u8; file_width * file_height * 2];
    for py in 0..file_height {
        for px in 0..file_width {
            let h = wgen.combined_height(px + offset_x, py + offset_y);
            let h = HeightRange::to01(min, coef, h);
            let offset = (px + py * file_width) * 2;
            let pixel = (h * 65535.0) as u16;
            let upixel = pixel.to_ne_bytes();
            buf[offset] = upixel[0];
            buf[offset + 1] = upixel[1];
        }
    }
    image::save_buffer(
        &Path::new(&path),
        &buf,
        file_width as u32,
        file_height as u32,
        image::ColorType::L16,
    )
    .map_err(|e| format!("Error while saving {}: {}", &path, e))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_exr(
    file_width: usize,
    file_height: usize,
    offset_x: usize,
    offset_y: usize,
    wgen: &WorldGenerator,
    min: f32,
    coef: f32,
    path: &str,
) -> Result<(), String> {
    use exr::prelude::*;

    let channel = SpecificChannels::new(
        (ChannelDescription::named("Y", SampleType::F16),),
        |Vec2(px, py)| {
            let h = wgen.combined_height(px + offset_x, py + offset_y);
            let h = f16::from_f32(HeightRange::to01(min, coef, h));
            (h,)
        },
    );

    Image::from_encoded_channels(
        (file_width, file_height),
        Encoding {
            compression: Compression::ZIP1,
            blocks: Blocks::ScanLines,
            line_order: LineOrder::Increasing,
        },
        channel,
    )
    .write()
    .to_file(path)
    .map_err(|e| format!("Error while saving {}: {}", &path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_png_clamps_to_the_manual_range() {
        let mut wgen = WorldGenerator::new(1, (2, 1));
        wgen.push_map(vec![-1.0, 2.0]);
        let range = HeightRange {
            auto: false,
            min: 0.0,
            max: 1.0,
        };
        let (min, coef) = export_unit(&wgen, &range);
        let dir = format!("{}/target", env!("CARGO_MANIFEST_DIR"));
        let path = format!("{dir}/write_png_clamps.png");
        write_png(2, 1, 0, 0, &wgen, min, coef, &path).unwrap();
        let img = image::open(&path).unwrap().into_luma16();
        assert_eq!(img.into_raw(), vec![0, 65535]);
    }
}
