use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2};
use opensc_launcher::{
    config::Config,
    resources::{self, Inventory},
    updater::{self, Available, Installed, Progress},
};
use std::{
    path::{Path, PathBuf},
    process::Child,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

const ACCENT: Color32 = Color32::from_rgb(211, 180, 100);
const MUTED: Color32 = Color32::from_rgb(149, 160, 154);
const PANEL: Color32 = Color32::from_rgb(24, 32, 29);

enum Event {
    Checked(Result<Option<Available>, String>),
    Inspected(Result<Inventory, String>),
    Progress(Progress),
    Installed(Result<Installed, String>),
    Verified(Result<(), String>),
}

pub struct Launcher {
    root: PathBuf,
    config: Config,
    installed: Option<Installed>,
    available: Option<Available>,
    inventory: Option<Inventory>,
    receiver: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    status: String,
    error: Option<String>,
    progress: Option<Progress>,
    activity: Vec<String>,
    settings: bool,
    child: Option<Child>,
    inspected_path: Option<String>,
}

impl Launcher {
    pub fn new(cc: &eframe::CreationContext<'_>, root: PathBuf, data: Option<String>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(16, 23, 20);
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = Color32::from_rgb(12, 18, 15);
        visuals.selection.bg_fill = Color32::from_rgb(82, 79, 44);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(40, 50, 44);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(57, 68, 54);
        cc.egui_ctx.set_visuals(visuals);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(12.0, 10.0);
        style.spacing.button_padding = Vec2::new(16.0, 11.0);
        cc.egui_ctx.set_style(style);
        let loaded = Config::load(&root);
        let error = loaded.as_ref().err().map(|e| format!("{e:#}"));
        let mut config = loaded.unwrap_or_default();
        if let Some(path) = data {
            config.data_path = path;
        }
        let installed = Installed::load(&root);
        let error = error.or_else(|| installed.as_ref().err().map(|e| format!("{e:#}")));
        let mut launcher = Self {
            root,
            config,
            installed: installed.unwrap_or_default(),
            available: None,
            inventory: None,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            status: "Проверка обновлений…".into(),
            error,
            progress: None,
            activity: Vec::new(),
            settings: false,
            child: None,
            inspected_path: None,
        };
        launcher.check(&cc.egui_ctx);
        launcher
    }

    fn busy(&self) -> bool {
        self.receiver.is_some()
    }

    fn save(&mut self) {
        if let Err(e) = self.config.save(&self.root) {
            self.error = Some(format!("{e:#}"));
        }
    }

    fn check(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        self.available = None;
        self.error = None;
        self.status = "Проверка обновлений…".into();
        let source = self.config.manifest_url.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = updater::check(&source).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Checked(result));
            ctx.request_repaint();
        });
    }

    fn inspect(&mut self, ctx: &egui::Context) {
        if self.busy() || self.config.data_path.trim().is_empty() {
            return;
        }
        self.config.data_path = resources::resolve_data(Path::new(&self.config.data_path))
            .to_string_lossy()
            .into_owned();
        self.save();
        self.inventory = None;
        self.error = None;
        self.status = "Проверка папки ресурсов…".into();
        let path = self.config.data_path.clone();
        self.inspected_path = Some(path.clone());
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = resources::inspect(Path::new(&path)).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Inspected(result));
            ctx.request_repaint();
        });
    }

    fn install(&mut self, ctx: &egui::Context) {
        let Some(available) = self.available.clone() else {
            return;
        };
        self.error = None;
        self.cancel.store(false, Ordering::Relaxed);
        let cancel = self.cancel.clone();
        let root = self.root.clone();
        self.status = "Загрузка и проверка файлов…".into();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = updater::install(&available, &root, &cancel, |progress| {
                let _ = tx.send(Event::Progress(progress));
                ctx.request_repaint();
            })
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Installed(result));
            ctx.request_repaint();
        });
    }

    fn verify(&mut self, ctx: &egui::Context) {
        let Some(installed) = self.installed.clone() else {
            return;
        };
        self.error = None;
        self.status = "Проверка установленных файлов…".into();
        let root = self.root.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = installed.verify(&root).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Verified(result));
            ctx.request_repaint();
        });
    }

    fn receive(&mut self, ctx: &egui::Context) {
        let events: Vec<_> = self
            .receiver
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            if let Event::Progress(progress) = event {
                self.progress = Some(progress);
                continue;
            }
            self.receiver = None;
            self.progress = None;
            match event {
                Event::Checked(Ok(Some(available))) => {
                    self.status = if updater::supported(&available) {
                        format!("Доступна версия {}", available.release.version)
                    } else {
                        "Для этой системы сборка ещё не опубликована".into()
                    };
                    self.available = Some(available);
                }
                Event::Checked(Ok(None)) => {
                    self.status = "Сервер доступен. Сборки ещё не опубликованы".into()
                }
                Event::Inspected(Ok(inventory)) => {
                    self.status = format!("В папке ресурсов {} файлов", inventory.files);
                    self.inventory = Some(inventory);
                }
                Event::Installed(Ok(installed)) => {
                    self.status = format!("Версия {} установлена", installed.release.version);
                    self.installed = Some(installed);
                }
                Event::Verified(Ok(())) => self.status = "Все файлы прошли проверку SHA-256".into(),
                Event::Checked(Err(error))
                | Event::Inspected(Err(error))
                | Event::Installed(Err(error))
                | Event::Verified(Err(error)) => {
                    self.status = "Не удалось завершить операцию".into();
                    self.error = Some(error);
                }
                Event::Progress(_) => unreachable!(),
            }
            self.activity
                .push(self.error.as_ref().unwrap_or(&self.status).clone());
            if self.activity.len() > 20 {
                self.activity.remove(0);
            }
        }
        if self.inventory.is_none()
            && !self.busy()
            && self.inspected_path.as_deref() != Some(self.config.data_path.as_str())
            && !self.config.data_path.trim().is_empty()
        {
            self.inspect(ctx);
        }
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.activity.push(format!("Движок завершился: {status}"));
                    self.child = None;
                }
                Err(e) => {
                    self.error = Some(e.to_string());
                    self.child = None;
                }
                _ => {}
            }
        }
    }

    fn resource_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            eyebrow(ui, "02 / РЕСУРСЫ ИГРЫ");
            ui.label(RichText::new("Папка оригинальной Shadow Company").size(19.0));
            ui.label(
                RichText::new("Выбери папку Data или используй ресурсы из установленной сборки.")
                    .color(MUTED),
            );
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.busy(), |ui| {
                    let width = (ui.available_width() - 145.0).max(200.0);
                    if ui
                        .add_sized(
                            [width, 36.0],
                            egui::TextEdit::singleline(&mut self.config.data_path)
                                .hint_text("Путь к папке Data"),
                        )
                        .changed()
                    {
                        self.inventory = None;
                    }
                    if ui.button("Выбрать…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title("Ресурсы Shadow Company")
                            .pick_folder()
                    {
                        self.config.data_path = path.to_string_lossy().into_owned();
                        self.inventory = None;
                        self.inspected_path = None;
                        self.inspect(ctx);
                    }
                });
            });
            if let Some(inventory) = &self.inventory {
                ui.label(
                    RichText::new(format!(
                        "{} файлов  ·  {}  ·  папка доступна",
                        inventory.files,
                        bytes(inventory.bytes)
                    ))
                    .color(ACCENT),
                );
                if !inventory.case_collisions.is_empty() {
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        format!(
                            "Совпадений имён без учёта регистра: {}",
                            inventory.case_collisions.len()
                        ),
                    );
                }
                ui.label(
                    RichText::new("Совместимость ресурсов проверим после подключения движка.")
                        .small()
                        .color(MUTED),
                );
            } else if !self.config.data_path.is_empty() && !self.busy() {
                if ui.small_button("Проверить папку").clicked() {
                    self.inspect(ctx);
                }
            } else if let Some(path) = self.bundled_data() {
                ui.label(
                    RichText::new(format!("Ресурсы установлены с сервера: {}", path.display()))
                        .color(ACCENT),
                );
            }
        });
    }
}

