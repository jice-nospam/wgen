//! GPU backend for the generators: an own wgpu device owned by the generator thread, a banded
//! per-pixel dispatch helper and the `Backend` switch. `src/gpu/<name>.rs` holds the GPU twin of
//! `src/generators/<name>.rs`; the CPU generator stays the reference (`work/GPU.md`).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::generators::Progress;
use crate::log;

pub mod fbm;
pub mod fluvial_erosion;
pub mod ping_pong;
pub mod plateau;
pub mod ridged;
pub mod thermal_erosion;

/// cells per band the twins pass to `run_per_pixel`: 16M cells = 64 MiB of `f32`
pub const BAND_CELLS: usize = 16 << 20;
const WORKGROUP: u32 = 16;

/// a compute step failed before it wrote anything; the caller runs the CPU generator instead
#[derive(Debug)]
pub struct GpuError(pub String);

/// which implementation `StepType::run` dispatches to
#[derive(Clone)]
pub enum Backend {
    Cpu,
    Gpu(Arc<GpuContext>),
}

impl Backend {
    /// the context to run on; `None` for `Cpu` and for a context that has failed
    pub fn gpu(&self) -> Option<&GpuContext> {
        match self {
            Backend::Cpu => None,
            Backend::Gpu(g) if g.failed.load(Ordering::Relaxed) => None,
            Backend::Gpu(g) => Some(g),
        }
    }
}

/// the band header every per-pixel kernel reads at binding 0
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Band {
    width: u32,
    height: u32,
    first_row: u32,
    rows: u32,
}

