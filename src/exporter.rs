use std::{path::Path, sync::mpsc::Sender};

use crate::{
    panel_export::{ExportFileType, PanelExport},
    worldgen::{Step, WorldGenerator},
    ThreadMessage,
};

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
) -> Result<(), String> {
    let file_width = export_data.export_width as usize;
    let file_height = export_data.export_height as usize;
    let mut wgen = WorldGenerator::new(
        seed,
        (
            (export_data.export_width * export_data.tiles_h) as usize,
            (export_data.export_height * export_data.tiles_v) as usize,
        ),
    );
    wgen.generate(steps, tx, min_progress_step);

    let (min, max) = wgen.get_min_max();
    let coef = if max - min > std::f32::EPSILON {
        1.0 / (max - min)
    } else {
        1.0
    };

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

fn write_png(
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
            let mut h = wgen.combined_height(px + offset_x, py + offset_y);
            h = (h - min) * coef;
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

fn write_exr(
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
            let h = f16::from_f32((h - min) * coef);
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
