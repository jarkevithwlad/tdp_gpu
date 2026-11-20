use crate::LIMITS;
use std::{fs, io, path::Path};

pub const DEFAULT_COLOR: [u8; 3] = [40, 245, 70];

pub struct Settings {
    pub limit: Option<u32>,
    pub color: [u8; 3],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            limit: None,
            color: DEFAULT_COLOR,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    eprintln!("Не удалось прочитать {}: {error}", path.display());
                }
                Self::default()
            }
        }
    }

    fn parse(text: &str) -> Self {
        let text = text.trim_start_matches('\u{feff}').trim();
        let mut settings = Self::default();
        for line in text.lines().map(str::trim) {
            match line.split_once('=') {
                Some((key, value)) if key.trim().eq_ignore_ascii_case("color") => {
                    if let Some(rgb) = parse_color(value.trim()) {
                        settings.color = rgb;
                    } else {
                        eprintln!("Некорректный цвет: {value:?}; формат — #RRGGBB");
                    }
                }
                Some((key, value)) if key.trim().eq_ignore_ascii_case("limit") => {
                    settings.limit = parse_limit(value.trim());
                }
                // Older versions stored just the number on its own line.
                None if line.as_bytes().first().is_some_and(u8::is_ascii_digit) => {
                    settings.limit = parse_limit(line);
                }
                _ => {}
            }
        }
        settings
    }

    pub fn save_limit(&self, path: &Path, limit: u32) -> io::Result<()> {
        let [r, g, b] = self.color;
        fs::write(
            path,
            format!("limit={limit}\ncolor=#{r:02X}{g:02X}{b:02X}\n"),
        )
    }
}

fn parse_limit(text: &str) -> Option<u32> {
    let limit = text.parse().ok().filter(|w| LIMITS.contains(w));
    if limit.is_none() {
        eprintln!("Некорректный лимит в файле настроек: {text:?}");
    }
    limit
}

fn parse_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_legacy_and_new_settings() {
        assert_eq!(Settings::parse("250\r\n").limit, Some(250));
        let settings = Settings::parse("\u{feff}limit=250\r\ncolor=#31aaff");
        assert_eq!(settings.limit, Some(250));
        assert_eq!(settings.color, [49, 170, 255]);
        let settings = Settings::parse("color=#ffffff");
        assert_eq!(settings.limit, None);
        assert_eq!(settings.color, [255; 3]);
        assert_eq!(
            Settings::parse("limit=300\ncolor=#oops").color,
            DEFAULT_COLOR
        );
    }
}
