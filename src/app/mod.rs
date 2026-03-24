use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use eframe::egui::{self, Align, Color32, Layout, RichText, TextEdit, Vec2};

use crate::{
    domain::{EventSelection, ExportSettings, JobStatus, LogSource, default_event_selections},
    export::{
        build_xml_document, default_output_path, metadata_from_snapshot, open_file, open_folder,
        resolve_export_path, write_xml_file,
    },
    platform::{
        EventLogProgress, EventLogQuery, EventLogService, PreviewSnapshot, ScanController,
        default_event_log_service,
    },
};

const PREVIEW_LIMIT: usize = 20;
const BUTTON_HEIGHT: f32 = 34.0;
const FOOTER_HEIGHT: f32 = 32.0;
const ACTION_BAR_HEIGHT: f32 = 58.0;
const PREVIEW_MIN_HEIGHT: f32 = 240.0;
const PREVIEW_BODY_MIN_HEIGHT: f32 = 180.0;
const PREVIEW_EDITOR_MIN_WIDTH: f32 = 520.0;
const SIDEBAR_LIST_MIN_HEIGHT: f32 = 120.0;
const SIDEBAR_ACTIONS_HEIGHT: f32 = 124.0;
const TOP_PANEL_MIN_HEIGHT: f32 = 224.0;
const PANEL_LABEL_SIZE: f32 = 14.0;
const ANALYTICS_METRIC_CARD_HEIGHT: f32 = 80.0;
const ANALYTICS_DETAIL_ROW_HEIGHT: f32 = 62.0;
const ACTION_BAR_WRAP_THRESHOLD: f32 = 760.0;
const SPACE_XXS: f32 = 4.0;
const SPACE_XS: f32 = 6.0;
const SPACE_SM: f32 = 8.0;
const SPACE_MD: f32 = 12.0;
const SPACE_LG: f32 = 16.0;

pub struct EventXmlExporterApp {
    event_log_service: Arc<dyn EventLogService>,
    log_source: LogSource,
    max_events_input: String,
    export_all: bool,
    output_path_input: String,
    new_event_id_input: String,
    new_event_label_input: String,
    show_selected_only: bool,
    events: Vec<EventSelection>,
    preview_xml: String,
    preview_snapshot: PreviewSnapshot,
    task_progress: TaskProgressState,
    task_receiver: Option<Receiver<BackgroundTaskMessage>>,
    task_cancel_flag: Option<Arc<AtomicBool>>,
    active_task: Option<ActiveTask>,
    pending_preview_refresh: bool,
    status: JobStatus,
    status_message: String,
    last_error: Option<String>,
    last_generated_file: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActiveTask {
    Preview,
    Export,
}

#[derive(Clone, Debug, Default)]
struct TaskProgressState {
    current_log: String,
    scanned_records: usize,
    matched_records: usize,
}

#[derive(Debug)]
enum BackgroundTaskMessage {
    Progress(EventLogProgress),
    PreviewFinished(Result<PreviewTaskSuccess, String>),
    ExportFinished(Result<ExportTaskSuccess, String>),
}

#[derive(Debug)]
struct PreviewTaskSuccess {
    snapshot: PreviewSnapshot,
    xml: String,
}

#[derive(Debug)]
struct ExportTaskSuccess {
    path: PathBuf,
    snapshot: PreviewSnapshot,
    preview_xml: String,
}

enum FinishedTask {
    Preview(Result<PreviewTaskSuccess, String>),
    Export(Result<ExportTaskSuccess, String>),
    Disconnected,
}

impl EventXmlExporterApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_theme(&cc.egui_ctx);

        let mut app = Self {
            event_log_service: Arc::from(default_event_log_service()),
            log_source: LogSource::System,
            max_events_input: "1000".to_owned(),
            export_all: false,
            output_path_input: default_output_path(Local::now()).display().to_string(),
            new_event_id_input: String::new(),
            new_event_label_input: String::new(),
            show_selected_only: false,
            events: default_event_selections(),
            preview_xml: preview_placeholder_text(),
            preview_snapshot: PreviewSnapshot::default(),
            task_progress: TaskProgressState::default(),
            task_receiver: None,
            task_cancel_flag: None,
            active_task: None,
            pending_preview_refresh: false,
            status: JobStatus::Ready,
            status_message: "Hazır. Gerçek Windows günlükleri taranabilir.".to_owned(),
            last_error: None,
            last_generated_file: None,
        };

