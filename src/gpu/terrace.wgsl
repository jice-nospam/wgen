// Port of `generators::plateau::terrace_profile`: the tread/riser shape inside one level.
// Shared by the Plateau and Canyon kernels.

fn terrace(f: f32, flat: f32, rounding: f32) -> f32 {
    let g = clamp((f - 0.5 * flat) / (1.0 - flat), 0.0, 1.0);
    let s = g * g * (3.0 - 2.0 * g);
    return g + (s - g) * rounding;
}
