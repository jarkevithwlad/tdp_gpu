#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod gpu;
mod power_icon;
mod settings;

use std::{error::Error, io, ptr::null_mut, sync::mpsc, thread};
use tray_icon::{
    TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MB_ICONERROR, MSG, MessageBoxW, MsgWaitForMultipleObjects, PM_REMOVE,
    PeekMessageW, QS_ALLINPUT, TranslateMessage, WM_QUIT,
};

const LIMITS: [u32; 9] = [150, 175, 200, 250, 300, 350, 400, 420, 450];

fn main() {
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    // Set this before creating any window (including the tray's hidden window).
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if let Err(error) = run() {
        eprintln!("{error}");
        let text: Vec<_> = error.to_string().encode_utf16().chain([0]).collect();
        let title: Vec<_> = "RTX 3090 Power".encode_utf16().chain([0]).collect();
        unsafe { MessageBoxW(null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONERROR) };
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let settings_path = std::env::current_exe()?.with_file_name("tdp_gpu.txt");
    let settings = settings::Settings::load(&settings_path);
    let (request_tx, request_rx) = mpsc::channel();
    let (update_tx, update_rx) = mpsc::channel();
    let gpu = gpu::Gpu::find()?;
    let mut limit = None;
    let mut power = None;
    let mut busy = true;
    let mut status = String::new();

    let menu = Menu::new();
    let mut items = Vec::new();
    for watts in LIMITS.into_iter().rev() {
        let item = CheckMenuItem::new(format!("{watts} Вт"), false, false, None);
        menu.append(&item)?;
        items.push((watts, item));
    }
    menu.append(&PredefinedMenuItem::separator())?;
    let exit = MenuItem::new("Выход", true, None);
    menu.append(&exit)?;

    let font = power_icon::PowerIcon::new(settings.color)?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("RTX 3090: чтение данных…")
        .with_icon(font.render(None)?)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true)
        .build()?;
    thread::spawn(move || gpu::monitor(gpu, settings_path, settings, request_rx, update_tx));
    println!("RTX 3090: ПКМ или ЛКМ по значку — меню. Мониторинг каждые 2 секунды.");

    loop {
        if !pump_messages() {
            return Ok(());
        }
        let mut changed = false;
        loop {
            match update_rx.try_recv() {
                Ok(gpu::Update::Limit { actual, message }) => {
                    limit = actual;
                    busy = false;
                    status = message;
                    update_checks(&items, limit, busy);
                    changed = true;
                }
                Ok(gpu::Update::Power(new)) => {
                    if power != new {
                        power = new;
                        let icon = font.render(power)?;
                        if let Err(error) = tray.set_icon(Some(icon)) {
                            eprintln!("Ошибка обновления значка: {error}");
                        }
                        changed = true;
                    }
                    println!(
                        "Потребление: {} | лимит: {}",
                        display_watts(power),
                        display_watts(limit)
                    );
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("Поток мониторинга завершился".into());
                }
            }
        }
        if changed
            && let Err(error) = tray.set_tooltip(Some(format!(
                "RTX 3090: {} | лимит: {}\n{status}",
                display_watts(power),
                display_watts(limit)
            )))
        {
            eprintln!("Ошибка обновления подсказки: {error}");
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id() == exit.id() {
                return Ok(());
            }
            if let Some((watts, _)) = items.iter().find(|(_, item)| event.id() == item.id()) {
                if !busy {
                    request_tx.send(*watts)?;
                    busy = true;
                }
                update_checks(&items, limit, busy);
            }
        }
        // Wake immediately for tray messages and periodically for worker replies.
        if unsafe { MsgWaitForMultipleObjects(0, null_mut(), 0, 40, QS_ALLINPUT) } == u32::MAX {
            return Err(io::Error::last_os_error().into());
        }
    }
}

fn pump_messages() -> bool {
    let mut message = MSG::default();
    unsafe {
        while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
            if message.message == WM_QUIT {
                return false;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    true
}

fn update_checks(items: &[(u32, CheckMenuItem)], limit: Option<u32>, busy: bool) {
    for (watts, item) in items {
        item.set_checked(Some(*watts) == limit);
        item.set_enabled(!busy);
    }
}

fn display_watts(watts: Option<u32>) -> String {
    watts.map_or_else(|| "нет данных".into(), |w| format!("{w} Вт"))
}