        app.start_preview_scan();
        app
    }

    fn start_preview_scan(&mut self) {
        if matches!(self.active_task, Some(ActiveTask::Preview)) {
            self.pending_preview_refresh = true;
            self.cancel_background_task();
            return;
        }

        if self.active_task.is_some() {
            self.pending_preview_refresh = true;
            return;
        }

        let settings = match self.collect_settings() {
            Ok(settings) => settings,
            Err(error) => {
                self.status = JobStatus::Error;
                self.status_message = "Önizleme hazırlanamadı.".to_owned();
                self.last_error = Some(error.to_string());
                return;
            }
        };

        let (sender, receiver) = mpsc::channel();
        let service = Arc::clone(&self.event_log_service);
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let progress_sender = sender.clone();
        let controller = ScanController::new(
            Arc::clone(&cancel_flag),
            Some(Arc::new(move |progress| {
                let _ = progress_sender.send(BackgroundTaskMessage::Progress(progress));
            })),
        );
        let finish_sender = sender.clone();

        self.active_task = Some(ActiveTask::Preview);
        self.task_receiver = Some(receiver);
        self.task_cancel_flag = Some(cancel_flag);
        self.task_progress = TaskProgressState::default();
        self.status = JobStatus::Work;
        self.status_message = "Canlı önizleme güncelleniyor.".to_owned();
        self.last_error = None;

        thread::spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                run_preview_task(service, settings, controller)
            }))
            .map_err(panic_payload_to_string)
            .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = finish_sender.send(BackgroundTaskMessage::PreviewFinished(result));
        });
    }

    fn begin_export(&mut self) {
        if self.active_task.is_some() {
            self.status_message = "Önce çalışan işlemi tamamlayın ya da iptal edin.".to_owned();
            return;
        }

        let settings = match self.collect_settings() {
            Ok(settings) => settings,
            Err(error) => {
                self.status = JobStatus::Error;
                self.status_message = "Dışa aktarma başlatılamadı.".to_owned();
                self.last_error = Some(error.to_string());
                return;
            }
        };

        let (sender, receiver) = mpsc::channel();
        let service = Arc::clone(&self.event_log_service);
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let progress_sender = sender.clone();
        let controller = ScanController::new(
            Arc::clone(&cancel_flag),
            Some(Arc::new(move |progress| {
                let _ = progress_sender.send(BackgroundTaskMessage::Progress(progress));
            })),
        );
        let finish_sender = sender.clone();

        self.active_task = Some(ActiveTask::Export);
        self.task_receiver = Some(receiver);
        self.task_cancel_flag = Some(cancel_flag);
        self.task_progress = TaskProgressState::default();
        self.status = JobStatus::Work;
        self.status_message = "Gerçek günlük kayıtları XML olarak dışa aktarılıyor.".to_owned();
        self.last_error = None;

        thread::spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                run_export_task(service, settings, controller)
            }))
            .map_err(panic_payload_to_string)
            .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = finish_sender.send(BackgroundTaskMessage::ExportFinished(result));
        });
    }

    fn cancel_background_task(&mut self) {
        if let Some(cancel_flag) = &self.task_cancel_flag {
            cancel_flag.store(true, Ordering::Relaxed);
            self.status_message = "Çalışan işlem iptal ediliyor.".to_owned();
        }
    }

    fn collect_settings(&self) -> Result<ExportSettings> {
        let output_path = self.output_path_input.trim();
        if output_path.is_empty() {
            bail!("Hedef dosya yolu boş bırakılamaz.");
        }

        let selected_event_ids = self
            .events
            .iter()
            .filter(|event| event.selected)
            .map(|event| event.event_id)
            .collect::<Vec<_>>();

        if selected_event_ids.is_empty() {
            bail!("En az bir Event ID seçmelisiniz.");
        }

        let max_events = if self.export_all {
            None
        } else {
            Some(parse_positive_usize(&self.max_events_input)?)
        };

        Ok(ExportSettings {
            source: self.log_source,
            max_events,
            export_all: self.export_all,
            output_path: PathBuf::from(output_path),
            selected_event_ids,
        })
    }

    fn add_event_id(&mut self) {
        let event_id = match self.new_event_id_input.trim().parse::<u32>() {
            Ok(value) if value > 0 => value,
            _ => {
                self.status = JobStatus::Error;
                self.status_message = "Geçerli bir Event ID girin.".to_owned();
                self.last_error =
                    Some("Event ID alanı yalnızca pozitif sayı kabul eder.".to_owned());
                return;
            }
        };

        let label = if self.new_event_label_input.trim().is_empty() {
            "Özel Kural".to_owned()
        } else {
            self.new_event_label_input.trim().to_owned()
        };

        if let Some(existing) = self
            .events
            .iter_mut()
            .find(|item| item.event_id == event_id)
        {
            existing.label = label;
            existing.selected = true;
            self.status_message = format!("Event ID {} güncellendi.", event_id);
        } else {
            self.events.push(EventSelection {
                event_id,
                label,
                selected: true,
            });
            self.events.sort_by_key(|item| item.event_id);
            self.status_message = format!("Event ID {} eklendi.", event_id);
        }

        self.new_event_id_input.clear();
        self.new_event_label_input.clear();
        self.status = JobStatus::Done;
        self.last_error = None;
        self.start_preview_scan();
    }

    fn remove_selected_event_ids(&mut self) {
        let before = self.events.len();
        self.events.retain(|event| !event.selected);
        let removed = before.saturating_sub(self.events.len());

        if removed == 0 {
            self.status = JobStatus::Error;
            self.status_message = "Silinecek seçili Event ID bulunamadı.".to_owned();
            self.last_error = Some("Listede kaldırılacak seçim yok.".to_owned());
            return;
        }

        self.status = JobStatus::Done;
        self.status_message = format!("{removed} adet Event ID kaldırıldı.");
        self.last_error = None;
        self.start_preview_scan();
    }

    fn select_all_event_ids(&mut self) {
        for event in &mut self.events {
            event.selected = true;
        }
        self.status = JobStatus::Done;
        self.status_message = "Tüm Event ID kayıtları seçildi.".to_owned();
        self.last_error = None;
        self.start_preview_scan();
    }

    fn clear_selected_event_ids(&mut self) {
        for event in &mut self.events {
            event.selected = false;
        }
        self.status = JobStatus::Done;
        self.status_message = "Tüm seçimler temizlendi.".to_owned();
        self.last_error = None;
        self.start_preview_scan();
    }

    fn poll_background_task(&mut self) {
        let mut finished = None;

        if let Some(receiver) = &self.task_receiver {
            loop {
                match receiver.try_recv() {
                    Ok(BackgroundTaskMessage::Progress(progress)) => {
                        self.task_progress.current_log = progress.current_log;
                        self.task_progress.scanned_records = progress.scanned_records;
                        self.task_progress.matched_records = progress.matched_records;
                    }
                    Ok(BackgroundTaskMessage::PreviewFinished(result)) => {
                        finished = Some(FinishedTask::Preview(result));
                        break;
                    }
                    Ok(BackgroundTaskMessage::ExportFinished(result)) => {
                        finished = Some(FinishedTask::Export(result));
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = Some(FinishedTask::Disconnected);
                        break;
                    }
                }
            }
        }

        if let Some(finished_task) = finished {
            self.task_receiver = None;
            self.task_cancel_flag = None;
            self.active_task = None;

            match finished_task {
                FinishedTask::Preview(result) => match result {
                    Ok(success) => self.apply_preview_success(success),
                    Err(error) => self.handle_task_error(error, "Önizleme güncellenemedi."),
                },
                FinishedTask::Export(result) => match result {
                    Ok(success) => self.apply_export_success(success),
                    Err(error) => self.handle_task_error(error, "Dışa aktarma tamamlanamadı."),
                },
                FinishedTask::Disconnected => {
                    self.handle_task_error(
                        "Arka plan görevi beklenmedik şekilde sonlandı.".to_owned(),
                        "Arka plan görevi yanıt vermiyor.",
                    );
                }
            }

            if self.pending_preview_refresh {
                self.pending_preview_refresh = false;
                self.start_preview_scan();
            }
        }
    }

    fn apply_preview_success(&mut self, success: PreviewTaskSuccess) {
        self.preview_snapshot = success.snapshot;
        self.preview_xml = success.xml;
        self.status = JobStatus::Ready;
        self.status_message = "Canlı önizleme güncel.".to_owned();
        self.last_error = None;
    }

    fn apply_export_success(&mut self, success: ExportTaskSuccess) {
        self.preview_snapshot = success.snapshot;
        self.preview_xml = success.preview_xml;
        self.output_path_input = success.path.display().to_string();
        self.last_generated_file = Some(success.path);
        self.status = JobStatus::Done;
        self.status_message = "XML dışa aktarma tamamlandı.".to_owned();
        self.last_error = None;
    }

    fn handle_task_error(&mut self, error: String, fallback_message: &str) {
        if error.contains("iptal edildi") {
            self.status = JobStatus::Ready;
            self.status_message = "İşlem iptal edildi.".to_owned();
            self.last_error = None;
            return;
        }

        self.status = JobStatus::Error;
        self.status_message = fallback_message.to_owned();
        self.last_error = Some(error);
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(SPACE_MD);
        panel_frame().show(ui, |ui| {
            egui::TopBottomPanel::bottom("sidebar_actions_zone")
                .resizable(false)
                .exact_height(SIDEBAR_ACTIONS_HEIGHT)
                .show_inside(ui, |ui| self.render_sidebar_actions(ui));

            egui::TopBottomPanel::top("sidebar_controls_zone")
                .resizable(false)
                .show_inside(ui, |ui| self.render_sidebar_controls(ui));

            ui.add_space(SPACE_SM);
            self.render_sidebar_event_list(ui);
        });
        ui.add_space(SPACE_MD);
    }

    fn render_sidebar_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading(
            RichText::new("Olay Kaynağı Seçimi")
                .size(22.0)
                .color(accent_color()),
        );
        ui.add_space(SPACE_MD);

        ui.label("Yeni Event ID");
        let add_enter = ui
            .add(TextEdit::singleline(&mut self.new_event_id_input).hint_text("Örn: 4625"))
            .lost_focus()
            && ui.input(|input| input.key_pressed(egui::Key::Enter));

        ui.add_space(SPACE_XS);
        ui.label("Özel etiket");
        ui.add(
            TextEdit::singleline(&mut self.new_event_label_input)
                .hint_text("Örn: Başarısız Oturum"),
        );
        ui.add_space(SPACE_SM);

        if ui
            .add_sized(
                [ui.available_width(), BUTTON_HEIGHT],
                egui::Button::new("ID Ekle"),
            )
            .clicked()
            || add_enter
        {
            self.add_event_id();
        }

        ui.add_space(SPACE_LG);
        ui.separator();
        ui.add_space(SPACE_LG);
        ui.checkbox(
            &mut self.show_selected_only,
            "Yalnızca seçili olanları göster",
        );
        ui.add_space(SPACE_SM);
    }

    fn render_sidebar_event_list(&mut self, ui: &mut egui::Ui) {
        let list_height = ui.available_height().max(0.0);
        apply_soft_min_height(ui, SIDEBAR_LIST_MIN_HEIGHT);
        let mut selection_changed = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .max_height(list_height)
            .show(ui, |ui| {
                for event in self.filtered_events_mut() {
                    let label = event.display_label();
                    selection_changed |= ui.checkbox(&mut event.selected, label).changed();
                }
            });

        if selection_changed {
            self.start_preview_scan();
        }
    }

    fn render_sidebar_actions(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_sized(
                [ui.available_width(), BUTTON_HEIGHT],
                egui::Button::new("Sil"),
            )
            .clicked()
        {
            self.remove_selected_event_ids();
        }
        if ui
            .add_sized(
                [ui.available_width(), BUTTON_HEIGHT],
                egui::Button::new("Tümünü Seç"),
            )
            .clicked()
        {
            self.select_all_event_ids();
        }
        if ui
            .add_sized(
                [ui.available_width(), BUTTON_HEIGHT],
                egui::Button::new("Temizle"),
            )
            .clicked()
        {
            self.clear_selected_event_ids();
        }
    }

    fn render_top_region(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |columns| {
            render_top_region_panel(
                &mut columns[0],
                "Dışa Aktarma Parametreleri",
                TOP_PANEL_MIN_HEIGHT,
                |ui| self.render_parameters_panel(ui),
            );
            render_top_region_panel(
                &mut columns[1],
                "Sistem Analitiği",
                TOP_PANEL_MIN_HEIGHT,
                |ui| self.render_analytics_panel(ui),
            );
        });
    }

    fn render_parameters_panel(&mut self, ui: &mut egui::Ui) {
        let mut refresh_requested = false;

        ui.label("Günlük kaynağı");
        egui::ComboBox::from_id_salt("log_source")
            .width(ui.available_width())
            .selected_text(self.log_source.display_name())
            .show_ui(ui, |ui| {
                for source in LogSource::ALL {
                    refresh_requested |= ui
                        .selectable_value(&mut self.log_source, source, source.display_name())
                        .changed();
                }
            });

        ui.add_space(SPACE_MD);
        ui.label("Maksimum kayıt");
        refresh_requested |= ui
            .add(TextEdit::singleline(&mut self.max_events_input))
            .changed();

        ui.add_space(SPACE_MD);
        if ui
            .checkbox(
                &mut self.export_all,
                "Tüm eşleşen kayıtları sınır olmadan dışa aktar",
            )
            .changed()
        {
            refresh_requested = true;
        }

        ui.add_space(SPACE_MD);
        ui.label("Hedef çıktı yolu");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add_sized([110.0, BUTTON_HEIGHT], egui::Button::new("Gözat"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("XML çıktı konumu seçin")
                    .add_filter("XML", &["xml"])
                    .set_file_name("GNN_Export.xml")
                    .save_file()
            {
                self.output_path_input = path.display().to_string();
            }

            ui.add_sized(
                [ui.available_width(), BUTTON_HEIGHT],
                TextEdit::singleline(&mut self.output_path_input).desired_width(f32::INFINITY),
            );
        });

        if refresh_requested {
            self.start_preview_scan();
        }
    }

    fn render_analytics_panel(&self, ui: &mut egui::Ui) {
        ui.columns(3, |columns| {
            analytics_card(
                &mut columns[0],
                "Toplam eşleşen kayıt",
                self.preview_snapshot.analytics.total_logs_found.to_string(),
                ANALYTICS_METRIC_CARD_HEIGHT,
            );
            analytics_card(
                &mut columns[1],
                "Kuyruğa alınan kayıt",
                self.preview_snapshot.analytics.queue_size.to_string(),
                ANALYTICS_METRIC_CARD_HEIGHT,
            );
            analytics_card(
                &mut columns[2],
                "Taranan kayıt",
                self.task_progress
                    .scanned_records
                    .max(self.preview_snapshot.scanned_records)
                    .to_string(),
                ANALYTICS_METRIC_CARD_HEIGHT,
            );
        });

        ui.add_space(SPACE_XXS);
        ui.columns(2, |columns| {
            status_card(
                &mut columns[0],
                self.status.display_name(),
                &self.status_message,
                ANALYTICS_DETAIL_ROW_HEIGHT,
            );
            notification_card(
                &mut columns[1],
                self.last_generated_file
                    .as_ref()
                    .and_then(|path| path.file_name().and_then(|value| value.to_str()))
                    .map(str::to_owned),
                self.last_error.clone(),
                ANALYTICS_DETAIL_ROW_HEIGHT,
            );
        });
        ui.add_space(SPACE_XXS);
        ui.columns(3, |columns| {
            summary_chip_fill(
                &mut columns[0],
                format!(
                    "Aktif günlük: {}",
                    if self.task_progress.current_log.is_empty() {
                        self.log_source.display_name().to_owned()
                    } else {
                        self.task_progress.current_log.clone()
                    }
                ),
            );
            summary_chip_fill(
                &mut columns[1],
                format!(
                    "Eşleşen: {}",
                    self.task_progress
                        .matched_records
                        .max(self.preview_snapshot.analytics.total_logs_found)
                ),
            );
            summary_chip_fill(
                &mut columns[2],
                format!("Süre: {} ms", self.preview_snapshot.duration_ms),
            );
        });
    }

    fn render_preview_panel(&mut self, ui: &mut egui::Ui) {
        egui::TopBottomPanel::top("preview_meta_zone")
            .resizable(false)
            .show_inside(ui, |ui| self.render_preview_header(ui));

        apply_soft_min_height(ui, PREVIEW_BODY_MIN_HEIGHT);
        let body_height = ui.available_height().max(0.0);
        egui::ScrollArea::both()
            .auto_shrink([false; 2])
            .max_height(body_height)
            .show(ui, |ui| self.render_preview_body(ui, body_height));
    }

    fn render_preview_header(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            summary_chip(
                ui,
                format!(
                    "İlk {} kayıt önizleniyor",
                    self.preview_snapshot.records.len().min(PREVIEW_LIMIT)
                ),
            );
            summary_chip(
                ui,
                format!(
                    "Toplam eşleşme: {}",
                    self.preview_snapshot.analytics.total_logs_found
                ),
            );
            summary_chip(
                ui,
                format!("Taranan kayıt: {}", self.preview_snapshot.scanned_records),
            );
        });

        ui.add_space(SPACE_XS);
        summary_chip_fill(
            ui,
            format!(
                "Sağlayıcı: {}",
                format_list_preview(&self.preview_snapshot.providers)
            ),
        );

        ui.add_space(SPACE_SM);
        if self.preview_snapshot.event_id_counts.is_empty() {
            ui.label("Henüz Event ID dağılımı oluşmadı.");
        } else {
            ui.label(format!(
                "Event ID dağılımı: {}",
                self.preview_snapshot
                    .event_id_counts
                    .iter()
                    .map(|(event_id, count)| format!("{event_id} ({count})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        ui.add_space(SPACE_SM);
    }

    fn render_preview_body(&mut self, ui: &mut egui::Ui, body_height: f32) {
        ui.add_sized(
            [
                ui.available_width().max(PREVIEW_EDITOR_MIN_WIDTH),
                body_height,
            ],
            TextEdit::multiline(&mut self.preview_xml)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .interactive(false),
        );
    }

    fn render_output_buttons(&mut self, ui: &mut egui::Ui) {
        let has_output = self.last_generated_file.is_some();
        if ui
            .add_enabled(
                has_output,
                egui::Button::new("Dosyayı Aç").min_size(Vec2::new(110.0, BUTTON_HEIGHT)),
            )
            .clicked()
            && let Some(path) = &self.last_generated_file
            && let Err(error) = open_file(path)
        {
            self.status = JobStatus::Error;
            self.status_message = "Dosya açılamadı.".to_owned();
            self.last_error = Some(error.to_string());
        }

        if ui
            .add_enabled(
                has_output,
                egui::Button::new("Klasörü Aç").min_size(Vec2::new(110.0, BUTTON_HEIGHT)),
            )
            .clicked()
            && let Some(path) = &self.last_generated_file
            && let Err(error) = open_folder(path)
        {
            self.status = JobStatus::Error;
            self.status_message = "Klasör açılamadı.".to_owned();
            self.last_error = Some(error.to_string());
        }
    }

    fn render_primary_actions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui
            .add(
                egui::Button::new(RichText::new("XML Olarak Dışa Aktar").color(Color32::WHITE))
                    .fill(accent_color())
                    .min_size(Vec2::new(190.0, BUTTON_HEIGHT)),
            )
            .clicked()
        {
            self.begin_export();
        }

        if ui
            .add(egui::Button::new("Kapat").min_size(Vec2::new(110.0, BUTTON_HEIGHT)))
            .clicked()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if self.active_task.is_some()
            && ui
                .add(egui::Button::new("İptal").min_size(Vec2::new(110.0, BUTTON_HEIGHT)))
                .clicked()
        {
            self.cancel_background_task();
        }
    }

    fn filtered_events_mut(&mut self) -> Vec<&mut EventSelection> {
        if self.show_selected_only {
            self.events
                .iter_mut()
                .filter(|event| event.selected)
                .collect()
        } else {
            self.events.iter_mut().collect()
        }
    }
}

impl eframe::App for EventXmlExporterApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_background_task();
        if self.active_task.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::TopBottomPanel::bottom("footer")
            .resizable(false)
            .exact_height(FOOTER_HEIGHT)
            .show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label("(c) 2026 GNNcyber | Olay XML Dışa Aktarıcı");
                });
            });

        egui::TopBottomPanel::bottom("action_bar")
            .resizable(false)
            .show(ctx, |ui| {
                let compact_actions = ui.available_width() < ACTION_BAR_WRAP_THRESHOLD;
                ui.add_space(SPACE_SM);
                ui.set_min_height(ACTION_BAR_HEIGHT);
                if compact_actions {
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| self.render_output_buttons(ui));
                        ui.add_space(SPACE_SM);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            self.render_primary_actions(ui, ctx);
                        });
                    });
                } else {
                    ui.vertical_centered(|ui| {
                        ui.horizontal(|ui| {
                            self.render_output_buttons(ui);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                self.render_primary_actions(ui, ctx);
                            });
                        });
                    });
                }
                ui.add_space(SPACE_SM);
            });

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .min_width(260.0)
            .default_width(290.0)
            .show(ctx, |ui| self.render_sidebar(ui));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(SPACE_MD);
            egui::TopBottomPanel::top("content_top_region")
                .resizable(false)
                .show_inside(ui, |ui| self.render_top_region(ui));

            ui.add_space(SPACE_MD);
            let preview_bottom_gap = SPACE_MD;
            let preview_height = (ui.available_height() - preview_bottom_gap).max(0.0);
            panel_frame().show(ui, |ui| {
                apply_soft_min_height(ui, PREVIEW_MIN_HEIGHT);
                section_title(ui, "Canlı Çıktı Önizlemesi");
                ui.add_space(SPACE_MD);
                self.render_preview_panel(ui);
                if preview_height > 0.0 {
                    ui.set_min_height(preview_height);
                }
            });
            ui.add_space(preview_bottom_gap);
        });
    }
}

