//! Уведомление на рабочем столе (D-Bus `org.freedesktop.Notifications`: mako, dunst, swaync…), когда рендер закончился.

use std::path::PathBuf;

/// Показывает уведомление в фоновом потоке; клик или «Открыть папку» открывает `dir` (xdg-open).
/// Нет демона уведомлений — только запись в лог.
pub fn render_done(summary: String, body: String, open_label: String, dir: Option<PathBuf>) {
    std::thread::spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.appname("ShortsCutter").summary(&summary).body(&body).icon("video-x-generic");
        if dir.is_some() {
            n.action("default", &open_label).action("open", &open_label);
        }
        match n.show() {
            Ok(handle) => {
                let Some(dir) = dir else { return };
                handle.wait_for_action(|action| {
                    if matches!(action, "default" | "open")
                        && let Err(e) = std::process::Command::new("xdg-open").arg(&dir).spawn()
                    {
                        log::warn!("xdg-open {}: {e}", dir.display());
                    }
                });
            }
            Err(e) => log::warn!("уведомление не показано: {e}"),
        }
    });
}
