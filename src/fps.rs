use std::time::{Duration, Instant};

pub struct FpsCounter {
    last: Instant,
    fps_counter: usize,
    fps: usize,
}

impl Default for FpsCounter {
    fn default() -> Self {
        Self {
            last: Instant::now(),
            fps_counter: 0,
            fps: 0,
        }
    }
}

impl FpsCounter {
    pub fn new_frame(&mut self) {
        self.fps_counter += 1;
        if self.last.elapsed() >= Duration::from_secs(1) {
            self.fps = self.fps_counter;
            self.fps_counter = 0;
            self.last = Instant::now();
        }
    }
    pub fn fps(&self) -> usize {
        self.fps
    }
}
