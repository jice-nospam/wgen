//! The two Bevy cameras behind the egui panels: one that carries the egui context and clears
//! the window, and one that draws the 3D scene into the "3d preview" square.
use bevy::camera::visibility::RenderLayers;
use bevy::camera::Viewport;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiGlobalSettings, PrimaryEguiContext};

/// marker of the camera that renders the 3D scene into the preview square
#[derive(Component)]
pub struct SceneCamera;

/// where the 3D square sits on screen this frame, written by the UI, read by `apply_viewport`
#[derive(Resource, Default)]
pub struct PreviewViewport {
    /// the square in egui points; `None` when the "3d preview" header is collapsed
    pub rect: Option<egui::Rect>,
    /// egui points to physical pixels
    pub pixels_per_point: f32,
}

/// egui camera (order 0): clears the whole window to the panel colour and draws only egui.
/// scene camera (order 1): drawn after egui, scissored to the preview square.
pub fn spawn_cameras(mut commands: Commands, mut settings: ResMut<EguiGlobalSettings>) {
    // otherwise bevy_egui attaches its context to the first camera it sees
    settings.auto_create_primary_context = false;
    commands.spawn((
        PrimaryEguiContext,
        Camera2d,
        RenderLayers::none(),
        Camera {
            order: 0,
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(27, 27, 27)),
            ..default()
        },
    ));
    commands.spawn((
        SceneCamera,
        Camera3d::default(),
        Camera {
            order: 1,
            is_active: false,
            viewport: Some(Viewport::default()),
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(10, 10, 10)),
            msaa_writeback: MsaaWriteback::Off,
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// moves the scene camera's viewport onto the preview square; the camera is inactive while
/// the square is hidden or clamped to nothing, so the viewport is never empty nor outside the window
pub fn apply_viewport(
    vp: Res<PreviewViewport>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cam: Single<&mut Camera, With<SceneCamera>>,
) {
    let Some((pos, size)) =
        viewport_in_window(vp.rect, vp.pixels_per_point, window.physical_size())
    else {
        cam.is_active = false;
        return;
    };
    cam.is_active = true;
    cam.viewport = Some(Viewport {
        physical_position: pos,
        physical_size: size,
        depth: 0.0..1.0,
    });
}

/// the square in physical pixels, clamped inside the window; `None` when nothing is left of it
fn viewport_in_window(
    rect: Option<egui::Rect>,
    pixels_per_point: f32,
    window: UVec2,
) -> Option<(UVec2, UVec2)> {
    let rect = rect?;
    let to_px = |v: f32| (v * pixels_per_point).round().max(0.0) as u32;
    let pos = UVec2::new(to_px(rect.min.x), to_px(rect.min.y)).min(window);
    let size = UVec2::new(to_px(rect.width()), to_px(rect.height())).min(window - pos);
    if size.x == 0 || size.y == 0 {
        None
    } else {
        Some((pos, size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_is_scaled_and_clamped() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(300.0, 300.0));
        assert_eq!(
            viewport_in_window(Some(rect), 2.0, UVec2::new(1000, 1000)),
            Some((UVec2::new(200, 100), UVec2::new(600, 600)))
        );
        // the square spills over the window: clipped to it
        assert_eq!(
            viewport_in_window(Some(rect), 1.0, UVec2::new(300, 200)),
            Some((UVec2::new(100, 50), UVec2::new(200, 150)))
        );
        // window smaller than the square's origin, or no square at all: no viewport
        assert_eq!(
            viewport_in_window(Some(rect), 1.0, UVec2::new(100, 100)),
            None
        );
        assert_eq!(viewport_in_window(None, 1.0, UVec2::new(1000, 1000)), None);
    }
}
