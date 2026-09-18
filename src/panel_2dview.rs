use egui::{Color32, ColorImage, TextureHandle};

use crate::{fps::FpsCounter, panel_maskedit::PanelMaskEdit, worldgen::ExportMap};

pub enum Panel2dAction {
    /// the preview size has changed : terrain and 3d view must be recomputed
    ResizePreview(usize),
    /// a brush stroke ended : this is the mask being edited, to store on its step
    MaskCommitted(Vec<f32>),
    /// the mask being edited was cleared : remove it from its step
    MaskDelete,
}
pub struct Panel2dView {
    /// preview image of the heightmap
    img: ColorImage,
    /// minimum value in the heightmap
    min: f32,
    /// maximum value in the heightmap
    max: f32,
    /// are we displaying the mask editor ?
    mask_mode: bool,
    /// size of the preview canvas in pixels
    image_size: usize,
    /// size of the heightmap
    preview_size: usize,
    /// should we update the preview every time a step is computed ?
    pub live_preview: bool,
    /// utility to display FPS
    fps_counter: FpsCounter,
    /// GPU texture of `img`, uploaded lazily in `render`
    ui_img: Option<TextureHandle>,
    /// `img` changed since the last upload to `ui_img`
    img_dirty: bool,
    /// a preview size change waiting for a frame with no other action to report
    pending_resize: Option<usize>,
    /// last heightmap displayed, re-rendered when the canvas size changes
    last_hmap: Option<ExportMap>,
    /// mask editor subpanel
    mask_editor: PanelMaskEdit,
}

impl Panel2dView {
    pub fn new(image_size: usize, preview_size: u32, hmap: &ExportMap) -> Self {
        let mut panel = Panel2dView {
            img: ColorImage::filled([image_size, image_size], Color32::BLACK),
            min: 0.0,
            max: 0.0,
            image_size,
            mask_mode: false,
            live_preview: true,
            preview_size: preview_size as usize,
            fps_counter: FpsCounter::default(),
            ui_img: None,
            img_dirty: false,
            pending_resize: None,
            last_hmap: None,
            mask_editor: PanelMaskEdit::new(image_size),
        };
        panel.refresh(image_size, preview_size, Some(hmap));
        panel
    }
    /// shows the mask editor on top of the current heightmap
    pub fn display_mask(&mut self, image_size: usize, preview_size: u32, mask: Vec<f32>) {
        self.image_size = image_size;
        self.preview_size = preview_size as usize;
        self.mask_editor.display_mask(image_size, mask);
        self.mask_mode = true;
    }
    /// shows the heightmap alone
    pub fn exit_mask_mode(&mut self) {
        self.mask_mode = false;
    }
    /// re-renders the preview image; the mask editor, if shown, keeps its mask on top of the new image
    pub fn refresh(&mut self, image_size: usize, preview_size: u32, hmap: Option<&ExportMap>) {
        self.image_size = image_size;
        self.preview_size = preview_size as usize;
        if self.img.width() != image_size {
            self.img = ColorImage::filled([self.image_size, self.image_size], Color32::BLACK);
        }
        if let Some(hmap) = hmap {
            self.last_hmap = Some(hmap.clone());
        }
        if let Some(hmap) = &self.last_hmap {
            let (min, max) = hmap.get_min_max();
            let coef = if max - min > std::f32::EPSILON {
                1.0 / (max - min)
            } else {
                1.0
            };
            self.min = min;
            self.max = max;
            let mut idx = 0;
            for y in 0..image_size {
                let py = ((y * preview_size as usize) as f32 / image_size as f32) as usize;
                for x in 0..image_size {
                    let px = ((x * preview_size as usize) as f32 / image_size as f32) as usize;
                    let mut h = hmap.height(px as usize, py as usize);
                    h = (h - min) * coef;
                    self.img.pixels[idx] = Color32::from_gray((h * 255.0).clamp(0.0, 255.0) as u8);
                    idx += 1;
                }
            }
        };
        self.img_dirty = true;
        if self.mask_mode {
            self.mask_editor.heightmap_changed(image_size);
        }
    }
    /// uploads `img` to the GPU texture when it changed since the last frame
    fn upload_image(&mut self, ctx: &egui::Context) {
        if !self.img_dirty {
            return;
        }
        match &mut self.ui_img {
            Some(handle) => handle.set(self.img.clone(), egui::TextureOptions::LINEAR),
            None => {
                self.ui_img =
                    Some(ctx.load_texture("hmap", self.img.clone(), egui::TextureOptions::LINEAR))
            }
        }
        self.img_dirty = false;
    }
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<Panel2dAction> {
        let mut action = None;
        let old_size = self.preview_size;
        self.fps_counter.new_frame();
        self.upload_image(ui.ctx());
        if self.mask_mode {
            action = match &self.ui_img {
                Some(handle) => self.mask_editor.render(ui, handle.id()),
                None => None,
            };
        } else {
            ui.vertical(|ui| {
                if let Some(handle) = &self.ui_img {
                    ui.image((handle.id(), handle.size_vec2()));
                }
                ui.horizontal(|ui| {
                    ui.label(format!("Height range : {} - {}", self.min, self.max));
                });
            });
        }
        ui.label(format!("FPS : {}", self.fps_counter.fps()));
        ui.horizontal(|ui| {
            ui.label("Preview size");
            egui::ComboBox::from_label("")
                .selected_text(format!("{}x{}", self.preview_size, self.preview_size))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.preview_size, 64, "64x64");
                    ui.selectable_value(&mut self.preview_size, 128, "128x128");
                    ui.selectable_value(&mut self.preview_size, 256, "256x256");
                    ui.selectable_value(&mut self.preview_size, 512, "512x512");
                });
            ui.label("Live preview");
            ui.checkbox(&mut self.live_preview, "");
        });
        if self.preview_size != old_size {
            self.pending_resize = Some(self.preview_size);
        }
        if action.is_none() {
            // a mask commit reported in the same frame goes first : the resize recomputes everything
            action = self.pending_resize.take().map(Panel2dAction::ResizePreview);
        }
        action
    }
}
