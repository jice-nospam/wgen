//! Noise conventions shared by the noise-based generators (Fbm's virtual plane and zoom,
//! per-stream seeding, a domain warp), so that each generator holds only its own formula.

use noise::{Fbm, MultiFractal, NoiseFn, Perlin, RidgedMulti};

/// the seed of noise stream `stream` (0, 1, 2 … in the order the generator's chapter lists
/// them) of the map seed `seed`; the top bit is cleared so the crate's `seed + octave` source
/// seeding never overflows
pub fn stream_seed(seed: u64, stream: u32) -> u32 {
    (seed as u32).wrapping_add(stream.wrapping_mul(0x9E37_79B9)) & 0x7FFF_FFFF
}

/// an Fbm over Perlin at the crate's defaults, seeded from stream `stream`
pub fn fbm_stream(seed: u64, stream: u32, octaves: usize) -> Fbm<Perlin> {
    Fbm::<Perlin>::new(stream_seed(seed, stream)).set_octaves(octaves)
}

/// a ridged multifractal over Perlin at the crate's defaults (lacunarity 2π/3, persistence
/// 1.0, attenuation 2.0), seeded from stream `stream`
pub fn ridged_stream(seed: u64, stream: u32, octaves: usize) -> RidgedMulti<Perlin> {
    RidgedMulti::<Perlin>::new(stream_seed(seed, stream)).set_octaves(octaves)
}

/// the noise frequency per virtual pixel for a `zoom` parameter: the Fbm convention
pub fn noise_coef(zoom: f32) -> f32 {
    zoom / 400.0
}

/// the position of cell `(x, y)` on the 512-wide virtual plane `gen_fbm` samples, computed in
/// f32 so that a cell of a small map and the matching cell of a larger one get the same value
pub fn virtual_coords(size: (usize, usize), x: usize, y: usize) -> (f32, f32) {
    (
        x as f32 * 512.0 / size.0 as f32,
        y as f32 * 512.0 / size.1 as f32,
    )
}

/// a domain warp: displaces a point by `amount` times two independent Fbm fields
pub struct Warp {
    a: Fbm<Perlin>,
    b: Fbm<Perlin>,
    coef: f64,
    amount: f64,
}

impl Warp {
    /// `stream_a` / `stream_b` seed the two fields, `zoom` is their zoom and `amount_px` the
    /// displacement in virtual pixels (a conf gives it as a % of the map side: `pct / 100 · 512`)
    pub fn new(
        seed: u64,
        stream_a: u32,
        stream_b: u32,
        octaves: usize,
        zoom: f32,
        amount_px: f32,
    ) -> Self {
        Self {
            a: fbm_stream(seed, stream_a, octaves),
            b: fbm_stream(seed, stream_b, octaves),
            coef: noise_coef(zoom) as f64,
            amount: amount_px as f64,
        }
    }

    /// `p + amount · (a(p · coef), b(p · coef))`
    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let q = [p[0] * self.coef, p[1] * self.coef];
        [
            p[0] + self.amount * self.a.get(q),
            p[1] + self.amount * self.b.get(q),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_seed_is_stable_and_distinct() {
        // pinned: a change to the formula reshapes every saved landscape
        assert_eq!(stream_seed(7, 0), 7);
        assert_eq!(stream_seed(7, 1), 506_952_128);
        let seeds: Vec<u32> = (0..4).map(|s| stream_seed(7, s)).collect();
        for i in 0..4 {
            assert!(seeds[i] < 0x8000_0000);
            for j in 0..i {
                assert_ne!(seeds[i], seeds[j], "streams {i} and {j} collide");
            }
        }
        assert_ne!(stream_seed(u64::MAX, 3), stream_seed(u64::MAX, 4));
    }

    #[test]
    fn warp_zero_is_identity() {
        let warp = Warp::new(3, 1, 2, 3, 1.5, 0.0);
        for p in [[0.0, 0.0], [17.5, 300.25], [511.0, 3.0]] {
            assert_eq!(warp.apply(p), p);
        }
        let bent = Warp::new(3, 1, 2, 3, 1.5, 51.2);
        assert_ne!(bent.apply([17.5, 300.25]), [17.5, 300.25]);
    }
}