impl Launcher {
    fn bundled_data(&self) -> Option<PathBuf> {
        let installed = self.installed.as_ref()?;
        let data = installed.release.data_directory.as_ref()?;
        let path = installed.directory(&self.root).ok()?.join(data);
        path.is_dir().then_some(path)
    }

    fn data_path(&self) -> Option<PathBuf> {
        if self.config.data_path.is_empty() {
            self.bundled_data()
        } else {
            self.inventory.as_ref().map(|i| i.root.clone())
        }
    }
}

impl eframe::App for Launcher {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive(ctx);
        if self.busy() || self.child.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        egui::TopBottomPanel::bottom("footer")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(16))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("ЛАУНЧЕР {}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(MUTED),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(opensc_launcher::platform())
                            .small()
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.link("Настройки").clicked() {
                            self.settings = !self.settings;
                        }
                        ui.hyperlink_to("Сайт проекта", "https://shadow-test.zaikinagroup.ru/");
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(32))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    eyebrow(ui, "OPENSC / OPEN SHADOW COMPANY");
                    ui.label(
                        RichText::new("Вернуться в строй.")
                            .font(FontId::proportional(38.0))
                            .strong()
                            .color(Color32::from_rgb(237, 239, 224)),
                    );
                    ui.label(
                        RichText::new("Установка, обновления и запуск нового движка")
                            .size(16.0)
                            .color(MUTED),
                    );
                    ui.add_space(14.0);
                    card(ui, |ui| {
                        eyebrow(ui, "01 / ДВИЖОК");
                        let title = self
                            .installed
                            .as_ref()
                            .map(|i| format!("Установлена версия {}", i.release.version))
                            .unwrap_or_else(|| "Движок ещё не подключён".into());
                        ui.label(RichText::new(title).size(22.0).strong());
                        if self.installed.is_none() {
                            ui.label(
                                RichText::new(
                                    "После публикации сборки её можно будет установить здесь.",
                                )
                                .color(MUTED),
                            );
                        }
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            let can_launch = !self.busy()
                                && self.installed.is_some()
                                && self.data_path().is_some()
                                && self.child.is_none();
                            if ui
                                .add_enabled(
                                    can_launch,
                                    egui::Button::new(
                                        RichText::new(if self.child.is_some() {
                                            "Игра запущена"
                                        } else {
                                            "Запустить игру"
                                        })
                                        .strong()
                                        .color(Color32::from_rgb(20, 27, 21)),
                                    )
                                    .fill(ACCENT),
                                )
                                .clicked()
                            {
                                self.save();
                                match self
                                    .installed
                                    .as_ref()
                                    .unwrap()
                                    .launch(&self.root, &self.data_path().unwrap())
                                {
                                    Ok(child) => {
                                        self.child = Some(child);
                                        self.status = "Движок запущен".into();
                                    }
                                    Err(error) => self.error = Some(format!("{error:#}")),
                                }
                            }
                            let can_install = !self.busy()
                                && self.available.as_ref().is_some_and(updater::supported);
                            if ui
                                .add_enabled(
                                    can_install,
                                    egui::Button::new("Установить / обновить"),
                                )
                                .clicked()
                            {
                                self.install(ctx);
                            }
                            if ui
                                .add_enabled(
                                    !self.busy(),
                                    egui::Button::new("Проверить обновления"),
                                )
                                .clicked()
                            {
                                self.check(ctx);
                            }
                        });
                        if let Some(available) = &self.available {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!(
                                    "Доступная версия: {}",
                                    available.release.version
                                ))
                                .color(ACCENT),
                            );
                            ui.label(&available.release.notes);
                        }
                    });
                    ui.add_space(6.0);
                    self.resource_card(ui, ctx);
                    ui.add_space(6.0);
                    card(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if self.busy() {
                                ui.spinner();
                            }
                            ui.label(RichText::new(&self.status).color(ACCENT));
                        });
                        if let Some(progress) = &self.progress {
                            let fraction = if progress.total > 0 {
                                progress.downloaded as f32 / progress.total as f32
                            } else {
                                1.0
                            };
                            ui.add(egui::ProgressBar::new(fraction).text(format!(
                                "{} / {}",
                                bytes(progress.downloaded),
                                bytes(progress.total)
                            )));
                            ui.label(RichText::new(&progress.file).small().color(MUTED));
                            if ui.small_button("Отменить установку").clicked() {
                                self.cancel.store(true, Ordering::Relaxed);
                            }
                        }
                        if let Some(error) = &self.error {
                            ui.colored_label(Color32::from_rgb(240, 144, 127), error);
                        }
                        egui::CollapsingHeader::new("Журнал действий").show(ui, |ui| {
                            for line in self.activity.iter().rev().take(8) {
                                ui.label(RichText::new(line).small().color(MUTED));
                            }
                        });
                    });
                });
            });
        if self.settings {
            let mut open = true;
            egui::Window::new("Настройки лаунчера")
                .open(&mut open)
                .default_width(660.0)
                .show(ctx, |ui| {
                    ui.label("Адрес манифеста обновлений");
                    ui.add_enabled(
                        !self.busy(),
                        egui::TextEdit::singleline(&mut self.config.manifest_url)
                            .desired_width(f32::INFINITY),
                    );
                    ui.label(
                        RichText::new("Выпуски и пакеты ресурсов загружаются с этого сервера.")
                            .small()
                            .color(MUTED),
                    );
                    ui.add_space(8.0);
                    ui.label("Папка установок и журналов");
                    ui.label(
                        RichText::new(self.root.display().to_string())
                            .monospace()
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Сохранить").clicked() {
                            self.save();
                        }
                        if ui
                            .add_enabled(
                                !self.busy() && self.installed.is_some(),
                                egui::Button::new("Проверить файлы"),
                            )
                            .clicked()
                        {
                            self.verify(ctx);
                        }
                    });
                });
            self.settings = open;
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.save();
    }
}

fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).strong().color(ACCENT));
}

fn card(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, Color32::from_rgb(49, 61, 51)))
        .corner_radius(8)
        .inner_margin(20)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            contents(ui);
        });
}

fn bytes(value: u64) -> String {
    if value >= 1024 * 1024 * 1024 {
        format!("{:.2} ГБ", value as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if value >= 1024 * 1024 {
        format!("{:.1} МБ", value as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} КБ", value as f64 / 1024.0)
    }
}
