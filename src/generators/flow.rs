//! Drainage routing for the generators that need a river network : a priority flood from the
//! base level gives every cell a receiver and a drainage area, depressions included.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::f32::consts::SQRT_2;

use super::{DIRX, DIRY};

/// the drainage network of a map : where each cell drains and how much drains through it.
/// Built by `route`; the scratch buffers are kept so routing again allocates nothing.
#[derive(Default)]
pub struct FlowNet {
    /// cells in flood pop order : every cell comes after the cell it drains into
    pub order: Vec<u32>,
    /// the cell each cell drains into; a base-level cell drains into itself
    pub recv: Vec<u32>,
    /// drainage area in cells, the cell itself included
    pub area: Vec<f32>,
    /// heights with every depression raised to its spill level
    filled: Vec<f32>,
    /// cells already pushed on the heap
    closed: Vec<bool>,
    /// (height key, cell) : the lowest filled height pops first, ties by cell index
    heap: BinaryHeap<Reverse<(u32, u32)>>,
}

impl FlowNet {
    pub fn new() -> Self {
        Self::default()
    }

    /// routes `h` : the map border and every cell at or below `water_level` are the base level
    pub fn route(&mut self, size: (usize, usize), h: &[f32], water_level: f32) {
        let n = size.0 * size.1;
        debug_assert!(n < u32::MAX as usize, "cell indices are u32");
        debug_assert_eq!(h.len(), n);
        self.flood(size, h, water_level);
        self.steepest_receivers(size);
        self.accumulate();
    }

    pub fn is_base_level(&self, i: usize) -> bool {
        self.recv[i] as usize == i
    }

    /// priority flood from the base level : fills `order` (pop order), `recv` (the neighbour
    /// that discovered each cell) and `filled`
    fn flood(&mut self, size: (usize, usize), h: &[f32], water_level: f32) {
        let n = size.0 * size.1;
        self.order.clear();
        self.order.reserve(n);
        self.recv.clear();
        self.recv.resize(n, 0);
        self.filled.clear();
        self.filled.extend_from_slice(h);
        self.closed.clear();
        self.closed.resize(n, false);
        self.heap.clear();
        for y in 0..size.1 {
            for x in 0..size.0 {
                let i = x + y * size.0;
                let border = x == 0 || y == 0 || x == size.0 - 1 || y == size.1 - 1;
                if border || h[i] <= water_level {
                    self.closed[i] = true;
                    self.recv[i] = i as u32;
                    self.heap.push(Reverse((height_key(h[i]), i as u32)));
                }
            }
        }
        while let Some(Reverse((_, c))) = self.heap.pop() {
            self.order.push(c);
            let ci = c as usize;
            let cx = (ci % size.0) as i32;
            let cy = (ci / size.0) as i32;
            for d in 1..9 {
                let nx = cx + DIRX[d];
                let ny = cy + DIRY[d];
                if nx < 0 || ny < 0 || nx >= size.0 as i32 || ny >= size.1 as i32 {
                    continue;
                }
                let ni = nx as usize + ny as usize * size.0;
                if self.closed[ni] {
                    continue;
                }
                self.closed[ni] = true;
                let f = h[ni].max(self.filled[ci]);
                self.filled[ni] = f;
                self.recv[ni] = c;
                self.heap.push(Reverse((height_key(f), ni as u32)));
            }
        }
    }

    /// D8 on the filled heights : the steepest strictly lower neighbour becomes the receiver;
    /// a cell without one (a flat, a lake floor) keeps the cell that discovered it
    fn steepest_receivers(&mut self, size: (usize, usize)) {
        for i in 0..size.0 * size.1 {
            let r = self.recv[i] as usize;
            if r == i {
                continue;
            }
            let fi = self.filled[i];
            let mut best = r;
            let mut best_slope = (fi - self.filled[r]) / receiver_distance(i, r, size.0);
            let x = (i % size.0) as i32;
            let y = (i / size.0) as i32;
            for d in 1..9 {
                let nx = x + DIRX[d];
                let ny = y + DIRY[d];
                if nx < 0 || ny < 0 || nx >= size.0 as i32 || ny >= size.1 as i32 {
                    continue;
                }
                let ni = nx as usize + ny as usize * size.0;
                let fnb = self.filled[ni];
                if fnb >= fi {
                    continue;
                }
                let slope = (fi - fnb) / receiver_distance(i, ni, size.0);
                if slope > best_slope {
                    best_slope = slope;
                    best = ni;
                }
            }
            self.recv[i] = best as u32;
        }
    }