/// the bind group layout a kernel is compiled against; a kernel has exactly one
#[derive(Clone, Copy)]
enum Layout {
    /// `run_per_pixel`: 0 `Band`, 1 params, 2 the band cells read-write, 3 a read-only table
    PerPixel,
    /// `run_ping_pong`: 0 `Band`, 1 params, 2 `src` read-only, 3 `dst` read-write
    PingPong,
    /// resident multi-kernel steps (`fluvial_erosion.rs`): 0 `Band`, 1 params bound with a
    /// dynamic offset (one entry per dispatch role), then one storage buffer per element,
    /// `true` for read-only
    Resident(&'static [bool]),
}

impl Layout {
    fn entries(self) -> Vec<wgpu::BindGroupLayoutEntry> {
        let storage = |binding, read_only| {
            buffer_entry(binding, wgpu::BufferBindingType::Storage { read_only })
        };
        match self {
            Layout::PerPixel | Layout::PingPong => {
                let (two, three) = match self {
                    Layout::PerPixel => (false, true),
                    _ => (true, false),
                };
                vec![
                    buffer_entry(0, wgpu::BufferBindingType::Uniform),
                    buffer_entry(1, wgpu::BufferBindingType::Uniform),
                    storage(2, two),
                    storage(3, three),
                ]
            }
            Layout::Resident(buffers) => {
                let mut params = buffer_entry(1, wgpu::BufferBindingType::Uniform);
                if let wgpu::BindingType::Buffer {
                    ref mut has_dynamic_offset,
                    ..
                } = params.ty
                {
                    *has_dynamic_offset = true;
                }
                let mut entries = vec![buffer_entry(0, wgpu::BufferBindingType::Uniform), params];
                entries.extend(
                    buffers
                        .iter()
                        .enumerate()
                        .map(|(i, &ro)| storage(i as u32 + 2, ro)),
                );
                entries
            }
        }
    }
}

/// a compiled kernel, cached by name
struct Kernel {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

/// the band storage buffer and its readback twin, grown on demand and never shrunk
struct BandBuffers {
    storage: wgpu::Buffer,
    staging: wgpu::Buffer,
    bytes: u64,
}

pub struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    limits: wgpu::Limits,
    adapter_name: String,
    /// set by any uncaptured error or device loss; `Backend::gpu` then answers `None`
    failed: AtomicBool,
    pipelines: Mutex<HashMap<&'static str, Kernel>>,
    band_buffers: Mutex<Option<BandBuffers>>,
    ping_pong_buffers: Mutex<Option<ping_pong::PingPongBuffers>>,
}

impl GpuContext {
    /// opens an own device on the adapter named `prefer_adapter` (case-insensitive) if present,
    /// else on the high-performance one; `None` with `WGEN_CPU` set, without an adapter, or when
    /// the device cannot be created
    pub fn new(prefer_adapter: Option<&str>) -> Option<Arc<GpuContext>> {
        if std::env::var_os("WGEN_CPU").is_some() {
            log("gpu=>none (WGEN_CPU)");
            return None;
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pick_adapter(&instance, prefer_adapter)?;
        let info = adapter.get_info();
        let adapter_limits = adapter.limits();
        let limits = wgpu::Limits {
            max_buffer_size: adapter_limits.max_buffer_size,
            max_storage_buffer_binding_size: adapter_limits.max_storage_buffer_binding_size,
            ..wgpu::Limits::default()
        };
        let desc = wgpu::DeviceDescriptor {
            label: Some("wgen generators"),
            required_limits: limits.clone(),
            ..Default::default()
        };
        let (device, queue) = match pollster::block_on(adapter.request_device(&desc)) {
            Ok(dq) => dq,
            Err(e) => {
                log(&format!("gpu=>none (device creation failed: {e})"));
                return None;
            }
        };
        log(&format!(
            "gpu=>adapter \"{}\" {:?} max_buffer_size={} max_storage_buffer_binding_size={}",
            info.name, info.backend, limits.max_buffer_size, limits.max_storage_buffer_binding_size
        ));
        let ctx = Arc::new(GpuContext {
            device,
            queue,
            limits,
            adapter_name: info.name,
            failed: AtomicBool::new(false),
            pipelines: Mutex::new(HashMap::new()),
            band_buffers: Mutex::new(None),
            ping_pong_buffers: Mutex::new(None),
        });
        let weak = Arc::downgrade(&ctx);
        ctx.device.on_uncaptured_error(Arc::new(move |e| {
            log(&format!("gpu=>uncaptured error: {e}"));
            if let Some(c) = weak.upgrade() {
                c.failed.store(true, Ordering::Relaxed);
            }
        }));
        let weak = Arc::downgrade(&ctx);
        ctx.device.set_device_lost_callback(move |reason, msg| {
            log(&format!("gpu=>device lost ({reason:?}): {msg}"));
            if let Some(c) = weak.upgrade() {
                c.failed.store(true, Ordering::Relaxed);
            }
        });
        Some(ctx)
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// marks the context failed and logs; panics when the map is already half-written, since a
    /// CPU fallback would then add on top of it (`work/GPU.md` §3.2)
    fn fail(&self, written: bool, msg: String) -> GpuError {
        self.failed.store(true, Ordering::Relaxed);
        log(&format!("gpu=>failed: {msg}"));
        if written {
            panic!("{msg}");
        }
        GpuError(msg)
    }

    /// runs `kernel` (entry point `main`, the four-binding layout of `work/GPU.md` §3.4) over
    /// `hmap` in row bands of at most `cells_per_band` cells, uploading and reading back each
    /// band; `progress` is reported once per band and a cancel leaves the remaining rows untouched
    #[allow(clippy::too_many_arguments)]
    pub fn run_per_pixel(
        &self,
        kernel: &'static str,
        wgsl: &'static str,
        params: &[u8],
        table: &[u32],
        size: (usize, usize),
        hmap: &mut [f32],
        cells_per_band: usize,
        progress: &mut Progress,
    ) -> Result<(), GpuError> {
        let start = Instant::now();
        let cells_per_band =
            cells_per_band.min(self.limits.max_storage_buffer_binding_size as usize / 4);
        let bands = bands(size, cells_per_band);
        let band_cells = bands
            .iter()
            .map(|r| r.len() * size.0)
            .max()
            .unwrap_or(0)
            .max(1);
        let pass = self.prepare(kernel, wgsl, params, table, band_cells)?;
        let mut written = false;
        for rows in &bands {
            if !progress.report(rows.start as f32 / size.1 as f32) {
                return Ok(());
            }
            let cells = rows.start * size.0..rows.end * size.0;
            let header = Band {
                width: size.0 as u32,
                height: size.1 as u32,
                first_row: rows.start as u32,
                rows: rows.len() as u32,
            };
            if let Err(msg) = self.run_band(&pass, header, &mut hmap[cells]) {
                return Err(self.fail(written, format!("{kernel}: {msg}")));
            }
            written = true;
        }
        log(&format!(
            "gpu=>{kernel} {}x{} {} band(s) {} ms",
            size.0,
            size.1,
            bands.len(),
            start.elapsed().as_millis()
        ));
        Ok(())
    }

    /// the setup phase: pipeline, buffers and bind group, under error scopes so that a validation
    /// or allocation failure is reported before anything is written
    fn prepare(
        &self,
        kernel: &'static str,
        wgsl: &'static str,
        params: &[u8],
        table: &[u32],
        band_cells: usize,
    ) -> Result<Pass<'_>, GpuError> {
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let (pipeline, layout) = self.kernel(kernel, wgsl, Layout::PerPixel);
        let buffers = self.band_buffers(band_cells as u64 * 4);
        let band = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("band"),
            size: std::mem::size_of::<Band>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("params"),
                contents: params,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let table = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("table"),
                contents: bytemuck::cast_slice(table),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(kernel),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: band.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.storage.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: table.as_entire_binding(),
                },
            ],
        });
        let errors = [
            pollster::block_on(oom.pop()),
            pollster::block_on(validation.pop()),
        ];
        if let Some(e) = errors.into_iter().flatten().next() {
            return Err(self.fail(false, format!("{kernel}: {e}")));
        }
        Ok(Pass {
            pipeline,
            bind_group,
            band,
            buffers,
        })
    }

    /// uploads one band, dispatches the kernel over it and reads it back into `cells`
    fn run_band(&self, pass: &Pass, header: Band, cells: &mut [f32]) -> Result<(), String> {
        let bytes = cells.len() as u64 * 4;
        let (storage, staging) = (&pass.buffers.storage, &pass.buffers.staging);
        self.queue
            .write_buffer(storage, 0, bytemuck::cast_slice(cells));
        self.queue
            .write_buffer(&pass.band, 0, bytemuck::bytes_of(&header));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut cpass = encoder.begin_compute_pass(&Default::default());
            cpass.set_pipeline(&pass.pipeline);
            cpass.set_bind_group(0, &pass.bind_group, &[]);
            cpass.dispatch_workgroups(
                header.width.div_ceil(WORKGROUP),
                header.rows.div_ceil(WORKGROUP),
                1,
            );
        }
        encoder.copy_buffer_to_buffer(storage, 0, staging, 0, bytes);
        let idx = self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging.map_async(wgpu::MapMode::Read, 0..bytes, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(idx),
                timeout: None,
            })
            .map_err(|e| e.to_string())?;
        match rx.recv() {
            Ok(Ok(())) => (),
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => return Err("readback callback dropped".to_string()),
        }
        if self.failed.load(Ordering::Relaxed) {
            return Err("device reported an error during the band".to_string());
        }
        {
            let view = staging.get_mapped_range(0..bytes);
            cells.copy_from_slice(bytemuck::cast_slice(&view));
        }
        staging.unmap();
        Ok(())
    }

    /// the cached pipeline for `name`, compiled from `wgsl` against `layout` on first use
    fn kernel(
        &self,
        name: &'static str,
        wgsl: &'static str,
        layout: Layout,
    ) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
        let mut cache = self.pipelines.lock().unwrap();
        let k = cache.entry(name).or_insert_with(|| {
            let module = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(name),
                    source: wgpu::ShaderSource::Wgsl(wgsl.into()),
                });
            let layout = self
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(name),
                    entries: &layout.entries(),
                });
            let pipeline_layout =
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(name),
                        bind_group_layouts: &[Some(&layout)],
                        immediate_size: 0,
                    });
            let pipeline = self
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(name),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });
            Kernel { pipeline, layout }
        });
        (k.pipeline.clone(), k.layout.clone())
    }

    /// the band buffers, at least `bytes` long; the guard is held for the whole call so that the
    /// generator and export threads never share a band in flight
    fn band_buffers(&self, bytes: u64) -> BandGuard<'_> {
        let mut slot = self.band_buffers.lock().unwrap();
        if slot.as_ref().is_none_or(|b| b.bytes < bytes) {
            let storage = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("band storage"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("band staging"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            *slot = Some(BandBuffers {
                storage,
                staging,
                bytes,
            });
        }
        BandGuard(slot)
    }
}

