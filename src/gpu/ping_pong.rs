//! Iterative kernels on the working grid: the whole map in two storage buffers, one dispatch per
//! pass alternating them, one readback at the end (`work/GPU.md` §3.4). `impl GpuContext` lives
//! here so the helper reaches the context's private fields.

use std::ops::Range;
use std::sync::atomic::Ordering;
use std::sync::MutexGuard;

use wgpu::util::DeviceExt;

use super::{Band, GpuContext, GpuError, Layout, WORKGROUP};
use crate::generators::Progress;

/// passes per command submission: bounds the work a submission holds (TDR) and spaces the
/// cancel points
pub const PING_PONG_CHUNK: usize = 8;

/// the two storage buffers the passes alternate between and their readback twin, grown on demand
/// and never shrunk
pub(super) struct PingPongBuffers {
    a: wgpu::Buffer,
    b: wgpu::Buffer,
    staging: wgpu::Buffer,
    bytes: u64,
}

/// exclusive use of the context's ping-pong buffers for a whole `run_ping_pong` call
struct PingPongGuard<'a>(MutexGuard<'a, Option<PingPongBuffers>>);

impl std::ops::Deref for PingPongGuard<'_> {
    type Target = PingPongBuffers;
    fn deref(&self) -> &PingPongBuffers {
        self.0
            .as_ref()
            .expect("ping-pong buffers are created before the guard")
    }
}

/// everything one `run_ping_pong` call binds
struct Run<'a> {
    pipeline: wgpu::ComputePipeline,
    /// `a → b` for even passes, `b → a` for odd ones
    bind_groups: [wgpu::BindGroup; 2],
    buffers: PingPongGuard<'a>,
}

impl GpuContext {
    /// runs `kernel` (entry point `main`, the `Layout::PingPong` bindings) `passes` times over the
    /// whole of `hmap`, each pass reading the previous one's output, and reads the result back
    /// once. `Ok(true)` with `hmap` holding the result; `Ok(false)` when `progress` cancelled and
    /// `Err` on any device error, `hmap` untouched in both cases: the map is written only from a
    /// successful final readback, so this helper never panics. `progress` is reported once per
    /// chunk of `PING_PONG_CHUNK` passes, mapped onto `window`.
    #[allow(clippy::too_many_arguments)]
    pub fn run_ping_pong(
        &self,
        kernel: &'static str,
        wgsl: &'static str,
        params: &[u8],
        size: (usize, usize),
        hmap: &mut [f32],
        passes: usize,
        window: (f32, f32),
        progress: &mut Progress,
    ) -> Result<bool, GpuError> {
        debug_assert_eq!(hmap.len(), size.0 * size.1);
        if passes == 0 || hmap.is_empty() {
            return Ok(true);
        }
        let bytes = hmap.len() as u64 * 4;
        if bytes > self.limits.max_storage_buffer_binding_size {
            return Err(self.fail(
                false,
                format!(
                    "{kernel}: {}x{} exceeds the storage binding limit",
                    size.0, size.1
                ),
            ));
        }
        let run = self.prepare_ping_pong(kernel, wgsl, params, size, hmap)?;
        let workgroups = (
            (size.0 as u32).div_ceil(WORKGROUP),
            (size.1 as u32).div_ceil(WORKGROUP),
        );
        for first in (0..passes).step_by(PING_PONG_CHUNK) {
            let p = window.0 + (window.1 - window.0) * first as f32 / passes as f32;
            if !progress.report(p) {
                return Ok(false);
            }
            let last = (first + PING_PONG_CHUNK).min(passes);
            let final_chunk = last == passes;
            let idx = self.submit_chunk(&run, first..last, workgroups, final_chunk, bytes);
            if self.failed.load(Ordering::Relaxed) {
                return Err(self.fail(
                    false,
                    format!("{kernel}: device reported an error during the passes"),
                ));
            }
            if final_chunk {
                if let Err(msg) = self.read_back(&run.buffers.staging, bytes, idx, hmap) {
                    return Err(self.fail(false, format!("{kernel}: {msg}")));
                }
            }
        }
        Ok(true)
    }