fn run_preview_task(
    service: Arc<dyn EventLogService>,
    settings: ExportSettings,
    controller: ScanController,
) -> Result<PreviewTaskSuccess> {
    let snapshot = service.scan(&EventLogQuery::from_settings(&settings), controller)?;
    let xml = build_preview_xml(&settings, &snapshot)?;
    Ok(PreviewTaskSuccess { snapshot, xml })
}

fn run_export_task(
    service: Arc<dyn EventLogService>,
    settings: ExportSettings,
    controller: ScanController,
) -> Result<ExportTaskSuccess> {
    let started_at = Instant::now();
    let snapshot = service.scan(&EventLogQuery::from_settings(&settings), controller)?;
    let exported_at = Local::now();
    let target_path = resolve_export_path(&settings.output_path, exported_at);
    let metadata = metadata_from_snapshot(&settings, &snapshot, started_at.elapsed().as_millis());
    let xml = build_xml_document(&settings, &snapshot.records, exported_at, &metadata)?;
    write_xml_file(&target_path, &xml)
        .with_context(|| format!("XML dosyası yazılamadı: {}", target_path.display()))?;
    let preview_xml = build_preview_xml(&settings, &snapshot)?;

    Ok(ExportTaskSuccess {
        path: target_path,
        snapshot,
        preview_xml,
    })
}