/// exclusive use of the context's band buffers
struct BandGuard<'a>(std::sync::MutexGuard<'a, Option<BandBuffers>>);

impl std::ops::Deref for BandGuard<'_> {
    type Target = BandBuffers;
    fn deref(&self) -> &BandBuffers {
        self.0
            .as_ref()
            .expect("band buffers are created before the guard")
    }
}

/// everything one `run_per_pixel` call binds
struct Pass<'a> {
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    band: wgpu::Buffer,
    buffers: BandGuard<'a>,
}

fn buffer_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// the adapter named `prefer` if one answers to it, else the high-performance one
fn pick_adapter(instance: &wgpu::Instance, prefer: Option<&str>) -> Option<wgpu::Adapter> {
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::PRIMARY));
    if let Some(name) = prefer {
        if let Some(a) = adapters
            .into_iter()
            .find(|a| a.get_info().name.eq_ignore_ascii_case(name))
        {
            return Some(a);
        }
    }
    match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    })) {
        Ok(a) => Some(a),
        Err(e) => {
            log(&format!("gpu=>none ({e})"));
            None
        }
    }
}

/// splits `size` into consecutive row ranges of at most `cells_per_band` cells (at least one row
/// each, the last one shorter); empty for an empty map
pub fn bands(size: (usize, usize), cells_per_band: usize) -> Vec<Range<usize>> {
    let (width, height) = size;
    let rows_per_band = (cells_per_band / width.max(1)).max(1);
    (0..height)
        .step_by(rows_per_band)
        .map(|start| start..(start + rows_per_band).min(height))
        .collect()
}

