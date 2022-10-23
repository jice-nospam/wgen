use std::{path::Path, sync::mpsc::Sender};

use crate::{
    panel_export::PanelExport,
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
    let mut buf = vec![0u8; file_width * file_height * 2];

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
            for py in 0..file_height {
                for px in 0..file_width {
                    let mut h = wgen.combined_height(px + offset_x, py + offset_y);
                    h = (h - min) * coef;
                    let pixel = (h * 65535.0) as u16;
                    let offset = (px + py * file_width) * 2;
                    buf[offset] = (pixel & 0xff) as u8;
                    buf[offset + 1] = ((pixel & 0xff00) >> 8) as u8;
                }
            }
            let path = format!("{}_x{}_y{}.png", export_data.file_path, tx, ty);
            image::save_buffer(
                &Path::new(&path),
                &buf,
                file_width as u32,
                file_height as u32,
                image::ColorType::L16,
            )
            .map_err(|e| format!("Error while saving {}: {}", &path, e))?;
        }
    }
    Ok(())
}
