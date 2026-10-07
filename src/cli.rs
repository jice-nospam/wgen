//! Command-line mode: loads a `.wgen` project and exports its heightmap without the editor,
//! logging every step's time. This is how a generation is measured without a launch.

use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use crate::exporter::{export_unit, write_exr, write_png};
use crate::gpu::{Backend, GpuContext};
use crate::height_range::HeightRange;
use crate::log;
use crate::project::Project;
use crate::worldgen::WorldGenerator;
use crate::ThreadMessage;

/// progress granularity of the export reporter in command-line mode
const PROGRESS_STEP: f32 = 0.25;
/// default side of the exported map
const DEFAULT_SIZE: usize = 1024;

pub const USAGE: &str = "usage:
  wgen                                   open the editor
  wgen --export <project.wgen> --out <file.png|file.exr> [--size <side>|<width>x<height>]
       [--tiles <n>|<nx>x<ny>] [--cpu]
                                         generate the project's stack at the given size
                                         (default 1024) and write it as a 16-bit PNG or an EXR;
                                         --tiles splits the map into nx × ny files
                                         <name>_x<i>_y<j>.<ext> (the size must divide exactly);
                                         --cpu runs every generator on the CPU
  wgen --help                            this text";

/// one `--export` invocation
#[derive(Debug, PartialEq)]
pub struct ExportArgs {
    pub project: String,
    pub out: String,
    pub size: (usize, usize),
    /// files across and down; the whole map is `size`
    pub tiles: (usize, usize),
    pub cpu: bool,
}

/// the command line without the program name: `Ok(None)` opens the editor, `Ok(Some)` runs an
/// export, `Err` is the message to print (the usage text for `--help`)
pub fn parse(args: &[String]) -> Result<Option<ExportArgs>, String> {
    if args.is_empty() {
        return Ok(None);
    }
    let mut project = None;
    let mut out = None;
    let mut size = (DEFAULT_SIZE, DEFAULT_SIZE);
    let mut tiles = (1, 1);
    let mut cpu = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--help" | "-h" => return Err(USAGE.to_owned()),
            "--export" => project = Some(value(&mut it, arg)?),
            "--out" => out = Some(value(&mut it, arg)?),
            "--size" => size = parse_size(&value(&mut it, arg)?)?,
            "--tiles" => tiles = parse_pair("--tiles", &value(&mut it, arg)?)?,
            "--cpu" => cpu = true,
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    match (project, out) {
        (Some(project), Some(out)) => {
            if !size.0.is_multiple_of(tiles.0) || !size.1.is_multiple_of(tiles.1) {
                return Err(format!(
                    "--size {}x{} does not split into {}x{} tiles exactly",
                    size.0, size.1, tiles.0, tiles.1
                ));
            }
            Ok(Some(ExportArgs {
                project,
                out,
                size,
                tiles,
                cpu,
            }))
        }
        (None, None) => Err(USAGE.to_owned()),
        _ => Err(format!("--export and --out go together\n{USAGE}")),
    }
}

fn value(it: &mut std::slice::Iter<String>, flag: &str) -> Result<String, String> {
    it.next()
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value\n{USAGE}"))
}

/// `<side>` or `<width>x<height>`, both at least 1
fn parse_size(s: &str) -> Result<(usize, usize), String> {
    parse_pair("--size", s)
}

/// `<n>` or `<a>x<b>`, both at least 1
fn parse_pair(flag: &str, s: &str) -> Result<(usize, usize), String> {
    let bad = || format!("bad {flag} {s}: expected <n> or <a>x<b>");
    let (w, h) = match s.split_once('x') {
        Some((w, h)) => (w.parse().map_err(|_| bad())?, h.parse().map_err(|_| bad())?),
        None => {
            let side = s.parse().map_err(|_| bad())?;
            (side, side)
        }
    };
    if w == 0 || h == 0 {
        return Err(bad());
    }
    Ok((w, h))
}

/// runs the project's stack on a worker thread, prints one line per step as it finishes, then
/// writes the map; the backend is the GPU when one opens and `--cpu` is absent
pub fn run(args: ExportArgs) -> Result<(), String> {
    let start = Instant::now();
    let project = Project::load(&args.project)?;
    let backend = match if args.cpu {
        None
    } else {
        GpuContext::new(None)
    } {
        Some(gpu) => {
            log(&format!("cli=>backend GPU ({})", gpu.adapter_name()));
            Backend::Gpu(gpu)
        }
        None => {
            log("cli=>backend CPU");
            Backend::Cpu
        }
    };
    let names: Vec<&str> = project.steps.iter().map(|s| s.typ.name()).collect();
    log(&format!(
        "cli=>{} : {} step(s) at {}x{}",
        args.project,
        names.len(),
        args.size.0,
        args.size.1
    ));
    let (tx, rx) = mpsc::channel();
    let steps = project.steps.clone();
    let size = args.size;
    let seed = project.seed;
    let water_level = project.water_level();
    let worker = thread::spawn(move || {
        let mut wgen = WorldGenerator::new(seed, size);
        wgen.set_backend(backend);
        wgen.set_water_level(water_level);
        wgen.generate(&steps, tx, PROGRESS_STEP);
        wgen
    });
    let mut step_start = Instant::now();
    while let Ok(msg) = rx.recv() {
        if let ThreadMessage::ExporterStepDone(i) = msg {
            log(&format!(
                "cli=>step {} {} : {:.2} s",
                i,
                names.get(i).unwrap_or(&"?"),
                step_start.elapsed().as_secs_f32()
            ));
            step_start = Instant::now();
        }
    }
    let wgen = worker
        .join()
        .map_err(|e| format!("generation failed: {}", crate::panic_message(&*e)))?;
    log_heights(&wgen);
    write(&wgen, &args.out, args.tiles, &project.height_range)?;
    log(&format!(
        "cli=>wrote {} in {:.2} s total",
        args.out,
        start.elapsed().as_secs_f32()
    ));
    Ok(())
}

