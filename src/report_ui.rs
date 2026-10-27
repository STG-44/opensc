use eframe::egui::{self, RichText};
use opensc_launcher::{config::Config, reports, updater::Installed};
use opensc_report_protocol::{Diagnostics, Receipt, Report};
use std::{
    path::Path,
    sync::mpsc::{self, Receiver},
};

#[derive(Default)]
pub struct Form {
    pub open: bool,
    title: String,
    steps: String,
    expected: String,
    actual: String,
    category: usize,
    include_log: bool,
    pending: Option<Report>,
    captured: Option<Diagnostics>,
    receipt: Option<Receipt>,
    error: Option<String>,
    receiver: Option<Receiver<Result<Receipt, String>>>,
}

const CATEGORIES: [(&str, &str); 6] = [
    ("gameplay", "Игровая механика"),
    ("interface", "Интерфейс и управление"),
    ("crash", "Сбой или вылет"),
    ("graphics", "Графика"),
    ("audio", "Звук"),
    ("other", "Другое"),
];

impl Form {
    pub fn can_restart(&self) -> bool {
        !self.open && self.receiver.is_none()
    }
    pub fn load(root: &Path) -> Self {
        let pending = reports::pending(root);
        let mut form = Self {
            include_log: true,
            ..Self::default()
        };
        if let Some(report) = pending {
            form.title = report.title.clone();
            form.steps = report.steps.clone();
            form.actual = report.actual.clone();
            form.expected = report.expected.clone();
            form.category = CATEGORIES
                .iter()
                .position(|(id, _)| *id == report.category)
                .unwrap_or(5);
            form.include_log = !report.diagnostics.log_excerpt.is_empty();
            form.pending = Some(report);
        }
        form
    }

    pub fn crash(&mut self, root: &Path, config: &Config, installed: Option<&Installed>) {
        self.open = true;
        if self.pending.is_none() {
            self.captured = Some(reports::diagnostics(root, config, installed, true));
            self.category = 2;
            self.title = "Игра неожиданно завершилась".into();
            self.actual =
                "Игра закрылась с кодом ошибки. Последний фрагмент журнала приложен ниже.".into();
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        root: &Path,
        config: &Config,
        installed: Option<&Installed>,
    ) {
        if let Some(result) = self.receiver.as_ref().and_then(|r| r.try_recv().ok()) {
            self.receiver = None;
            match result {
                Ok(receipt) => {
                    self.receipt = Some(receipt);
                    self.pending = None;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        if !self.open {
            return;
        }
        if self.captured.is_none() {
            self.captured = Some(reports::diagnostics(root, config, installed, true));
        }
        let mut open = true;
        egui::Window::new("Сообщить об ошибке").open(&mut open).default_width(710.0)
            .vscroll(true).show(ctx, |ui| {
                if let Some(receipt) = &self.receipt {
                    ui.heading(format!("Обращение {} принято", receipt.ticket_id));
                    ui.label("Сохрани этот номер. Мы сможем найти обращение, сопоставить повторы и отметить исправление.");
                    if ui.button("Новое обращение").clicked() {
                        *self = Self { open: true, include_log: true, ..Self::default() };
                    }
                    return;
                }
                ui.label("Опиши, что произошло и как это повторить. Ниже можно проверить всю отправляемую диагностику.");
                let editable = self.pending.is_none() && self.receiver.is_none();
                ui.add_enabled_ui(editable, |ui| {
                    egui::ComboBox::from_label("Категория").selected_text(CATEGORIES[self.category].1).show_ui(ui, |ui| {
                        for (index,(_,label)) in CATEGORIES.iter().enumerate() { ui.selectable_value(&mut self.category,index,*label); }
                    });
                    ui.label("Краткий заголовок");
                    ui.add(egui::TextEdit::singleline(&mut self.title).desired_width(f32::INFINITY).char_limit(80));
                    ui.label("Шаги воспроизведения");
                    ui.add(egui::TextEdit::multiline(&mut self.steps).desired_width(f32::INFINITY).desired_rows(4).hint_text("1. Запустил обучение\n2. Открыл инвентарь\n3. Нажал…").char_limit(2000));
                    ui.label("Ожидаемый результат");
                    ui.add(egui::TextEdit::multiline(&mut self.expected).desired_width(f32::INFINITY).desired_rows(2).char_limit(2000));
                    ui.label("Что получилось");
                    ui.add(egui::TextEdit::multiline(&mut self.actual).desired_width(f32::INFINITY).desired_rows(2).char_limit(2000));
                    ui.checkbox(&mut self.include_log,"Приложить последний фрагмент журнала игры (до 24 КБ)");
                });
                let report = self.pending.clone().unwrap_or_else(|| Report {
                    schema_version: 1, request_id: reports::request_id(), category: CATEGORIES[self.category].0.into(),
                    title: self.title.trim().into(), steps: self.steps.trim().into(), expected: self.expected.trim().into(), actual: self.actual.trim().into(),
                    diagnostics: { let mut data = self.captured.clone().unwrap(); if !self.include_log { data.log_excerpt.clear(); } data },
                });
                egui::CollapsingHeader::new("Посмотреть всю отправляемую диагностику").show(ui, |ui| {
                    let mut text = serde_json::to_string_pretty(&report.diagnostics).unwrap_or_default();
                    ui.add(egui::TextEdit::multiline(&mut text).interactive(false).desired_width(f32::INFINITY).desired_rows(9));
                });
                if self.pending.is_some() { ui.label("Обращение сохранено локально. Повторная отправка использует тот же номер запроса и не создаёт лишнюю копию."); }
                if let Some(error) = &self.error { ui.colored_label(egui::Color32::LIGHT_RED,error); }
                if report.validate().is_err() { ui.label(RichText::new("Заполни краткий заголовок и шаги воспроизведения. Если текст слишком длинный, сократи его.").small()); }
                let can_send = self.receiver.is_none() && report.validate().is_ok();
                if ui.add_enabled(can_send,egui::Button::new(if self.pending.is_some() { "Повторить отправку" } else { "Отправить обращение" })).clicked() {
                    match reports::save_outbox(root,&report) {
                        Ok(()) => {
                            self.pending = Some(report.clone()); self.error = None;
                            let (tx,rx) = mpsc::channel(); self.receiver = Some(rx);
                            let (root,config,ctx) = (root.to_owned(),config.clone(),ctx.clone());
                            std::thread::spawn(move || { let result = reports::send(&root,&config,&report).map_err(|e|format!("{e:#}")); let _ = tx.send(result); ctx.request_repaint(); });
                        }
                        Err(error) => self.error = Some(format!("Не удалось сохранить обращение: {error:#}")),
                    }
                }
                if self.receiver.is_some() { ui.horizontal(|ui| { ui.spinner(); ui.label("Отправка обращения…"); }); }
            });
        self.open = open;
    }
}
