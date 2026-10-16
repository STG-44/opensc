#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ui;

use anyhow::{Context, Result, bail};
use opensc_launcher::{
    config::{Config, default_root},
    resources, updater,
};
use std::{path::PathBuf, sync::atomic::AtomicBool};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let root = default_root();
    match command.as_deref() {
        Some("--help" | "-h") => {
            println!(
                "OpenSC Launcher {}\n\nБез аргументов: открыть лаунчер\n--check [URL]       Проверить опубликованные сборки\n--install [URL]     Установить сборку для текущей системы\n--verify            Проверить установленную сборку\n--inspect <папка>    Инвентаризация ресурсов в JSON\n--data <папка>       Открыть лаунчер с выбранной папкой ресурсов\n\nOPENSC_HOME задаёт отдельную папку настроек и установок.",
                env!("CARGO_PKG_VERSION")
            );
        }
        Some("--inspect") => {
            let path = PathBuf::from(args.next().context("Укажи папку ресурсов")?);
            println!(
                "{}",
                serde_json::to_string_pretty(&resources::inspect(&path)?)?
            );
        }
        Some("--check" | "--install") => {
            let config = Config::load(&root)?;
            let url = args.next().unwrap_or(config.manifest_url);
            match updater::check(&url)? {
                None => {
                    println!("Движок ещё не опубликован. Сервер обновлений доступен.")
                }
                Some(available) => {
                    println!(
                        "Версия: {}\nПлатформа: {}\n{}",
                        available.release.version,
                        opensc_launcher::platform(),
                        available.release.notes
                    );
                    if command.as_deref() == Some("--install") {
                        let installed =
                            updater::install(&available, &root, &AtomicBool::new(false), |p| {
                                eprintln!("{}: {} / {} байт", p.file, p.downloaded, p.total)
                            })?;
                        println!("Установлено: {}", installed.directory(&root)?.display());
                    }
                }
            }
        }
        Some("--verify") => {
            let installed = updater::Installed::load(&root)?.context("Движок ещё не установлен")?;
            installed.verify(&root)?;
            println!("Все файлы прошли проверку SHA-256");
        }
        Some("--data") | None => {
            let data = if command.is_some() {
                Some(args.next().context("Укажи папку ресурсов")?)
            } else {
                None
            };
            let icon =
                image::load_from_memory(include_bytes!("../assets/opensc.png"))?.into_rgba8();
            let icon = eframe::egui::IconData {
                width: icon.width(),
                height: icon.height(),
                rgba: icon.into_raw(),
            };
            let options = eframe::NativeOptions {
                viewport: eframe::egui::ViewportBuilder::default()
                    .with_title("OpenSC Launcher")
                    .with_app_id("opensc-launcher")
                    .with_inner_size([1100.0, 820.0])
                    .with_min_inner_size([760.0, 680.0])
                    .with_icon(icon),
                ..Default::default()
            };
            eframe::run_native(
                "OpenSC Launcher",
                options,
                Box::new(move |cc| Ok(Box::new(ui::Launcher::new(cc, root, data)))),
            )
            .map_err(|e| anyhow::anyhow!("Не удалось открыть окно: {e}"))?;
        }
        Some(other) => bail!("Неизвестный аргумент: {other}. Используй --help"),
    }
    Ok(())
}