fn build_preview_xml(settings: &ExportSettings, snapshot: &PreviewSnapshot) -> Result<String> {
    let preview_records = snapshot
        .records
        .iter()
        .take(PREVIEW_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    let metadata = metadata_from_snapshot(settings, snapshot, snapshot.duration_ms);
    build_xml_document(settings, &preview_records, Local::now(), &metadata)
}

fn parse_positive_usize(input: &str) -> Result<usize> {
    let value = input
        .trim()
        .parse::<usize>()
        .map_err(|_| anyhow!("Maksimum kayıt alanı yalnızca pozitif sayı kabul eder."))?;

    if value == 0 {
        bail!("Maksimum kayıt değeri sıfır olamaz.");
    }

    Ok(value)
}

fn format_list_preview(values: &[String]) -> String {
    if values.is_empty() {
        return "Bulunamadı".to_owned();
    }

    let preview = values
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if values.len() > 3 {
        format!("{preview} +{}", values.len() - 3)
    } else {
        preview
    }
}

fn panic_payload_to_string(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        format!("Arka plan görevi panik ile sonlandı: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("Arka plan görevi panik ile sonlandı: {message}")
    } else {
        "Arka plan görevi panik ile sonlandı.".to_owned()
    }
}

fn preview_placeholder_text() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<events source=\"System\" exported_at=\"hazırlanıyor\">\n  <metadata>\n    <status>Önizleme hazırlanıyor...</status>\n  </metadata>\n</events>\n".to_owned()
}

