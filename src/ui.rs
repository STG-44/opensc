use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use opensc_launcher::{
    config::Config,
    resources::{self, Inventory},
    self_update,
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
    time::{Duration, Instant},
};

const ACCENT: Color32 = Color32::from_rgb(224, 167, 83);
const PLAY: Color32 = Color32::from_rgb(190, 65, 31);
const CHECK_INTERVAL: Duration = Duration::from_secs(60);
const MUTED: Color32 = Color32::from_rgb(161, 173, 163);
const PANEL: Color32 = Color32::from_rgb(24, 31, 26);
const BACKGROUND: Color32 = Color32::from_rgb(13, 18, 15);

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
    hero: Option<egui::TextureHandle>,
    next_check: Instant,
    blocked_update: Option<String>,
    download_started: Option<Instant>,
    reports: crate::report_ui::Form,
    launcher_receiver: Option<Receiver<Result<Option<self_update::Prepared>, String>>>,
    launcher_ready: Option<self_update::Prepared>,
    launcher_handoff: Option<self_update::Handoff>,
    launcher_status: String,
    next_launcher_check: Instant,
}

impl Launcher {
    pub fn new(cc: &eframe::CreationContext<'_>, root: PathBuf, data: Option<String>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BACKGROUND;
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = Color32::from_rgb(12, 18, 15);
        visuals.selection.bg_fill = Color32::from_rgb(82, 79, 44);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(40, 50, 44);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(57, 68, 54);
        visuals.override_text_color = Some(Color32::from_rgb(237, 237, 222));
        visuals.hyperlink_color = ACCENT;
        visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(4);
        visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(4);
        visuals.widgets.active.corner_radius = egui::CornerRadius::same(4);
        cc.egui_ctx.set_visuals(visuals);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(12.0, 10.0);
        style.spacing.button_padding = Vec2::new(16.0, 11.0);
        cc.egui_ctx.set_style(style);
        let hero =
            image::load_from_memory(include_bytes!("../deploy/site/assets/operation-map.jpg"))
                .ok()
                .map(|image| {
                    let image = image.into_rgba8();
                    let size = [image.width() as usize, image.height() as usize];
                    cc.egui_ctx.load_texture(
                        "operation-map",
                        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                        egui::TextureOptions::LINEAR,
                    )
                });
        let loaded = Config::load(&root);
        let error = loaded.as_ref().err().map(|e| format!("{e:#}"));
        let mut config = loaded.unwrap_or_default();
        if let Some(path) = data {
            config.data_path = path;
        }
        let installed = Installed::load(&root);
        let error = error.or_else(|| installed.as_ref().err().map(|e| format!("{e:#}")));
        let reports = crate::report_ui::Form::load(&root);
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
            hero,
            next_check: Instant::now() + CHECK_INTERVAL,
            blocked_update: None,
            download_started: None,
            reports,
            launcher_receiver: None,
            launcher_ready: None,
            launcher_handoff: None,
            launcher_status: String::new(),
            next_launcher_check: Instant::now(),
        };
        launcher.check(&cc.egui_ctx);
        launcher.check_launcher(&cc.egui_ctx);
        if let Err(error) = self_update::signal_ready(&launcher.root) {
            launcher
                .activity
                .push(format!("Подтверждение запуска: {error:#}"));
        }
        launcher
    }

    fn check_launcher(&mut self, ctx: &egui::Context) {
        if self.launcher_receiver.is_some()
            || self.launcher_handoff.is_some()
            || self.busy()
            || !self.config.auto_update_launcher
        {
            return;
        }
        self.next_launcher_check = Instant::now() + Duration::from_secs(3600);
        self.launcher_status = "Проверка обновления лаунчера…".into();
        let (tx, rx) = mpsc::channel();
        self.launcher_receiver = Some(rx);
        let (root, source, ctx) = (
            self.root.clone(),
            self.config.manifest_url.clone(),
            ctx.clone(),
        );
        std::thread::spawn(move || {
            let result = self_update::prepare(&root, &source).map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    fn poll_launcher(&mut self, ctx: &egui::Context) {
        if let Some(result) = self
            .launcher_receiver
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.launcher_receiver = None;
            match result {
                Ok(Some(prepared)) => {
                    self.launcher_status = format!("Лаунчер {} готов к запуску", prepared.version);
                    self.launcher_ready = Some(prepared);
                }
                Ok(None) => {
                    self.launcher_status =
                        format!("Лаунчер {} · проверка завершена", env!("CARGO_PKG_VERSION"))
                }
                Err(error) => {
                    self.launcher_status =
                        "Обновление лаунчера отложено; текущая версия работает".into();
                    self.activity.push(error);
                }
            }
        }
        if let Some(handoff) = &mut self.launcher_handoff {
            match handoff.poll() {
                Ok(true) => {
                    self.launcher_handoff = None;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Ok(false) => {}
                Err(error) => {
                    handoff.cancel();
                    self.launcher_handoff = None;
                    self.launcher_status = format!("{error:#}");
                }
            }
        } else if self.config.auto_update_launcher
            && !self.busy()
            && self.child.is_none()
            && !self.settings
            && self.reports.can_restart()
            && let Some(prepared) = self.launcher_ready.take()
        {
            self.save();
            match prepared.start(&self.root) {
                Ok(handoff) => {
                    self.launcher_status = "Открывается новая версия лаунчера…".into();
                    self.launcher_handoff = Some(handoff);
                }
                Err(error) => self.launcher_status = format!("{error:#}"),
            }
        }
        if self.config.auto_update_launcher && Instant::now() >= self.next_launcher_check {
            self.check_launcher(ctx);
        }
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
        self.next_check = Instant::now() + CHECK_INTERVAL;
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
        self.download_started = Some(Instant::now());
        self.progress = Some(Progress {
            file: "Подключение к серверу…".into(),
            downloaded: 0,
            total: available
                .release
                .files_for(&opensc_launcher::platform())
                .map(|f| f.size)
                .sum(),
        });
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
            self.download_started = None;
            match event {
                Event::Checked(Ok(Some(available))) => {
                    self.status = if !updater::supported(&available) {
                        "Для этой системы сборка ещё не опубликована".into()
                    } else if self.needs_update(&available) {
                        format!("Доступна версия {}", available.release.version)
                    } else {
                        "Установлена последняя версия. Можно играть".into()
                    };
                    self.available = Some(available);
                }
                Event::Checked(Ok(None)) => {
                    self.status = "Сервер доступен. Движок ещё не опубликован".into()
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
                | Event::Verified(Err(error)) => {
                    self.status = "Не удалось завершить операцию".into();
                    self.error = Some(error);
                }
                Event::Installed(Err(error)) => {
                    self.blocked_update = self
                        .available
                        .as_ref()
                        .and_then(|a| a.release.fingerprint().ok());
                    if self.cancel.load(Ordering::Relaxed) {
                        self.status = "Загрузка остановлена. Можно начать её заново".into();
                    } else {
                        self.status = "Загрузка прервана. Нажми «Повторить»".into();
                        self.error = Some(error);
                    }
                }
                Event::Progress(_) => unreachable!(),
            }
            self.activity
                .push(self.error.as_ref().unwrap_or(&self.status).clone());
            if self.activity.len() > 20 {
                self.activity.remove(0);
            }
        }
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.activity.push(format!("Движок завершился: {status}"));
                    if !status.success() {
                        self.reports
                            .crash(&self.root, &self.config, self.installed.as_ref());
                    }
                    self.child = None;
                }
                Err(e) => {
                    self.error = Some(e.to_string());
                    self.child = None;
                }
                _ => {}
            }
        }
        self.auto_install(ctx);
        if self.inventory.is_none()
            && !self.busy()
            && self.inspected_path.as_deref() != Some(self.config.data_path.as_str())
            && !self.config.data_path.trim().is_empty()
        {
            self.inspect(ctx);
        }
    }

    fn needs_update(&self, available: &Available) -> bool {
        updater::supported(available)
            && self.installed.as_ref().is_none_or(|installed| {
                installed.release.fingerprint().ok() != available.release.fingerprint().ok()
            })
    }

    fn auto_install(&mut self, ctx: &egui::Context) {
        if !self.busy()
            && self.child.is_none()
            && self.config.auto_update
            && self.available.as_ref().is_some_and(|available| {
                self.needs_update(available)
                    && self.blocked_update != available.release.fingerprint().ok()
            })
        {
            self.install(ctx);
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        self.receive(ctx);
        if !self.busy() && Instant::now() >= self.next_check {
            self.check(ctx);
        }
    }

    fn resource_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            eyebrow(ui, "02 / РЕСУРСЫ ИГРЫ");
            ui.label(RichText::new("Папка оригинальной Shadow Company").size(19.0));
            ui.label(
                RichText::new(
                    "Укажи папку Data из своей копии игры. Выбор сохраняется при обновлениях.",
                )
                .color(MUTED),
            );
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.busy(), |ui| {
                    let width = (ui.available_width() - 175.0).max(200.0);
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
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Выбрать папку").strong().color(BACKGROUND),
                            )
                            .fill(ACCENT),
                        )
                        .clicked()
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
                    RichText::new("Меню и обучение загружаются из выбранной установки оригинала.")
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

    fn background(&self, ui: &egui::Ui) {
        let rect = ui.max_rect();
        let painter = ui.painter();
        painter.rect_filled(rect, 0, BACKGROUND);
        if let Some(texture) = &self.hero {
            let source = texture.size_vec2().x / texture.size_vec2().y;
            let aspect = rect.aspect_ratio();
            let (w, h) = if aspect > source {
                (1.0, source / aspect)
            } else {
                (aspect / source, 1.0)
            };
            let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), Vec2::new(w, h));
            painter.image(texture.id(), rect, uv, Color32::WHITE);
        }
        let mut fade = egui::Mesh::default();
        fade.colored_vertex(rect.left_top(), Color32::from_black_alpha(200));
        fade.colored_vertex(rect.right_top(), Color32::from_black_alpha(65));
        fade.colored_vertex(rect.left_bottom(), Color32::from_black_alpha(235));
        fade.colored_vertex(rect.right_bottom(), Color32::from_black_alpha(180));
        fade.indices.extend_from_slice(&[0, 1, 2, 1, 3, 2]);
        painter.add(egui::Shape::mesh(fade));
    }

    fn launch(&mut self) {
        let Some(data) = self.data_path() else {
            self.settings = true;
            return;
        };
        self.save();
        if let Some(installed) = &self.installed {
            match installed.launch(&self.root, &data) {
                Ok(child) => {
                    self.child = Some(child);
                    self.status = "Игра запущена".into();
                }
                Err(error) => self.error = Some(format!("{error:#}")),
            }
        }
    }

    fn download_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let width = ui.available_width();
        let button_width = 220.0;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(width - button_width - 24.0, 94.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(width - button_width - 24.0);
                    ui.horizontal(|ui| {
                        if self.busy() {
                            ui.spinner();
                        }
                        ui.label(RichText::new(&self.status).size(16.0).strong());
                    });
                    if let Some(progress) = &self.progress {
                        let fraction = if progress.total > 0 {
                            progress.downloaded as f32 / progress.total as f32
                        } else {
                            0.0
                        };
                        let speed = self
                            .download_started
                            .map(|at| {
                                progress.downloaded as f64 / at.elapsed().as_secs_f64().max(0.1)
                            })
                            .unwrap_or(0.0);
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .fill(ACCENT)
                                .desired_height(8.0),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} / {}  ·  {}/с  ·  {:.0}%",
                                bytes(progress.downloaded),
                                bytes(progress.total),
                                bytes(speed as u64),
                                fraction * 100.0
                            ))
                            .small()
                            .color(MUTED),
                        );
                        if ui.small_button("Остановить загрузку").clicked() {
                            self.cancel.store(true, Ordering::Relaxed);
                            self.blocked_update = self
                                .available
                                .as_ref()
                                .and_then(|a| a.release.fingerprint().ok());
                        }
                    } else {
                        let version = self
                            .installed
                            .as_ref()
                            .map(|i| i.release.version.as_str())
                            .unwrap_or("не установлена");
                        ui.label(
                            RichText::new(format!(
                                "Версия игры: {version}  ·  {}",
                                opensc_launcher::platform()
                            ))
                            .small()
                            .color(MUTED),
                        );
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    !self.busy(),
                                    egui::Button::new("Проверить обновления").small(),
                                )
                                .clicked()
                            {
                                self.check(ctx);
                                self.next_launcher_check = Instant::now();
                            }
                            if ui.small_button("Папка игры и настройки").clicked()
                            {
                                self.settings = true;
                            }
                        });
                    }
                    if let Some(error) = &self.error {
                        egui::ScrollArea::vertical()
                            .max_height(48.0)
                            .show(ui, |ui| {
                                ui.colored_label(Color32::from_rgb(245, 154, 126), error);
                            });
                    }
                },
            );
            ui.vertical(|ui| {
                let update = self
                    .available
                    .as_ref()
                    .is_some_and(|a| self.needs_update(a));
                let text = if self.child.is_some() {
                    "ИГРА ЗАПУЩЕНА"
                } else if self.busy() {
                    if self.progress.is_some() {
                        "ЗАГРУЗКА…"
                    } else {
                        "ПРОВЕРКА…"
                    }
                } else if update {
                    if self.blocked_update.is_some() {
                        if self.error.is_some() {
                            "ПОВТОРИТЬ"
                        } else {
                            "ЗАГРУЗИТЬ"
                        }
                    } else {
                        "ОБНОВИТЬ"
                    }
                } else if self.installed.is_none() {
                    "СКАЧАТЬ ИГРУ"
                } else if self.data_path().is_none() {
                    "ВЫБРАТЬ DATA"
                } else {
                    "ИГРАТЬ"
                };
                let can_click =
                    !self.busy() && self.child.is_none() && (self.installed.is_some() || update);
                if ui
                    .add_enabled(
                        can_click,
                        egui::Button::new(
                            RichText::new(text)
                                .size(21.0)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(PLAY)
                        .corner_radius(2)
                        .min_size(Vec2::new(button_width, 62.0)),
                    )
                    .clicked()
                {
                    if update {
                        self.blocked_update = None;
                        self.install(ctx);
                    } else {
                        self.launch();
                    }
                }
                ui.label(
                    RichText::new(if self.config.auto_update {
                        "Обновления загружаются автоматически"
                    } else {
                        "Автозагрузка выключена в настройках"
                    })
                    .size(10.0)
                    .color(MUTED),
                );
            });
        });
        ui.label(RichText::new(&self.launcher_status).size(10.0).color(MUTED));
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
        self.tick(ctx);
        self.poll_launcher(ctx);
        ctx.request_repaint_after(Duration::from_millis(if self.busy() { 100 } else { 500 }));
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(28, 15)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("OpenSC").size(27.0).strong());
                    ui.add_space(18.0);
                    ui.label(
                        RichText::new("SHADOW COMPANY: LEFT FOR DEAD")
                            .monospace()
                            .size(10.0)
                            .color(ACCENT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("НАСТРОЙКИ").clicked() {
                            self.settings = !self.settings;
                        }
                        if ui.button("СООБЩИТЬ ОБ ОШИБКЕ").clicked() {
                            self.reports.open = true;
                        }
                        ui.hyperlink_to("САЙТ ПРОЕКТА", "https://shadow-test.zaikinagroup.ru/");
                    });
                });
            });
        egui::TopBottomPanel::bottom("download")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(28, 18)),
            )
            .show(ctx, |ui| {
                self.download_panel(ui, ctx);
            });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BACKGROUND)).show(ctx, |ui| {
            self.background(ui);
            let rect = ui.max_rect().shrink2(Vec2::new(32.0, 26.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                let side = if rect.width() > 1050.0 { 310.0 } else { 265.0 };
                let left = rect.width() - side - 40.0;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(Vec2::new(left, rect.height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(left);
                        ui.add_space((rect.height() * 0.15).clamp(20.0, 80.0));
                        eyebrow(ui, "OPEN SHADOW COMPANY");
                        ui.add_space(12.0);
                        ui.label(RichText::new("ВЕРНУТЬСЯ\nВ СТРОЙ.").size(if left > 550.0 { 64.0 } else { 50.0 }).strong().color(Color32::from_rgb(244, 239, 223)));
                        ui.add_space(14.0);
                        ui.label(RichText::new("Новый движок. Знакомые операции.").size(21.0).color(ACCENT));
                        ui.add_space(8.0);
                        ui.label(RichText::new("Меню, видео и обучение на ресурсах вашей копии Shadow Company: Left for Dead.").size(15.0).color(Color32::from_rgb(203, 207, 194)));
                        ui.add_space(18.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("RUST / BEVY").monospace().size(10.0).color(MUTED));
                            ui.separator();
                            ui.label(RichText::new("WINDOWS · LINUX · macOS").monospace().size(10.0).color(MUTED));
                        });
                    });
                    ui.allocate_ui_with_layout(Vec2::new(side, rect.height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(side);
                        egui::ScrollArea::vertical().id_salt("news").show(ui, |ui| {
                            eyebrow(ui, "ОБНОВЛЕНИЯ ПРОЕКТА");
                            ui.add_space(8.0);
                            news_card(ui, "В ЭТОМ ВЫПУСКЕ", "Меню и русский / English", self.available.as_ref().map(|a| a.release.notes.as_str()).unwrap_or("Новое главное меню, анимация кнопок, современные шрифты и переключение языка интерфейса."));
                            ui.add_space(10.0);
                            news_card(ui, "НАЧАЛО ИГРЫ", "Ваша копия Shadow Company", "Один раз выберите папку Data в настройках. Лаунчер запомнит путь и установит обновления движка.");
                            ui.add_space(10.0);
                            news_card(ui, "ТЕСТОВЫЙ ПРОЕКТ", "Возвращение на учебную карту", "Доступны меню, видео и обучение. Полная кампания, сохранения и сетевая игра ещё разрабатываются.");
                        });
                    });
                });
            });
        });
        self.reports
            .show(ctx, &self.root, &self.config, self.installed.as_ref());
        if self.settings {
            let mut open = true;
            egui::Window::new("Настройки лаунчера")
                .open(&mut open)
                .default_width(700.0)
                .vscroll(true)
                .show(ctx, |ui| {
                    self.resource_card(ui, ctx);
                    ui.add_space(14.0);
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
                    ui.add_enabled_ui(!self.busy(), |ui| {
                        ui.checkbox(
                            &mut self.config.auto_update,
                            "Обновлять игру при запуске лаунчера",
                        );
                    });
                    ui.checkbox(&mut self.config.auto_update_launcher,"Автоматически обновлять сам лаунчер (подписанные выпуски)");
                    ui.label(RichText::new(&self.launcher_status).small().color(MUTED));
                    ui.label("Улучшенные ресурсы (пусто — папка overrides в OpenSC)");
                    ui.add_enabled(
                        !self.busy() && self.child.is_none(),
                        egui::TextEdit::singleline(&mut self.config.overrides_path)
                            .desired_width(f32::INFINITY),
                    );
                    ui.label(RichText::new("Проверка при открытии и каждые 60 секунд. Во время игры установка откладывается до её завершения.").small().color(MUTED));
                    ui.label("Папка установок и журналов");
                    ui.label(
                        RichText::new(self.root.display().to_string())
                            .monospace()
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Сохранить и проверить").clicked() {
                            self.save(); self.next_check = Instant::now();
                            self.next_launcher_check = Instant::now();
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
                    ui.separator();
                    ui.label(RichText::new(format!("Лаунчер {}", env!("CARGO_PKG_VERSION"))).small().color(MUTED));
                    egui::CollapsingHeader::new("Журнал действий").show(ui, |ui| {
                        for line in self.activity.iter().rev().take(12) { ui.label(line); }
                    });
                });
            self.settings = open;
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(handoff) = &mut self.launcher_handoff {
            handoff.cancel();
        }
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
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(49, 61, 51)))
        .corner_radius(4)
        .inner_margin(18)
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

fn news_card(ui: &mut egui::Ui, tag: &str, title: &str, body: &str) {
    egui::Frame::new()
        .fill(Color32::from_black_alpha(185))
        .stroke(Stroke::new(1.0_f32, Color32::from_white_alpha(40)))
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, tag);
            ui.label(RichText::new(title).size(18.0).strong());
            ui.label(
                RichText::new(body)
                    .size(13.0)
                    .color(Color32::from_rgb(190, 198, 185)),
            );
        });
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