/// the one context the test binary shares; `None` (tests skip) without a usable GPU
#[cfg(test)]
pub fn test_context() -> Option<Arc<GpuContext>> {
    static CTX: std::sync::OnceLock<Option<Arc<GpuContext>>> = std::sync::OnceLock::new();
    let ctx = CTX.get_or_init(|| GpuContext::new(None)).clone();
    if ctx.is_none() {
        log("gpu=>no context, test skipped");
    }
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    /// writes `y * 1000 + x` on top of each cell from the band header and the invocation id
    const TEST_WGSL: &str = r#"
struct Band { width: u32, height: u32, first_row: u32, rows: u32 }
struct Params { unused: vec4<u32> }
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> map: array<f32>;
@group(0) @binding(3) var<storage, read> table: array<u32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= band.width || gid.y >= band.rows) { return; }
    let i = gid.x + gid.y * band.width;
    map[i] = map[i] + f32(band.first_row + gid.y) * 1000.0 + f32(gid.x);
}
"#;

    fn run_test_kernel(
        gpu: &GpuContext,
        size: (usize, usize),
        cells_per_band: usize,
        progress: &mut Progress,
    ) -> Vec<f32> {
        let mut h = vec![0.5; size.0 * size.1];
        gpu.run_per_pixel(
            "test",
            TEST_WGSL,
            &[0u8; 16],
            &[0],
            size,
            &mut h,
            cells_per_band,
            progress,
        )
        .unwrap();
        h
    }

    #[test]
    fn bands_cover_every_row_once() {
        let one_row: Vec<Range<usize>> = (0..37).map(|y| y..y + 1).collect();
        assert_eq!(bands((10, 37), 10), one_row);
        let ten_rows = bands((10, 37), 100);
        assert_eq!(ten_rows, vec![0..10, 10..20, 20..30, 30..37]);
        assert!(bands((10, 0), 100).is_empty());
        assert_eq!(bands((100, 3), 10), vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn per_pixel_helper_adds_cell_coordinates() {
        let Some(gpu) = test_context() else { return };
        let size = (37, 29);
        let one = run_test_kernel(&gpu, size, size.0 * size.1, &mut Progress::headless());
        let many = run_test_kernel(&gpu, size, 37 * 7, &mut Progress::headless());
        for y in 0..size.1 {
            for x in 0..size.0 {
                let expected = 0.5 + y as f32 * 1000.0 + x as f32;
                let i = x + y * size.0;
                assert_eq!(one[i], expected, "one band, row {y} col {x}");
                assert_eq!(many[i], expected, "7-row bands, row {y} col {x}");
            }
        }
        assert_eq!(one, many);
    }

    #[test]
    fn per_pixel_helper_leaves_map_when_cancelled() {
        let Some(gpu) = test_context() else { return };
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut progress = Progress::preview(tx, 1.0, || true);
        let size = (8, 8);
        let mut h = vec![0.5; 64];
        let res = gpu.run_per_pixel(
            "test",
            TEST_WGSL,
            &[0u8; 16],
            &[0],
            size,
            &mut h,
            64,
            &mut progress,
        );
        assert!(res.is_ok());
        assert!(h.iter().all(|&v| v == 0.5));
    }
}
