use crate::settings::Settings;
use std::{
    os::windows::process::CommandExt,
    path::PathBuf,
    process::Command,
    sync::mpsc::{Receiver, RecvTimeoutError, Sender},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

pub struct Gpu(String);

pub enum Update {
    Power(Option<u32>),
    Limit {
        actual: Option<u32>,
        message: String,
    },
}

impl Gpu {
    pub fn find() -> Result<Self, String> {
        let output = smi(&["--query-gpu=name,uuid", "--format=csv,noheader"])?;
        output
            .lines()
            .filter_map(|line| line.split_once(','))
            .find_map(|(name, uuid)| {
                (name.trim() == "NVIDIA GeForce RTX 3090" && uuid.trim().starts_with("GPU-"))
                    .then(|| Self(uuid.trim().into()))
            })
            .ok_or_else(|| "NVIDIA GeForce RTX 3090 не найдена".into())
    }

    fn watts(&self, field: &str) -> Result<u32, String> {
        let value = smi(&[
            "-i",
            &self.0,
            "--query-gpu",
            field,
            "--format=csv,noheader,nounits",
        ])?;
        parse_watts(&value).ok_or_else(|| format!("{field}: {value}"))
    }

    fn set_limit(&self, watts: u32) -> Result<(), String> {
        smi(&["-i", &self.0, "-pl", &watts.to_string()])?;
        let actual = self.watts("power.limit")?;
        if actual != watts {
            return Err(format!("драйвер сообщает {actual} Вт вместо {watts} Вт"));
        }
        Ok(())
    }
}

fn parse_watts(text: &str) -> Option<u32> {
    let value: f64 = text.trim().parse().ok()?;
    (0.0..=u32::MAX as f64)
        .contains(&value)
        .then(|| value.round() as u32)
}

fn smi(args: &[&str]) -> Result<String, String> {
    let out = Command::new("nvidia-smi.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("Не удалось запустить nvidia-smi: {error}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success() {
        return Ok(stdout.trim().into());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let details = [stdout.trim(), stderr.trim()].join(" ");
    Err(format!("nvidia-smi ({}): {}", out.status, details.trim()))
}

fn failure(error: &str) -> &'static str {
    if error.contains("Insufficient Permissions") {
        "Запустите от имени администратора"
    } else {
        "Не удалось применить лимит (см. консоль)"
    }
}

pub fn monitor(
    gpu: Gpu,
    path: PathBuf,
    settings: Settings,
    requests: Receiver<u32>,
    updates: Sender<Update>,
) {
    let read_limit = || {
        gpu.watts("power.limit")
            .map_err(|err| eprintln!("Лимит не прочитан: {err}"))
            .ok()
    };
    println!("GPU: {} | Настройки: {}", gpu.0, path.display());
    let (mut actual, message) = match settings.limit {
        Some(saved) => match gpu.set_limit(saved) {
            Ok(()) => (Some(saved), "Лимит восстановлен из файла"),
            Err(error) => {
                eprintln!("Не удалось восстановить {saved} Вт: {error}");
                (read_limit(), failure(&error))
            }
        },
        None => (read_limit(), "Сохранённого лимита нет — лимит считан с GPU"),
    };
    println!(
        "Лимит при запуске: {} | {message}",
        crate::display_watts(actual)
    );
    if updates
        .send(Update::Limit {
            actual,
            message: message.into(),
        })
        .is_err()
    {
        return;
    }
    let mut next_sample = Instant::now();
    loop {
        match requests.recv_timeout(next_sample.saturating_duration_since(Instant::now())) {
            Ok(watts) => {
                let message = match gpu.set_limit(watts) {
                    Ok(()) => {
                        actual = Some(watts);
                        match settings.save_limit(&path, watts) {
                            Ok(()) => "Лимит применён и сохранён",
                            Err(error) => {
                                eprintln!("Не удалось записать {}: {error}", path.display());
                                "Лимит применён, но файл не сохранён"
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("Не удалось установить {watts} Вт: {error}");
                        actual = read_limit();
                        failure(&error)
                    }
                };
                println!("Лимит: {} | {message}", crate::display_watts(actual));
                if updates
                    .send(Update::Limit {
                        actual,
                        message: message.into(),
                    })
                    .is_err()
                {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                let draw = gpu
                    .watts("power.draw.instant")
                    .or_else(|_| gpu.watts("power.draw"))
                    .map_err(|err| eprintln!("Потребление не прочитано: {err}"))
                    .ok();
                if updates.send(Update::Power(draw)).is_err() {
                    break;
                }
                next_sample = Instant::now() + Duration::from_secs(2);
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_driver_watts() {
        assert_eq!(parse_watts(" 420.00\r\n"), Some(420));
        assert_eq!(parse_watts("29.57"), Some(30));
        assert_eq!(parse_watts("N/A"), None);
        assert_eq!(parse_watts("32\n54"), None);
    }
}
