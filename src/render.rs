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
        let (draw, offset_x, offset_y) = fit(area, self.frame.width, self.frame.height, 2.0);
        for row in 0..draw.height {
            for col in 0..draw.width {
                let source_x = scale_coord(col, draw.width, self.frame.width);
                let source_y = scale_coord(row, draw.height, self.frame.height);
                let rgb = self.frame.pixel(source_x, source_y);
                let luminance = luminance(rgb);
                let index = usize::from(luminance) * (ramp.len() - 1) / 255;
                let foreground = if self.color {
                    rgb_color(rgb)
                } else {
                    gray_color(luminance)
                };
                buffer[(area.x + offset_x + col, area.y + offset_y + row)]
                    .set_char(ramp[index])
                    .set_style(Style::default().fg(foreground).bg(Color::Black));
            }
        }
    }

    fn render_blocks(&self, area: Rect, buffer: &mut Buffer) {
        let virtual_height = area.height.saturating_mul(2);
        let virtual_area = Rect::new(0, 0, area.width, virtual_height);
        let (draw, offset_x, offset_y) =
            fit(virtual_area, self.frame.width, self.frame.height, 1.0);
        let terminal_rows = draw.height.div_ceil(2);
        for row in 0..terminal_rows.min(area.height) {
            for col in 0..draw.width {
                let source_x = scale_coord(col, draw.width, self.frame.width);
                let top_virtual = (row * 2).min(draw.height.saturating_sub(1));
                let bottom_virtual = (top_virtual + 1).min(draw.height.saturating_sub(1));
                let top = self.frame.pixel(
                    source_x,
                    scale_coord(top_virtual, draw.height, self.frame.height),
                );
                let bottom = self.frame.pixel(
                    source_x,
                    scale_coord(bottom_virtual, draw.height, self.frame.height),
                );
                let (foreground, background) = if self.color {
                    (rgb_color(top), rgb_color(bottom))
                } else {
                    (gray_color(luminance(top)), gray_color(luminance(bottom)))
                };
                let terminal_y = area.y + offset_y / 2 + row;
                if terminal_y < area.bottom() {
                    buffer[(area.x + offset_x + col, terminal_y)]
                        .set_char('▀')
                        .set_style(Style::default().fg(foreground).bg(background));
                }
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

fn fit(area: Rect, source_width: u16, source_height: u16, cell_aspect: f32) -> (Rect, u16, u16) {
    let available_ratio = f32::from(area.width) / (f32::from(area.height.max(1)) * cell_aspect);
    let source_ratio = f32::from(source_width) / f32::from(source_height.max(1));
    let (width, height) = if source_ratio > available_ratio {
        let width = area.width;
        let height = ((f32::from(width) / source_ratio) / cell_aspect)
            .round()
            .max(1.0) as u16;
        (width, height.min(area.height))
    } else {
        let height = area.height;
        let width = (f32::from(height) * cell_aspect * source_ratio)
            .round()
            .max(1.0) as u16;
        (width.min(area.width), height)
    };
    let offset_x = area.width.saturating_sub(width) / 2;
    let offset_y = area.height.saturating_sub(height) / 2;
    (Rect::new(0, 0, width, height), offset_x, offset_y)
}

fn scale_coord(value: u16, destination_size: u16, source_size: u16) -> u16 {
    if destination_size <= 1 {
        return source_size / 2;
    }
    ((u32::from(value) * u32::from(source_size)) / u32::from(destination_size))
        .min(u32::from(source_size.saturating_sub(1))) as u16
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
        assert_eq!(scale_coord(99, 100, 192), 190);
        assert_eq!(scale_coord(0, 1, 192), 96);
    }

    #[test]
    fn black_and_white_luminance_are_stable() {
        assert_eq!(luminance([0, 0, 0]), 0);
        assert_eq!(luminance([255, 255, 255]), 255);
    }

    #[test]
    fn fitted_rectangle_never_exceeds_area() {
        let (rect, x, y) = fit(Rect::new(0, 0, 80, 20), 192, 108, 2.0);
        assert!(rect.width + x <= 80);
        assert!(rect.height + y <= 20);
    }
}
