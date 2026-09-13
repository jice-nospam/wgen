use std::sync::atomic::{AtomicBool, Ordering};

use super::Progress;

/// Runs `f(y, row)` over every row of `map` (row-major, `width` cells per row), the rows split
/// into contiguous strips over `num_cpus::get()` scoped threads. Progress runs from `window.0`
/// to `window.1` as strip 0 advances. Returns false once the step was cancelled: the other
/// strips stop at their next row and `map` holds a partial result.
pub fn par_rows<F>(
    width: usize,
    map: &mut [f32],
    progress: &mut Progress,
    window: (f32, f32),
    f: F,
) -> bool
where
    F: Fn(usize, &mut [f32]) + Sync,
{
    par_rows_with(num_cpus::get(), width, map, progress, window, f)
}

/// `par_rows` with an explicit thread count; tests use it to prove split independence.
fn par_rows_with<F>(
    threads: usize,
    width: usize,
    map: &mut [f32],
    progress: &mut Progress,
    window: (f32, f32),
    f: F,
) -> bool
where
    F: Fn(usize, &mut [f32]) + Sync,
{
    if map.is_empty() {
        return true;
    }
    let rows = map.len() / width;
    let threads = threads.max(1);
    // at most `threads` strips, at least one row each
    let strip_rows = rows.div_ceil(threads);
    let (p0, p1) = window;
    let cancelled = AtomicBool::new(false);
    let cancelled = &cancelled;
    let f = &f;
    // only strip 0 reports progress; on cancel it raises the flag and the other strips stop at their next row
    let mut progress = Some(progress);
    std::thread::scope(|s| {
        for (strip, chunk) in map.chunks_mut(strip_rows * width).enumerate() {
            let mut strip_progress = if strip == 0 { progress.take() } else { None };
            s.spawn(move || {
                for (local_y, row) in chunk.chunks_mut(width).enumerate() {
                    if strip_progress.is_none() && cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    f(strip * strip_rows + local_y, row);
                    if let Some(progress) = strip_progress.as_mut() {
                        let p = p0 + (p1 - p0) * (local_y + 1) as f32 / strip_rows as f32;
                        if !progress.report(p) {
                            cancelled.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            });
        }
    });
    !cancelled.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThreadMessage;

    #[test]
    fn par_rows_visits_every_row_once_at_any_thread_count() {
        let (width, rows) = (5, 7);
        let expected: Vec<f32> = (0..rows)
            .flat_map(|y| std::iter::repeat_n(y as f32, width))
            .collect();
        for threads in [1, 3, 16] {
            let mut map = vec![-1.0; width * rows];
            let done = par_rows_with(
                threads,
                width,
                &mut map,
                &mut Progress::headless(),
                (0.0, 1.0),
                |y, row| row.iter_mut().for_each(|v| *v = y as f32),
            );
            assert!(done, "{threads} threads: reported cancelled");
            assert_eq!(map, expected, "{threads} threads: wrong rows");
        }
    }

    #[test]
    fn par_rows_progress_spans_the_window_and_ends_at_p1() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut progress = Progress::preview(tx, 0.0, || false);
        let mut map = vec![0.0; 5 * 7];
        assert!(par_rows_with(
            3,
            5,
            &mut map,
            &mut progress,
            (0.25, 0.75),
            |_, _| {}
        ));
        let mut last = None;
        while let Ok(ThreadMessage::GeneratorStepProgress(p)) = rx.try_recv() {
            assert!(
                (0.25..=0.75 + 1e-6).contains(&p),
                "progress {p} outside the window"
            );
            last = Some(p);
        }
        let last = last.expect("no progress message received");
        assert!((last - 0.75).abs() < 1e-6, "last progress {last} is not p1");
    }

    #[test]
    fn par_rows_returns_false_when_cancelled() {
        let (tx, _) = std::sync::mpsc::channel();
        let mut progress = Progress::preview(tx, 1.0, || true);
        let mut map = vec![0.0; 5 * 7];
        assert!(!par_rows_with(
            3,
            5,
            &mut map,
            &mut progress,
            (0.0, 1.0),
            |_, _| {}
        ));
    }

    #[test]
    fn par_rows_handles_an_empty_map() {
        assert!(par_rows(
            5,
            &mut [],
            &mut Progress::headless(),
            (0.0, 1.0),
            |_, _| { panic!("closure called on an empty map") }
        ));
    }

    /// `cargo test par_rows_speedup_report -- --ignored --nocapture`: the number that replaces a launch
    #[test]
    #[ignore]
    fn par_rows_speedup_report() {
        let n = 2048;
        let mut map = vec![0.5; n * n];
        let body = |y: usize, row: &mut [f32]| {
            for (x, v) in row.iter_mut().enumerate() {
                *v = (*v + (x + y) as f32 / n as f32).powf(2.5);
            }
        };
        for threads in [1, num_cpus::get()] {
            let start = std::time::Instant::now();
            par_rows_with(
                threads,
                n,
                &mut map,
                &mut Progress::headless(),
                (0.0, 1.0),
                body,
            );
            println!(
                "par_rows {n}x{n} powf: {threads} thread(s) -> {:?}",
                start.elapsed()
            );
        }
    }
}
