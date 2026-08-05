use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::{config::Renderer, media::VideoFrame};

pub struct VideoWidget<'a> {
    pub frame: &'a VideoFrame,
    pub renderer: Renderer,
    pub color: bool,
    pub ramp: &'a str,
}

impl Widget for VideoWidget<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        match self.renderer {
            Renderer::Ascii => self.render_ascii(area, buffer),
            Renderer::Blocks => self.render_blocks(area, buffer),
        }
    }
}

impl VideoWidget<'_> {
    fn render_ascii(&self, area: Rect, buffer: &mut Buffer) {
        let ramp: Vec<char> = self.ramp.chars().collect();
        let crop = cover_crop(
            self.frame.width,
            self.frame.height,
            area.width,
            area.height,
            2.0,
        );
        for row in 0..area.height {
            for col in 0..area.width {
                let source_x = crop_coord(col, area.width, crop.x, crop.width, self.frame.width);
                let source_y = crop_coord(row, area.height, crop.y, crop.height, self.frame.height);
                let rgb = self.frame.pixel(source_x, source_y);
                let luminance = luminance(rgb);
                let index = usize::from(luminance) * (ramp.len() - 1) / 255;
                let foreground = if self.color {
                    rgb_color(rgb)
                } else {
                    Color::White
                };
                buffer[(area.x + col, area.y + row)]
                    .set_char(ramp[index])
                    .set_style(Style::default().fg(foreground).bg(Color::Black));
            }
        }
    }

    fn render_blocks(&self, area: Rect, buffer: &mut Buffer) {
        let virtual_height = area.height.saturating_mul(2);
        let crop = cover_crop(
            self.frame.width,
            self.frame.height,
            area.width,
            virtual_height,
            1.0,
        );
        for row in 0..area.height {
            for col in 0..area.width {
                let source_x = crop_coord(col, area.width, crop.x, crop.width, self.frame.width);
                let top_virtual = row * 2;
                let bottom_virtual = top_virtual + 1;
                let top = self.frame.pixel(
                    source_x,
                    crop_coord(
                        top_virtual,
                        virtual_height,
                        crop.y,
                        crop.height,
                        self.frame.height,
                    ),
                );
                let bottom = self.frame.pixel(
                    source_x,
                    crop_coord(
                        bottom_virtual,
                        virtual_height,
                        crop.y,
                        crop.height,
                        self.frame.height,
                    ),
                );
                let (foreground, background) = if self.color {
                    (rgb_color(top), rgb_color(bottom))
                } else {
                    (gray_color(luminance(top)), gray_color(luminance(bottom)))
                };
                buffer[(area.x + col, area.y + row)]
                    .set_char('▀')
                    .set_style(Style::default().fg(foreground).bg(background));
            }
        }
    }
}

impl VideoFrame {
    fn pixel(&self, x: u16, y: u16) -> [u8; 3] {
        let index = (usize::from(y) * usize::from(self.width) + usize::from(x)) * 3;
        [
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
        ]
    }
}

#[derive(Debug, Clone, Copy)]
struct Crop {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn cover_crop(
    source_width: u16,
    source_height: u16,
    destination_width: u16,
    destination_height: u16,
    cell_aspect: f32,
) -> Crop {
    let destination_ratio =
        f32::from(destination_width) / (f32::from(destination_height.max(1)) * cell_aspect);
    let source_ratio = f32::from(source_width) / f32::from(source_height.max(1));
    if source_ratio > destination_ratio {
        let width = f32::from(source_height) * destination_ratio;
        Crop {
            x: (f32::from(source_width) - width) / 2.0,
            y: 0.0,
            width,
            height: f32::from(source_height),
        }
    } else {
        let height = f32::from(source_width) / destination_ratio;
        Crop {
            x: 0.0,
            y: (f32::from(source_height) - height) / 2.0,
            width: f32::from(source_width),
            height,
        }
    }
}

fn crop_coord(
    value: u16,
    destination_size: u16,
    crop_start: f32,
    crop_size: f32,
    source_size: u16,
) -> u16 {
    let normalized = (f32::from(value) + 0.5) / f32::from(destination_size.max(1));
    (crop_start + normalized * crop_size)
        .floor()
        .clamp(0.0, f32::from(source_size.saturating_sub(1))) as u16
}

fn luminance(rgb: [u8; 3]) -> u8 {
    ((u16::from(rgb[0]) * 54 + u16::from(rgb[1]) * 183 + u16::from(rgb[2]) * 19) / 256) as u8
}

fn rgb_color(rgb: [u8; 3]) -> Color {
    Color::Rgb(rgb[0], rgb[1], rgb[2])
}
fn gray_color(value: u8) -> Color {
    Color::Rgb(value, value, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinate_scaling_stays_in_bounds() {
        assert_eq!(crop_coord(99, 100, 0.0, 192.0, 192), 191);
        assert_eq!(crop_coord(0, 1, 0.0, 192.0, 192), 96);
    }

    #[test]
    fn black_and_white_luminance_are_stable() {
        assert_eq!(luminance([0, 0, 0]), 0);
        assert_eq!(luminance([255, 255, 255]), 255);
    }

    #[test]
    fn cover_crop_fills_destination_by_cropping_source() {
        let crop = cover_crop(192, 108, 80, 20, 2.0);
        assert_eq!(crop.x, 0.0);
        assert_eq!(crop.width, 192.0);
        assert!(crop.y > 0.0);
        assert!(crop.height < 108.0);
    }
}