    /// drainage area : one cell each, summed downstream in reverse pop order
    fn accumulate(&mut self) {
        self.area.clear();
        self.area.resize(self.recv.len(), 1.0);
        for &i in self.order.iter().rev() {
            let r = self.recv[i as usize] as usize;
            if r != i as usize {
                self.area[r] += self.area[i as usize];
            }
        }
    }
}

/// distance in cells between a cell and one of its 8 neighbours
pub fn receiver_distance(i: usize, r: usize, width: usize) -> f32 {
    let dx = (i % width) as i32 - (r % width) as i32;
    let dy = (i / width) as i32 - (r / width) as i32;
    if dx != 0 && dy != 0 {
        SQRT_2
    } else {
        1.0
    }
}

/// the bit pattern of `h` folded so that `u32` order is float order, negatives included
fn height_key(h: f32) -> u32 {
    let bits = h.to_bits();
    if bits & 0x8000_0000 != 0 {
        !bits
    } else {
        bits | 0x8000_0000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (usize, usize) = (16, 16);

    /// integer pyramid with a noise term : has pits and flats
    fn bumpy() -> Vec<f32> {
        let mut hmap = vec![0.0; 256];
        for y in 0..16 {
            for x in 0..16 {
                hmap[x + y * 16] = (32 - (x as i32 - 8).abs() - (y as i32 - 8).abs()) as f32
                    + ((x * 7 + y * 13) % 5) as f32;
            }
        }
        hmap
    }

    fn route(h: &[f32], water_level: f32) -> FlowNet {
        let mut net = FlowNet::new();
        net.route(SIZE, h, water_level);
        net
    }

    /// number of receiver links from `i` to a base-level cell; panics on a cycle
    fn chain_length(net: &FlowNet, mut i: usize) -> usize {
        let mut steps = 0;
        while !net.is_base_level(i) {
            i = net.recv[i] as usize;
            steps += 1;
            assert!(steps <= 256, "cycle from {i}");
        }
        steps
    }

    #[test]
    fn height_key_is_monotone() {
        let keys: Vec<u32> = [-10.0, -1.0, -0.5, 0.0, 0.25, 1.0, 100.0]
            .iter()
            .map(|h| height_key(*h))
            .collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "{keys:?}");
    }

    #[test]
    fn every_cell_drains_to_a_seed_in_order() {
        let net = route(&bumpy(), 0.0);
        let mut seen = vec![false; 256];
        let mut position = vec![0; 256];
        for (p, &i) in net.order.iter().enumerate() {
            assert!(!seen[i as usize], "cell {i} popped twice");
            seen[i as usize] = true;
            position[i as usize] = p;
        }
        assert!(seen.iter().all(|s| *s), "a cell was never popped");
        for i in 0..256 {
            if !net.is_base_level(i) {
                let r = net.recv[i] as usize;
                assert!(position[r] < position[i], "receiver of {i} comes after it");
            }
            chain_length(&net, i);
        }
    }

    #[test]
    fn pit_drains_over_its_rim() {
        let mut h = vec![1.0; 256];
        h[8 + 8 * 16] = 0.0;
        let net = route(&h, -1.0);
        assert!(!net.is_base_level(8 + 8 * 16));
        assert!(chain_length(&net, 8 + 8 * 16) >= 7);
    }

    #[test]
    fn ramp_accumulates_along_rows() {
        let h: Vec<f32> = (0..256).map(|i| (i % 16) as f32).collect();
        let net = route(&h, -1.0);
        for y in 1..15 {
            assert_eq!(net.area[1 + y * 16], 14.0, "row {y}");
            assert_eq!(net.area[14 + y * 16], 1.0, "row {y}");
        }
    }

    #[test]
    fn flat_map_is_fully_routed() {
        let net = route(&vec![0.5; 256], -1.0);
        for y in 1..15 {
            for x in 1..15 {
                assert!(!net.is_base_level(x + y * 16), "({x}, {y}) has no receiver");
                chain_length(&net, x + y * 16);
            }
        }
    }

    #[test]
    fn same_input_same_net() {
        let a = route(&bumpy(), 0.0);
        let b = route(&bumpy(), 0.0);
        assert_eq!(a.order, b.order);
        assert_eq!(a.recv, b.recv);
        assert_eq!(a.area, b.area);
    }
}
