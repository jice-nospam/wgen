//! The step mask: a fixed square of 0..1 weights blended over the map by `worldgen::apply_mask`,
//! and the feather that softens its edges before the blend

/// side of every step mask, in cells
pub const MASK_SIZE: usize = 64;
/// the ramp of a 1.0 feather, as a fraction of the map side
pub const MAX_FEATHER: f32 = 0.25;

/// softens the mask's edges: the grayscale erosion of the mask by a cone of slope `1 / R`,
/// `f(p) = min over q of (mask(q) + d(p, q) / R)` with `R = feather × MASK_SIZE × MAX_FEATHER`
/// cells, so no cell brightens and the mask never climbs faster than `1 / R` per cell. `d` is
/// the chamfer (1, √2) distance, computed by the two-pass sequential erosion. `feather` is
/// clamped to `0.0..=1.0`; at `R <= 1` the erosion is the identity and the mask is returned as is
pub fn feather_mask(mask: &[f32], feather: f32) -> Vec<f32> {
    let radius = feather.clamp(0.0, 1.0) * MASK_SIZE as f32 * MAX_FEATHER;
    let mut out = mask.to_vec();
    if radius <= 1.0 {
        return out;
    }
    let (edge, diag) = (1.0 / radius, std::f32::consts::SQRT_2 / radius);
    let n = MASK_SIZE;
    let at = |x: usize, y: usize| x + y * n;
    // forward pass: from the left, up-left, up and up-right neighbours
    for y in 0..n {
        for x in 0..n {
            let mut v = out[at(x, y)];
            if x > 0 {
                v = v.min(out[at(x - 1, y)] + edge);
            }
            if y > 0 {
                v = v.min(out[at(x, y - 1)] + edge);
                if x > 0 {
                    v = v.min(out[at(x - 1, y - 1)] + diag);
                }
                if x + 1 < n {
                    v = v.min(out[at(x + 1, y - 1)] + diag);
                }
            }
            out[at(x, y)] = v;
        }
    }
    // backward pass: from the right, down-right, down and down-left neighbours
    for y in (0..n).rev() {
        for x in (0..n).rev() {
            let mut v = out[at(x, y)];
            if x + 1 < n {
                v = v.min(out[at(x + 1, y)] + edge);
            }
            if y + 1 < n {
                v = v.min(out[at(x, y + 1)] + edge);
                if x + 1 < n {
                    v = v.min(out[at(x + 1, y + 1)] + diag);
                }
                if x > 0 {
                    v = v.min(out[at(x - 1, y + 1)] + diag);
                }
            }
            out[at(x, y)] = v;
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// a fixed checkerboard of 0.0 / 0.5 / 1.0 cells
    fn patterned_mask() -> Vec<f32> {
        (0..MASK_SIZE * MASK_SIZE)
            .map(|i| ((i % MASK_SIZE + i / MASK_SIZE) % 3) as f32 * 0.5)
            .collect()
    }

    /// columns 0..=31 black, 32..=63 white
    pub(crate) fn half_black_mask() -> Vec<f32> {
        (0..MASK_SIZE * MASK_SIZE)
            .map(|i| if i % MASK_SIZE < MASK_SIZE / 2 { 0.0 } else { 1.0 })
            .collect()
    }

    #[test]
    fn feather_zero_is_identity() {
        let mask = patterned_mask();
        assert_eq!(feather_mask(&mask, 0.0), mask);
        // R = 0.8 cell: the neighbour term is above 1, still the identity
        assert_eq!(feather_mask(&mask, 0.05), mask);
    }

    #[test]
    fn feather_never_brightens() {
        let mask = patterned_mask();
        let out = feather_mask(&mask, 0.7);
        assert!(out.iter().zip(&mask).all(|(o, m)| o <= m));
    }

    #[test]
    fn feather_ramps_from_a_hard_edge() {
        // R = 8 cells
        let out = feather_mask(&half_black_mask(), 0.5);
        let row = 10 * MASK_SIZE;
        for x in 0..MASK_SIZE {
            let expected = if x <= 31 { 0.0 } else { ((x - 31) as f32 / 8.0).min(1.0) };
            assert!((out[x + row] - expected).abs() < 1e-6, "x={x}: {} vs {expected}", out[x + row]);
        }
        assert_eq!(out[39 + row], 1.0);
    }

    #[test]
    fn feather_bounds_the_slope_and_is_symmetric() {
        // one black cell at the centre of a white mask, R = 4 cells
        let c = MASK_SIZE / 2;
        let mut mask = vec![1.0; MASK_SIZE * MASK_SIZE];
        mask[c + c * MASK_SIZE] = 0.0;
        let out = feather_mask(&mask, 0.25);
        let at = |x: usize, y: usize| out[x + y * MASK_SIZE];
        for y in 0..MASK_SIZE {
            for x in 0..MASK_SIZE {
                if x + 1 < MASK_SIZE {
                    assert!((at(x, y) - at(x + 1, y)).abs() <= 0.25 + 1e-6);
                }
                if y + 1 < MASK_SIZE {
                    assert!((at(x, y) - at(x, y + 1)).abs() <= 0.25 + 1e-6);
                }
            }
        }
        assert!((at(c + 3, c) - 0.75).abs() < 1e-6);
        let diag = 2.0 * std::f32::consts::SQRT_2 / 4.0;
        for (dx, dy) in [(2i32, 2i32), (2, -2), (-2, 2), (-2, -2)] {
            let v = at((c as i32 + dx) as usize, (c as i32 + dy) as usize);
            assert!((v - diag).abs() < 1e-6, "({dx}, {dy}): {v} vs {diag}");
        }
    }

    #[test]
    fn feather_is_clamped() {
        let mask = half_black_mask();
        assert_eq!(feather_mask(&mask, 2.0), feather_mask(&mask, 1.0));
        assert_eq!(feather_mask(&mask, -1.0), mask);
    }
}