fn configure_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = Color32::from_rgb(13, 17, 23);
    visuals.panel_fill = Color32::from_rgb(13, 17, 23);
    visuals.extreme_bg_color = Color32::from_rgb(20, 25, 32);
    visuals.faint_bg_color = Color32::from_rgb(24, 30, 38);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(20, 25, 32);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(30, 36, 45);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(40, 48, 59);
    visuals.widgets.active.bg_fill = Color32::from_rgb(0, 159, 187);
    visuals.override_text_color = Some(Color32::from_rgb(224, 228, 236));
    ctx.set_visuals(visuals);
}

fn accent_color() -> Color32 {
    Color32::from_rgb(0, 179, 216)
}

fn panel_frame() -> egui::Frame {
    egui::Frame::group(&egui::Style::default())
        .fill(Color32::from_rgb(22, 28, 35))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(48, 62, 80)))
        .inner_margin(egui::Margin::same(16))
}

fn render_top_region_panel(
    ui: &mut egui::Ui,
    title: &str,
    min_height: f32,
    content: impl FnOnce(&mut egui::Ui),
) {
    panel_frame().show(ui, |ui| {
        apply_soft_min_height(ui, min_height);
        section_title(ui, title);
        ui.add_space(SPACE_MD);
        content(ui);
    });
}

fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).size(21.0).color(accent_color()));
}

fn analytics_card(ui: &mut egui::Ui, title: &str, value: String, min_height: f32) {
    egui::Frame::group(&egui::Style::default())
        .fill(Color32::from_rgb(16, 21, 27))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(44, 56, 72)))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            apply_soft_min_height(ui, min_height);
            ui.label(
                RichText::new(title)
                    .size(PANEL_LABEL_SIZE)
                    .color(Color32::from_rgb(150, 160, 174)),
            );
            ui.add_space(SPACE_XXS);
            ui.label(
                RichText::new(value)
                    .size(26.0)
                    .color(Color32::from_rgb(240, 244, 250)),
            );
        });
}

fn status_card(ui: &mut egui::Ui, status: &str, message: &str, min_height: f32) {
    egui::Frame::group(&egui::Style::default())
        .fill(Color32::from_rgb(16, 21, 27))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(44, 56, 72)))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            apply_soft_min_height(ui, min_height);
            ui.label(
                RichText::new("Durum")
                    .size(PANEL_LABEL_SIZE)
                    .color(Color32::from_rgb(150, 160, 174)),
            );
            ui.add_space(SPACE_XXS);
            ui.add_sized(
                [ui.available_width(), 18.0],
                egui::Label::new(
                    RichText::new(format!("{status} {message}"))
                        .size(14.0)
                        .color(Color32::from_rgb(240, 244, 250)),
                )
                .truncate(),
            );
        });
}

