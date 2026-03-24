use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventSelection {
    pub event_id: u32,
    pub label: String,
    pub selected: bool,
}

impl EventSelection {
    pub fn display_label(&self) -> String {
        format!("{} {}", self.event_id, self.label)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LogSource {
    #[default]
    System,
    Application,
    Security,
}

impl LogSource {
    pub const ALL: [Self; 3] = [Self::System, Self::Application, Self::Security];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Application => "Application",
            Self::Security => "Security",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::System => "Sistem",
            Self::Application => "Uygulama",
            Self::Security => "Güvenlik",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Ready,
    Work,
    Done,
    Error,
}

impl JobStatus {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Ready => "HAZIR",
            Self::Work => "ÇALIŞIYOR",
            Self::Done => "TAMAMLANDI",
            Self::Error => "HATA",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSettings {
    pub source: LogSource,
    pub max_events: Option<usize>,
    pub export_all: bool,
    pub output_path: PathBuf,
    pub selected_event_ids: Vec<u32>,
}

impl ExportSettings {
    pub fn effective_max_events(&self) -> Option<usize> {
        if self.export_all {
            None
        } else {
            self.max_events
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRecord {
    pub event_id: u32,
    pub provider: String,
    pub level: String,
    pub computer: String,
    pub created_at: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnalyticsSnapshot {
    pub total_logs_found: usize,
    pub queue_size: usize,
}

pub fn default_event_selections() -> Vec<EventSelection> {
    vec![
        EventSelection {
            event_id: 41,
            label: "Kernel Power".to_owned(),
            selected: true,
        },
        EventSelection {
            event_id: 55,
            label: "NTFS Hatası".to_owned(),
            selected: true,
        },
        EventSelection {
            event_id: 6008,
            label: "Beklenmeyen Kapanma".to_owned(),
            selected: true,
        },
        EventSelection {
            event_id: 6005,
            label: "Günlük Başladı".to_owned(),
            selected: true,
        },
        EventSelection {
            event_id: 6006,
            label: "Günlük Durduruldu".to_owned(),
            selected: true,
        },
        EventSelection {
            event_id: 1001,
            label: "Hata Denetimi".to_owned(),
            selected: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::{ExportSettings, LogSource, default_event_selections};
    use std::path::PathBuf;

    #[test]
    fn default_presets_match_expected_ids() {
        let ids: Vec<u32> = default_event_selections()
            .into_iter()
            .map(|item| item.event_id)
            .collect();

        assert_eq!(ids, vec![41, 55, 6008, 6005, 6006, 1001]);
    }

    #[test]
    fn effective_max_events_ignores_limit_when_export_all_is_enabled() {
        let settings = ExportSettings {
            source: LogSource::System,
            max_events: Some(200),
            export_all: true,
            output_path: PathBuf::from("output.xml"),
            selected_event_ids: vec![41],
        };

        assert_eq!(settings.effective_max_events(), None);
    }
}