/// logs the raw height range of the final map, before the export rescales it to 0..1
fn log_heights(wgen: &WorldGenerator) {
    let (min, max) = wgen.get_export_map().get_min_max();
    log(&format!("cli=>heights {min:.4}..{max:.4}"));
}

/// the path of tile `(i, j)`: `out` itself for a single tile, `<stem>_x<i>_y<j>.<ext>` otherwise
fn tile_path(out: &str, tiles: (usize, usize), i: usize, j: usize) -> String {
    if tiles == (1, 1) {
        return out.to_owned();
    }
    let path = Path::new(out);
    let stem = path.with_extension("");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    format!("{}_x{i}_y{j}.{ext}", stem.display())
}

/// the map as `tiles` files, PNG or EXR by extension, heights rescaled to 0..1 over the whole map
fn write(
    wgen: &WorldGenerator,
    out: &str,
    tiles: (usize, usize),
    height_range: &HeightRange,
) -> Result<(), String> {
    let (w, h) = wgen.get_export_map().get_size();
    let (min, coef) = export_unit(wgen, height_range);
    let (tw, th) = (w / tiles.0, h / tiles.1);
    let lower = out.to_ascii_lowercase();
    for j in 0..tiles.1 {
        for i in 0..tiles.0 {
            let path = tile_path(out, tiles, i, j);
            if lower.ends_with(".png") {
                write_png(tw, th, i * tw, j * th, wgen, min, coef, &path)?
            } else if lower.ends_with(".exr") {
                write_exr(tw, th, i * tw, j * th, wgen, min, coef, &path)?
            } else {
                return Err(format!("--out {out}: the extension must be .png or .exr"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_arguments_opens_the_editor() {
        assert_eq!(parse(&[]), Ok(None));
    }

    #[test]
    fn export_arguments_parse() {
        let parsed = parse(&args(&[
            "--export", "a.wgen", "--out", "b.png", "--size", "256x128", "--cpu",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            parsed,
            ExportArgs {
                project: "a.wgen".into(),
                out: "b.png".into(),
                size: (256, 128),
                tiles: (1, 1),
                cpu: true,
            }
        );
        let square = parse(&args(&[
            "--export", "a.wgen", "--out", "b.exr", "--size", "64",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(square.size, (64, 64));
        assert!(!square.cpu);
        let tiled = parse(&args(&[
            "--export", "a.wgen", "--out", "b.png", "--size", "512", "--tiles", "2",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(tiled.tiles, (2, 2));
        let tiled = parse(&args(&[
            "--export", "a.wgen", "--out", "b.png", "--size", "512x256", "--tiles", "4x2",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(tiled.tiles, (4, 2));
    }

    #[test]
    fn tiles_must_divide_the_size() {
        assert!(parse(&args(&[
            "--export", "a.wgen", "--out", "b.png", "--size", "100", "--tiles", "3"
        ]))
        .is_err());
        assert!(parse(&args(&[
            "--export", "a.wgen", "--out", "b.png", "--size", "64", "--tiles", "0"
        ]))
        .is_err());
    }

    #[test]
    fn tile_paths_follow_the_editor_pattern() {
        assert_eq!(tile_path("dir/map.png", (1, 1), 0, 0), "dir/map.png");
        assert_eq!(tile_path("dir/map.png", (2, 2), 1, 0), "dir/map_x1_y0.png");
    }

    #[test]
    fn tiled_write_equals_slices_of_the_whole_map() {
        use crate::generators::FbmConf;
        use crate::worldgen::{Step, StepType};
        let dir = std::env::temp_dir().join("wgen_cli_tiles");
        std::fs::create_dir_all(&dir).unwrap();
        let steps = vec![Step {
            typ: StepType::Fbm(FbmConf::default()),
            ..Default::default()
        }];
        let (tx, _rx) = mpsc::channel();
        let mut wgen = WorldGenerator::new(3, (32, 32));
        wgen.generate(&steps, tx, 1.0);
        let whole = dir.join("whole.png").to_string_lossy().into_owned();
        let tiled = dir.join("tiled.png").to_string_lossy().into_owned();
        write(&wgen, &whole, (1, 1), &HeightRange::default()).unwrap();
        write(&wgen, &tiled, (2, 2), &HeightRange::default()).unwrap();
        let whole = image::open(&whole).unwrap().into_luma16();
        for j in 0..2u32 {
            for i in 0..2u32 {
                let tile = image::open(tile_path(&tiled, (2, 2), i as usize, j as usize))
                    .unwrap()
                    .into_luma16();
                assert_eq!(tile.dimensions(), (16, 16));
                for (x, y, p) in tile.enumerate_pixels() {
                    assert_eq!(*p, *whole.get_pixel(x + i * 16, y + j * 16), "tile {i},{j}");
                }
            }
        }
    }

    #[test]
    fn bad_arguments_are_errors() {
        assert!(parse(&args(&["--help"])).unwrap_err().starts_with("usage"));
        assert!(parse(&args(&["--export", "a.wgen"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
        assert!(parse(&args(&[
            "--export", "a.wgen", "--out", "b.png", "--size", "0"
        ]))
        .is_err());
        assert!(parse(&args(&[
            "--export", "a.wgen", "--out", "b.png", "--size", "axb"
        ]))
        .is_err());
    }
}
