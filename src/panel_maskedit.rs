use eframe::egui;
use egui_extras::RetainedImage;
use epaint::{Color32, ColorImage};
use image::{imageops::FilterType, GrayImage, Luma};

use crate::generators::get_min_max;

pub enum PanelMaskEditAction {}
pub struct PanelMaskEdit {
    buff: GrayImage,
    image_size: usize,
    preview_size: usize,
    img: Option<RetainedImage>,
    brush_value: f32,
    brush_size: f32,
    brush_falloff: f32,
}

impl PanelMaskEdit {
    pub fn new(image_size: usize, preview_size: u32) -> Self {
        PanelMaskEdit {
            buff: GrayImage::new(1, 1),
            image_size,
            preview_size: preview_size as usize,
            img: None,
            brush_value: 1.0,
            brush_size: 8.0,
            brush_falloff: 0.0,
        }
    }
    pub fn display_mask(&mut self, image_size: usize, preview_size: u32, mask: Option<Vec<f32>>) {
        self.image_size = image_size;
        self.preview_size = preview_size as usize;
        let buff = if let Some(mask) = mask {
            let (min, max) = get_min_max(&mask);
            let coef = if max - min > std::f32::EPSILON {
                1.0 / (max - min)
            } else {
                1.0
            };
            GrayImage::from_fn(preview_size, preview_size, |x, y| {
                let mut h = mask[x as usize + y as usize * preview_size as usize];
                h = (h - min) * coef;
                Luma([(h * 255.0).clamp(0.0, 255.0) as u8])
            })
        } else {
            let mut img = GrayImage::new(1, 1);
            img.fill(255);
            img
        };
        self.buff = image::imageops::resize(
            &buff,
            self.image_size as u32,
            self.image_size as u32,
            FilterType::Nearest,
        );
        let mut img = ColorImage::new([self.image_size, self.image_size], Color32::BLACK);
        for y in 0..self.image_size {
            for x in 0..self.image_size {
                let rgb = self.buff.get_pixel(x as u32, y as u32)[0];
                img[(x, y)][0] = rgb;
                img[(x, y)][1] = rgb;
                img[(x, y)][2] = rgb;
            }
        }
        self.img = Some(RetainedImage::from_color_image("mask", img));
    }
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<PanelMaskEditAction> {
        ui.vertical(|ui| {
            if let Some(img) = &self.img {
                img.show(ui);
            }
            ui.label("Use left and right mouse buttons to edit the mask");
        });
        None
    }
}
