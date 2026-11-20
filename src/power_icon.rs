use fontdue::{Font, FontSettings, Metrics};
use std::{error::Error, fs, path::PathBuf};
use tray_icon::Icon;
use windows_sys::{
    Win32::UI::{
        HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi},
        WindowsAndMessaging::{FindWindowW, SM_CXSMICON, SM_CYSMICON},
    },
    core::w,
};

fn icon_size(dpi: u32) -> (usize, usize) {
    unsafe {
        (
            GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16) as usize,
            GetSystemMetricsForDpi(SM_CYSMICON, dpi).max(16) as usize,
        )
    }
}

fn digit_size(font: &Font, width: usize, height: usize) -> usize {
    (1..=height * 2)
        .rev()
        .find(|&size| {
            let digits = ('0'..='9').map(|c| font.metrics(c, size as f32));
            let advance = digits.clone().map(|m| m.advance_width).fold(0.0, f32::max);
            let left = digits.clone().map(|m| m.xmin).min().unwrap();
            let right = digits
                .clone()
                .map(|m| m.xmin + m.width as i32)
                .max()
                .unwrap();
            let bottom = digits.clone().map(|m| m.ymin).min().unwrap();
            let top = digits.map(|m| m.ymin + m.height as i32).max().unwrap();
            // Reserve the widest possible three-digit number, independent of the reading.
            let max_width = (2.0 * advance).round() as i32 + right - left;
            max_width < width as i32 && top - bottom <= height as i32 - 2
        })
        .unwrap_or(1)
}

pub struct PowerIcon {
    font: Font,
    width: usize,
    height: usize,
    size: usize,
    color: [u8; 3],
}

impl PowerIcon {
    pub fn new(color: [u8; 3]) -> Result<Self, Box<dyn Error>> {
        let dir = PathBuf::from(std::env::var_os("WINDIR").ok_or("WINDIR не задан")?).join("Fonts");
        for name in ["segoeui.ttf", "arial.ttf"] {
            if let Ok(data) = fs::read(dir.join(name))
                && let Ok(font) = Font::from_bytes(data, FontSettings::default())
            {
                let dpi = unsafe {
                    let taskbar = FindWindowW(w!("Shell_TrayWnd"), std::ptr::null());
                    match GetDpiForWindow(taskbar) {
                        0 => GetDpiForSystem(),
                        dpi => dpi,
                    }
                };
                let (width, height) = icon_size(dpi);
                println!("Трей: {dpi} DPI, растеризация значка {width}×{height} пикселей");
                let size = digit_size(&font, width, height);
                return Ok(Self {
                    font,
                    width,
                    height,
                    size,
                    color,
                });
            }
        }
        Err("Не удалось загрузить Segoe UI или Arial".into())
    }

    pub fn render(&self, watts: Option<u32>) -> Result<Icon, tray_icon::BadIcon> {
        let rgba = self.pixels(watts);
        Icon::from_rgba(rgba, self.width as u32, self.height as u32)
    }

    fn pixels(&self, watts: Option<u32>) -> Vec<u8> {
        let text = watts.map_or_else(|| "–".into(), |w| w.to_string());
        let mut pixels = vec![0; self.width * self.height * 4];
        let [r, g, b] = self.color;
        // One size for 1–3 digits; shrink only if an unusually long reading does not fit.
        for size in (1..=self.size).rev() {
            let mut pen = 0.0_f32;
            let glyphs: Vec<(i32, Metrics, Vec<u8>)> = text
                .chars()
                .map(|c| {
                    let (m, bitmap) = self.font.rasterize(c, size as f32);
                    let left = pen.round() as i32 + m.xmin;
                    pen += m.advance_width;
                    (left, m, bitmap)
                })
                .collect();
            let left = glyphs.iter().map(|(x, _, _)| *x).min().unwrap();
            let right = glyphs
                .iter()
                .map(|(x, m, _)| *x + m.width as i32)
                .max()
                .unwrap();
            let top = glyphs
                .iter()
                .map(|(_, m, _)| m.ymin + m.height as i32)
                .max()
                .unwrap();
            let bottom = glyphs.iter().map(|(_, m, _)| m.ymin).min().unwrap();
            let w = (right - left) as usize;
            let h = (top - bottom) as usize;
            if w > self.width - 1 || h > self.height - 2 {
                continue;
            }
            let pad_x = (self.width - w) / 2;
            let pad_y = (self.height - h) / 2;
            for (x, m, bitmap) in glyphs {
                let x0 = pad_x + (x - left) as usize;
                let y0 = pad_y + (top - m.ymin - m.height as i32) as usize;
                for (row, line) in bitmap.chunks(m.width.max(1)).enumerate() {
                    for (col, &alpha) in line.iter().enumerate() {
                        let i = ((y0 + row) * self.width + x0 + col) * 4;
                        if alpha > pixels[i + 3] {
                            pixels[i..i + 4].copy_from_slice(&[r, g, b, alpha]);
                        }
                    }
                }
            }
            break;
        }
        pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_physical_pixels_at_200_percent() {
        assert_eq!(icon_size(192), (32, 32));
    }

    #[test]
    fn draws_three_digits_inside_a_16px_icon() {
        let color = [49, 170, 255];
        let mut renderer = PowerIcon::new(color).unwrap();
        renderer.width = 16;
        renderer.height = 16;
        renderer.size = digit_size(&renderer.font, 16, 16);
        for power in [None, Some(9), Some(38), Some(111), Some(450), Some(999)] {
            let pixels = renderer.pixels(power);
            assert_eq!(pixels.len(), 16 * 16 * 4);
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[..3] == color && p[3] > 0)
            );
            assert!(pixels[..64].iter().all(|&channel| channel == 0));
            assert!(pixels[15 * 64..].iter().all(|&channel| channel == 0));
        }
        let occupied_rows = |watts| {
            renderer
                .pixels(Some(watts))
                .chunks(64)
                .map(|row| row.as_chunks::<4>().0.iter().any(|p| p[3] != 0))
                .collect::<Vec<_>>()
        };
        assert_eq!(occupied_rows(88), occupied_rows(888));
    }
}
