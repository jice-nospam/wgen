//! Resampling helpers shared by the generators that simulate on a reduced working grid.

/// working grid for `size`: `size` itself when it already fits, otherwise both sides
/// scaled by `max_res / max(size)`, never below 1
pub fn work_size(size: (usize, usize), max_res: usize) -> (usize, usize) {
    let longest = size.0.max(size.1);
    if longest <= max_res {
        return size;
    }
    let coef = max_res as f32 / longest as f32;
    (
        ((size.0 as f32 * coef).round() as usize).max(1),
        ((size.1 as f32 * coef).round() as usize).max(1),
    )
}

/// block average of `src` onto a `dst`-sized grid; `dst` must not exceed `size` on either axis
pub fn downsample(src: &[f32], size: (usize, usize), dst: (usize, usize)) -> Vec<f32> {
    debug_assert!(dst.0 <= size.0 && dst.1 <= size.1, "downsample enlarges the map");
    let mut sum = vec![0.0; dst.0 * dst.1];
    let mut count = vec![0u32; dst.0 * dst.1];
    for y in 0..size.1 {
        let by = y * dst.1 / size.1;
        for x in 0..size.0 {
            let bx = x * dst.0 / size.0;
            let off = bx + by * dst.0;
            sum[off] += src[x + y * size.0];
            count[off] += 1;
        }
    }
    for (val, n) in sum.iter_mut().zip(count.iter()) {
        if *n > 0 {
            *val /= *n as f32;
        }
    }
    sum
}

/// adds `delta` (a `dsize` grid) to `dst` (a `size` grid), bilinearly upsampled on cell centres
pub fn add_upsampled(dst: &mut [f32], size: (usize, usize), delta: &[f32], dsize: (usize, usize)) {
    let coefx = dsize.0 as f32 / size.0 as f32;
    let coefy = dsize.1 as f32 / size.1 as f32;
    for y in 0..size.1 {
        let v = ((y as f32 + 0.5) * coefy - 0.5).max(0.0);
        for x in 0..size.0 {
            let u = ((x as f32 + 0.5) * coefx - 0.5).max(0.0);
            dst[x + y * size.0] += bilinear(delta, u, v, dsize);
        }
    }
}

/// bilinear sample of `v` at (`x`, `y`) in cell coordinates, clamped at the far edge
pub fn bilinear(v: &[f32], x: f32, y: f32, size: (usize, usize)) -> f32 {
    let ix = x as usize;
    let iy = y as usize;
    let dx = x.fract();
    let dy = y.fract();

    let val_nw = v[ix + iy * size.0];
    let val_ne = if ix < size.0 - 1 {
        v[ix + 1 + iy * size.0]
    } else {
        val_nw
    };
    let val_sw = if iy < size.1 - 1 {
        v[ix + (iy + 1) * size.0]
    } else {
        val_nw
    };
    let val_se = if ix < size.0 - 1 && iy < size.1 - 1 {
        v[ix + 1 + (iy + 1) * size.0]
    } else {
        val_nw
    };
    let val_n = (1.0 - dx) * val_nw + dx * val_ne;
    let val_s = (1.0 - dx) * val_sw + dx * val_se;
    (1.0 - dy) * val_n + dy * val_s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_size_keeps_small_maps_and_scales_by_the_long_side() {
        assert_eq!(work_size((300, 300), 512), (300, 300));
        assert_eq!(work_size((2048, 1024), 512), (512, 256));
        assert_eq!(work_size((1000, 1000), 512), (512, 512));
        assert_eq!(work_size((4096, 100), 512), (512, 13));
    }

    #[test]
    fn downsample_of_block_constant_map_is_exact() {
        // 64x64 built from 4x4 blocks of integer values : the 16x16 average is the block value
        let mut src = vec![0.0; 64 * 64];
        for y in 0..64 {
            for x in 0..64 {
                src[x + y * 64] = ((x / 4) + (y / 4) * 16) as f32;
            }
        }
        let small = downsample(&src, (64, 64), (16, 16));
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    small[x + y * 16],
                    (x + y * 16) as f32,
                    "block ({x}, {y}) averaged wrong"
                );
            }
        }
    }

    #[test]
    fn downsample_handles_non_square() {
        // ramp along x : the downsampled ramp stays monotone on both orientations
        for (size, dst) in [((32, 16), (8, 4)), ((16, 32), (4, 8))] {
            let mut src = vec![0.0; size.0 * size.1];
            for y in 0..size.1 {
                for x in 0..size.0 {
                    src[x + y * size.0] = x as f32;
                }
            }
            let small = downsample(&src, size, dst);
            assert_eq!(small.len(), dst.0 * dst.1);
            for y in 0..dst.1 {
                for x in 1..dst.0 {
                    assert!(
                        small[x + y * dst.0] > small[x - 1 + y * dst.0],
                        "{size:?} -> {dst:?} : ramp broken at ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn add_upsampled_constant_delta_adds_it_everywhere() {
        let mut map = vec![1.0; 64 * 64];
        let delta = vec![0.25; 16 * 16];
        add_upsampled(&mut map, (64, 64), &delta, (16, 16));
        for (i, v) in map.iter().enumerate() {
            assert_eq!(*v, 1.25, "cell {i} got {v}");
        }
        let zero = vec![0.0; 16 * 16];
        add_upsampled(&mut map, (64, 64), &zero, (16, 16));
        for (i, v) in map.iter().enumerate() {
            assert_eq!(*v, 1.25, "cell {i} changed on a zero delta");
        }
    }
}