fn notification_card(
    ui: &mut egui::Ui,
    last_output_file: Option<String>,
    error_message: Option<String>,
    min_height: f32,
) {
    egui::Frame::group(&egui::Style::default())
        .fill(Color32::from_rgb(16, 21, 27))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(44, 56, 72)))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            apply_soft_min_height(ui, min_height);
            let has_output = last_output_file.is_some();
            let has_error = error_message.is_some();
            ui.label(
                RichText::new("Bildirimler")
                    .size(PANEL_LABEL_SIZE)
                    .color(Color32::from_rgb(150, 160, 174)),
            );
            ui.add_space(SPACE_XXS);

            let notification_text = if let Some(error) = error_message {
                RichText::new(error)
                    .size(14.0)
                    .color(Color32::from_rgb(255, 120, 120))
            } else if let Some(file_name) = last_output_file {
                RichText::new(format!("Son çıktı: {file_name}"))
                    .size(14.0)
                    .color(Color32::from_rgb(240, 244, 250))
            } else {
                RichText::new(if !has_output && !has_error {
                    "Yeni bildirim yok."
                } else {
                    ""
                })
                .size(14.0)
                .color(Color32::from_rgb(240, 244, 250))
            };

            ui.add_sized(
                [ui.available_width(), 18.0],
                egui::Label::new(notification_text).truncate(),
            );
        });
}

fn summary_chip(ui: &mut egui::Ui, text: String) {
    egui::Frame::group(&egui::Style::default())
        .fill(Color32::from_rgb(16, 24, 33))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(36, 56, 74)))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.label(text);
        });
}

fn summary_chip_fill(ui: &mut egui::Ui, text: String) {
    egui::Frame::group(&egui::Style::default())
        .fill(Color32::from_rgb(16, 24, 33))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(36, 56, 74)))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.add_sized(
                [ui.available_width(), 18.0],
                egui::Label::new(text).truncate(),
            );
        });
}

fn apply_soft_min_height(ui: &mut egui::Ui, preferred: f32) {
    if ui.available_height() >= preferred {
        ui.set_min_height(preferred);
    }
}
