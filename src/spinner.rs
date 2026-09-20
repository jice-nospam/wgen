//! egui's busy spinner without its per-frame repaint request: the arc is drawn from the frame's
//! time, so it turns at whatever rate the app renders (the 100 ms busy poll while a thread runs)
//! instead of forcing a frame every frame, which starves the generator thread's GPU steps.

use egui::{lerp, vec2, Pos2, Sense, Shape, Stroke, Ui, WidgetInfo, WidgetType};

/// the same arc as `ui.spinner()`, at the style's `interact_size`
pub fn spinner(ui: &mut Ui) {
    let size = ui.style().spacing.interact_size.y;
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    response.widget_info(|| WidgetInfo::new(WidgetType::ProgressIndicator));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let color = ui.visuals().strong_text_color();
    let radius = (rect.height().min(rect.width()) / 2.0) - 2.0;
    let n_points = (radius.round() as u32).clamp(8, 128);
    let time = ui.input(|i| i.time);
    let start_angle = time * std::f64::consts::TAU;
    let end_angle = start_angle + 240f64.to_radians() * time.sin();
    let points: Vec<Pos2> = (0..n_points)
        .map(|i| {
            let angle = lerp(start_angle..=end_angle, i as f64 / n_points as f64);
            let (sin, cos) = angle.sin_cos();
            rect.center() + radius * vec2(cos as f32, sin as f32)
        })
        .collect();
    ui.painter()
        .add(Shape::line(points, Stroke::new(3.0, color)));
}
