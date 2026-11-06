#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod report_ui;
mod ui;

use anyhow::{Context, Result, bail};
use opensc_launcher::{
    config::{Config, default_root},
    importer, resources, self_update, updater,
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
                "OpenSC Launcher {}\n\nБез аргументов: открыть лаунчер\n--check [URL]       Проверить опубликованные сборки\n--install [URL]     Установить сборку для текущей системы\n--verify            Проверить установленную сборку\n--verify-launcher-index FILE [URL]  Проверить подпись индекса лаунчера\n--launch [папка]     Запустить установленную игру без окна лаунчера\n--inspect <папка>    Инвентаризация ресурсов в JSON\n--import <источник>  Импорт игры из папки или MDF/MDS/ISO\n--data <папка>       Открыть лаунчер с выбранной папкой ресурсов\n\nOPENSC_HOME задаёт отдельную папку настроек и установок.",
                env!("CARGO_PKG_VERSION")
            );
        }
        Some("--verify-launcher-index") => {
            use std::io::Read;
            let path = args.next().context("Укажи файл подписанного индекса")?;
            let source = args.next().unwrap_or_else(|| {
                "https://shadow-test.zaikinagroup.ru/updates/launcher-v1.json".into()
            });
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(256 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            let available = self_update::verify_index(&bytes, &url::Url::parse(&source)?)?;
            println!(
                "Подпись индекса верна. Лаунчер {}. Платформ: {}",
                available.release.version,
                available.release.entrypoints.len()
            );
        }
        Some("--import") => {
            let source = PathBuf::from(args.next().context("Укажи папку игры, MDF/MDS или ISO")?);
            let inventory =
                importer::import(&root, &source, &AtomicBool::new(false), |phase, p| {
                    eprintln!("{phase}: {} / {}", p.downloaded, p.total)
                })?;
            let mut config = Config::load(&root)?;
            config.data_path = inventory.root.to_string_lossy().into_owned();
            config.save(&root)?;
            println!(
                "Игровые файлы скопированы и проверены: {}",
                inventory.root.display()
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
        Some("--launch") => {
            let config = Config::load(&root)?;
            let data = args.next().unwrap_or(config.data_path);
            anyhow::ensure!(
                !data.trim().is_empty(),
                "Укажи --launch PATH или папку в настройках"
            );
            let mut data_path = resources::resolve_data(&PathBuf::from(data));
            if !importer::is_managed(&root, &data_path) {
                let inventory =
                    importer::import(&root, &data_path, &AtomicBool::new(false), |phase, p| {
                        eprintln!("{phase}: {} / {}", p.downloaded, p.total)
                    })?;
                data_path = inventory.root;
                let mut saved = Config::load(&root)?;
                saved.data_path = data_path.to_string_lossy().into_owned();
                saved.save(&root)?;
            }
            let available = updater::check(&config.manifest_url)?
                .context("В выбранном канале пока нет сборки")?;
            let installed = updater::Installed::load(&root)?;
            let installed = if let Some(installed) = installed
                && installed.release.fingerprint()? == available.release.fingerprint()?
            {
                installed
            } else {
                updater::install(&available, &root, &AtomicBool::new(false), |p| {
                    eprintln!("{}: {} / {} байт", p.file, p.downloaded, p.total)
                })?
            };
            let status = installed.launch(&root, &data_path)?.wait()?;
            anyhow::ensure!(
                status.success(),
                "Игра завершилась с ошибкой: {status}; см. engine.log"
            );
        }
        Some("--data") | None => {
            let data = if command.is_some() {
                Some(args.next().context("Укажи папку ресурсов")?)
            } else {
                None
            };
            // An installed bootstrap must not read or rewrite a newer profile.
            if let Ok(Some(prepared)) = self_update::cached(&root) {
                let forwarded = data
                    .as_deref()
                    .map(|path| vec!["--data", path])
                    .unwrap_or_default();
                match prepared.start_with_args(&root, &forwarded) {
                    Ok(mut handoff) => loop {
                        match handoff.poll() {
                            Ok(true) => return Ok(()),
                            Ok(false) => std::thread::sleep(std::time::Duration::from_millis(25)),
                            Err(error) => {
                                handoff.cancel();
                                eprintln!("Cached launcher did not start: {error:#}");
                                break;
                            }
                        }
                    },
                    Err(error) => eprintln!("Cached launcher did not start: {error:#}"),
                }
            }
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
                    .with_inner_size([1280.0, 760.0])
                    .with_min_inner_size([960.0, 640.0])
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
