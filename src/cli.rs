//! Command-line mode: loads a `.wgen` project and exports its heightmap without the editor,
//! logging every step's time. This is how a generation is measured without a launch.

use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use crate::exporter::{write_exr, write_png};
use crate::gpu::{Backend, GpuContext};
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
  wgen --export <project.wgen> --out <file.png|file.exr> [--size <side>|<width>x<height>] [--cpu]
                                         generate the project's stack at the given size
                                         (default 1024) and write it as a 16-bit PNG or an EXR;
                                         --cpu runs every generator on the CPU
  wgen --help                            this text";

/// one `--export` invocation
#[derive(Debug, PartialEq)]
pub struct ExportArgs {
    pub project: String,
    pub out: String,
    pub size: (usize, usize),
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
    let mut cpu = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--help" | "-h" => return Err(USAGE.to_owned()),
            "--export" => project = Some(value(&mut it, arg)?),
            "--out" => out = Some(value(&mut it, arg)?),
            "--size" => size = parse_size(&value(&mut it, arg)?)?,
            "--cpu" => cpu = true,
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    match (project, out) {
        (Some(project), Some(out)) => Ok(Some(ExportArgs {
            project,
            out,
            size,
            cpu,
        })),
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
    let bad = || format!("bad --size {s}: expected <side> or <width>x<height>");
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
    let worker = thread::spawn(move || {
        let mut wgen = WorldGenerator::new(seed, size);
        wgen.set_backend(backend);
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
    write(&wgen, &args.out)?;
    log(&format!(
        "cli=>wrote {} in {:.2} s total",
        args.out,
        start.elapsed().as_secs_f32()
    ));
    Ok(())
}

/// the whole map as one file, PNG or EXR by extension, heights rescaled to 0..1
fn write(wgen: &WorldGenerator, out: &str) -> Result<(), String> {
    let map = wgen.get_export_map();
    let (w, h) = map.get_size();
    let (min, max) = map.get_min_max();
    let coef = if max - min > f32::EPSILON {
        1.0 / (max - min)
    } else {
        1.0
    };
    let lower = out.to_ascii_lowercase();
    if lower.ends_with(".png") {
        write_png(w, h, 0, 0, wgen, min, coef, out)
    } else if lower.ends_with(".exr") {
        write_exr(w, h, 0, 0, wgen, min, coef, out)
    } else {
        Err(format!("--out {out}: the extension must be .png or .exr"))
    }
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