    /// the setup phase: pipeline, buffers, uniforms, the two bind groups and the upload of
    /// `hmap`, under error scopes so that a validation or allocation failure is an `Err`
    fn prepare_ping_pong(
        &self,
        kernel: &'static str,
        wgsl: &'static str,
        params: &[u8],
        size: (usize, usize),
        hmap: &[f32],
    ) -> Result<Run<'_>, GpuError> {
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let (pipeline, layout) = self.kernel(kernel, wgsl, Layout::PingPong);
        let buffers = self.ping_pong_buffers(hmap.len() as u64 * 4);
        let band = Band {
            width: size.0 as u32,
            height: size.1 as u32,
            first_row: 0,
            rows: size.1 as u32,
        };
        let band = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("band"),
                contents: bytemuck::bytes_of(&band),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("params"),
                contents: params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind_group = |src: &wgpu::Buffer, dst: &wgpu::Buffer| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                        resource: src.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: dst.as_entire_binding(),
                    },
                ],
            })
        };
        let bind_groups = [
            bind_group(&buffers.a, &buffers.b),
            bind_group(&buffers.b, &buffers.a),
        ];
        self.queue
            .write_buffer(&buffers.a, 0, bytemuck::cast_slice(hmap));
        let errors = [
            pollster::block_on(oom.pop()),
            pollster::block_on(validation.pop()),
        ];
        if let Some(e) = errors.into_iter().flatten().next() {
            return Err(self.fail(false, format!("{kernel}: {e}")));
        }
        Ok(Run {
            pipeline,
            bind_groups,
            buffers,
        })
    }

    /// one submission holding the passes of `range`, each in its own compute pass so the
    /// previous pass's writes are ordered before the next one's reads; the final chunk also
    /// copies the buffer written last into the staging buffer
    fn submit_chunk(
        &self,
        run: &Run,
        range: Range<usize>,
        workgroups: (u32, u32),
        final_chunk: bool,
        bytes: u64,
    ) -> wgpu::SubmissionIndex {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for pass in range.clone() {
            let mut cpass = encoder.begin_compute_pass(&Default::default());
            cpass.set_pipeline(&run.pipeline);
            cpass.set_bind_group(0, &run.bind_groups[pass % 2], &[]);
            cpass.dispatch_workgroups(workgroups.0, workgroups.1, 1);
        }
        if final_chunk {
            let written = if range.end % 2 == 1 {
                &run.buffers.b
            } else {
                &run.buffers.a
            };
            encoder.copy_buffer_to_buffer(written, 0, &run.buffers.staging, 0, bytes);
        }
        self.queue.submit([encoder.finish()])
    }

    /// waits for submission `idx` and copies the staging buffer into `hmap`
    fn read_back(
        &self,
        staging: &wgpu::Buffer,
        bytes: u64,
        idx: wgpu::SubmissionIndex,
        hmap: &mut [f32],
    ) -> Result<(), String> {
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
            return Err("device reported an error during the passes".to_string());
        }
        {
            let view = staging.get_mapped_range(0..bytes);
            hmap.copy_from_slice(bytemuck::cast_slice(&view));
        }
        staging.unmap();
        Ok(())
    }

    /// the ping-pong buffers, at least `bytes` long; the guard is held for the whole call, so two
    /// iterative steps on different threads never share a buffer in flight
    fn ping_pong_buffers(&self, bytes: u64) -> PingPongGuard<'_> {
        let mut slot = self.ping_pong_buffers.lock().unwrap();
        if slot.as_ref().is_none_or(|b| b.bytes < bytes) {
            let storage = |label| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                })
            };
            let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ping-pong staging"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            *slot = Some(PingPongBuffers {
                a: storage("ping-pong a"),
                b: storage("ping-pong b"),
                staging,
                bytes,
            });
        }
        PingPongGuard(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::Progress;
    use crate::gpu::{test_context, GpuError};

    /// `dst[i] = src[i] + 1`
    const ADD_ONE_WGSL: &str = r#"
struct Band { width: u32, height: u32, first_row: u32, rows: u32 }
struct Params { unused: vec4<u32> }
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> src: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= band.width || gid.y >= band.rows) { return; }
    let i = gid.x + (band.first_row + gid.y) * band.width;
    dst[i] = src[i] + 1.0;
}
"#;

    /// `dst[x, y] = src[x - 1, y]`, 0 at `x = 0`: every pass shifts the row right by one cell
    const SHIFT_WGSL: &str = r#"
struct Band { width: u32, height: u32, first_row: u32, rows: u32 }
struct Params { unused: vec4<u32> }
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> src: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= band.width || gid.y >= band.rows) { return; }
    let i = gid.x + (band.first_row + gid.y) * band.width;
    if (gid.x == 0u) {
        dst[i] = 0.0;
    } else {
        dst[i] = src[i - 1u];
    }
}
"#;

    fn run(
        gpu: &GpuContext,
        kernel: &'static str,
        wgsl: &'static str,
        size: (usize, usize),
        hmap: &mut [f32],
        passes: usize,
        progress: &mut Progress,
    ) -> Result<bool, GpuError> {
        gpu.run_ping_pong(
            kernel,
            wgsl,
            &[0u8; 16],
            size,
            hmap,
            passes,
            (0.0, 1.0),
            progress,
        )
    }

    #[test]
    fn ping_pong_runs_every_pass() {
        let Some(gpu) = test_context() else { return };
        let mut h = vec![0.5; 64 * 64];
        let res = run(
            &gpu,
            "pp_add_one",
            ADD_ONE_WGSL,
            (64, 64),
            &mut h,
            2 * PING_PONG_CHUNK + 1,
            &mut Progress::headless(),
        );
        assert!(matches!(res, Ok(true)), "{res:?}");
        let expected = 0.5 + (2 * PING_PONG_CHUNK + 1) as f32;
        if let Some(i) = h.iter().position(|&v| v != expected) {
            panic!("cell {i} is {} instead of {expected}", h[i]);
        }
    }

    #[test]
    fn ping_pong_zero_passes_leaves_map_untouched() {
        let Some(gpu) = test_context() else { return };
        let mut h = vec![0.5; 64 * 64];
        let res = run(
            &gpu,
            "pp_add_one",
            ADD_ONE_WGSL,
            (64, 64),
            &mut h,
            0,
            &mut Progress::headless(),
        );
        assert!(matches!(res, Ok(true)), "{res:?}");
        assert!(h.iter().all(|&v| v == 0.5));
    }

    #[test]
    fn ping_pong_orders_passes() {
        let Some(gpu) = test_context() else { return };
        let mut h = vec![0.0; 16];
        h[3] = 1.0;
        let res = run(
            &gpu,
            "pp_shift",
            SHIFT_WGSL,
            (16, 1),
            &mut h,
            5,
            &mut Progress::headless(),
        );
        assert!(matches!(res, Ok(true)), "{res:?}");
        let mut expected = vec![0.0; 16];
        expected[8] = 1.0;
        assert_eq!(h, expected);
    }

    #[test]
    fn ping_pong_cancel_leaves_map_untouched() {
        let Some(gpu) = test_context() else { return };
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut progress = Progress::preview(tx, 1.0, || true);
        let mut h = vec![0.5; 64 * 64];
        let res = run(
            &gpu,
            "pp_add_one",
            ADD_ONE_WGSL,
            (64, 64),
            &mut h,
            3,
            &mut progress,
        );
        assert!(matches!(res, Ok(false)), "{res:?}");
        assert!(h.iter().all(|&v| v == 0.5));
    }

    #[test]
    fn ping_pong_is_deterministic() {
        let Some(gpu) = test_context() else { return };
        let input: Vec<f32> = (0..64 * 64).map(|i| (i % 13) as f32 * 0.25).collect();
        let mut a = input.clone();
        let mut b = input;
        for h in [&mut a, &mut b] {
            let res = run(
                &gpu,
                "pp_shift",
                SHIFT_WGSL,
                (64, 64),
                h,
                PING_PONG_CHUNK + 3,
                &mut Progress::headless(),
            );
            assert!(matches!(res, Ok(true)), "{res:?}");
        }
        assert_eq!(a, b);
    }
}
