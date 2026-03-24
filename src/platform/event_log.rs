use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use crate::domain::{AnalyticsSnapshot, EventRecord, ExportSettings, LogSource};
use anyhow::{Context, Result, anyhow};

#[derive(Clone, Debug)]
pub struct EventLogQuery {
    pub source: LogSource,
    pub event_ids: Vec<u32>,
    pub max_events: Option<usize>,
}

impl EventLogQuery {
    pub fn from_settings(settings: &ExportSettings) -> Self {
        Self {
            source: settings.source,
            event_ids: settings.selected_event_ids.clone(),
            max_events: settings.effective_max_events(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EventLogProgress {
    pub current_log: String,
    pub scanned_records: usize,
    pub matched_records: usize,
}

#[derive(Clone)]
pub struct ScanController {
    cancel_flag: Arc<AtomicBool>,
    progress_callback: Option<Arc<dyn Fn(EventLogProgress) + Send + Sync>>,
}

impl ScanController {
    pub fn new(
        cancel_flag: Arc<AtomicBool>,
        progress_callback: Option<Arc<dyn Fn(EventLogProgress) + Send + Sync>>,
    ) -> Self {
        Self {
            cancel_flag,
            progress_callback,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
    }

    pub fn report(&self, progress: EventLogProgress) {
        if let Some(callback) = &self.progress_callback {
            callback(progress);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PreviewSnapshot {
    pub records: Vec<EventRecord>,
    pub analytics: AnalyticsSnapshot,
    pub scanned_records: usize,
    pub event_id_counts: BTreeMap<u32, usize>,
    pub providers: Vec<String>,
    pub machine_names: Vec<String>,
    pub duration_ms: u128,
}

pub trait EventLogService: Send + Sync {
    fn scan(&self, query: &EventLogQuery, controller: ScanController) -> Result<PreviewSnapshot>;
}

pub fn default_event_log_service() -> Box<dyn EventLogService> {
    #[cfg(target_os = "windows")]
    {
        Box::new(WindowsEventLogService)
    }

    #[cfg(not(target_os = "windows"))]
    {
        Box::new(StubEventLogService)
    }
}

#[cfg(target_os = "windows")]
pub struct WindowsEventLogService;

#[cfg(target_os = "windows")]
impl EventLogService for WindowsEventLogService {
    fn scan(&self, query: &EventLogQuery, controller: ScanController) -> Result<PreviewSnapshot> {
        read_windows_event_log(query, controller)
    }
}

#[cfg(not(target_os = "windows"))]
#[derive(Default)]
pub struct StubEventLogService;

#[cfg(not(target_os = "windows"))]
impl EventLogService for StubEventLogService {
    fn scan(&self, _query: &EventLogQuery, _controller: ScanController) -> Result<PreviewSnapshot> {
        Err(anyhow!(
            "Gerçek Event Log okuma yalnızca Windows üzerinde destekleniyor"
        ))
    }
}

#[cfg(target_os = "windows")]
use chrono::{DateTime, Local, Utc};
#[cfg(target_os = "windows")]
use std::{mem::size_of, ptr};
#[cfg(target_os = "windows")]
use windows::{
    Win32::{
        Foundation::{ERROR_HANDLE_EOF, ERROR_INSUFFICIENT_BUFFER, GetLastError, HANDLE},
        System::EventLog::{
            CloseEventLog, EVENTLOG_AUDIT_FAILURE, EVENTLOG_AUDIT_SUCCESS, EVENTLOG_ERROR_TYPE,
            EVENTLOG_INFORMATION_TYPE, EVENTLOG_SEQUENTIAL_READ, EVENTLOG_SUCCESS,
            EVENTLOG_WARNING_TYPE, EVENTLOGRECORD, OpenEventLogW, READ_EVENT_LOG_READ_FLAGS,
            REPORT_EVENT_TYPE, ReadEventLogW,
        },
    },
    core::{HSTRING, PCWSTR},
};

#[cfg(target_os = "windows")]
const EVENTLOG_BACKWARDS_READ_FLAG: u32 = 0x0008;

#[cfg(target_os = "windows")]
fn read_windows_event_log(
    query: &EventLogQuery,
    controller: ScanController,
) -> Result<PreviewSnapshot> {
    let started_at = Instant::now();
    let handle = EventLogHandle::open(query.source.as_str())?;
    let filter_ids: HashSet<u32> = query.event_ids.iter().copied().collect();
    let flags =
        READ_EVENT_LOG_READ_FLAGS(EVENTLOG_SEQUENTIAL_READ.0 | EVENTLOG_BACKWARDS_READ_FLAG);

    let mut buffer = vec![0_u8; 64 * 1024];
    let mut scanned_total = 0usize;
    let mut matched_total = 0usize;
    let mut records = Vec::new();
    let mut event_id_counts = BTreeMap::new();
    let mut providers = BTreeSet::new();
    let mut machine_names = BTreeSet::new();

    loop {
        if controller.is_cancelled() {
            return Err(anyhow!("Tarama kullanıcı tarafından iptal edildi"));
        }

        let mut bytes_read = 0u32;
        let mut bytes_needed = 0u32;

        let read_result = unsafe {
            ReadEventLogW(
                handle.0,
                flags,
                0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut bytes_read,
                &mut bytes_needed,
            )
        };

        match read_result {
            Ok(()) => {
                let mut offset = 0usize;
                let chunk = &buffer[..bytes_read as usize];

                while offset + size_of::<EVENTLOGRECORD>() <= chunk.len() {
                    if controller.is_cancelled() {
                        return Err(anyhow!("Tarama kullanıcı tarafından iptal edildi"));
                    }

                    let Some(record) = read_record_header(chunk, offset) else {
                        break;
                    };
                    let record_len = record.Length as usize;
                    if record_len == 0 || offset + record_len > chunk.len() {
                        break;
                    }

                    scanned_total += 1;
                    let event_id = record.EventID & 0xFFFF;
                    if filter_ids.contains(&event_id) {
                        matched_total += 1;
                        *event_id_counts.entry(event_id).or_insert(0) += 1;

                        let raw_record = &chunk[offset..offset + record_len];
                        let parsed = parse_record(raw_record, &record)?;
                        if !parsed.provider.is_empty() {
                            providers.insert(parsed.provider.clone());
                        }
                        if !parsed.computer.is_empty() {
                            machine_names.insert(parsed.computer.clone());
                        }

                        if query.max_events.is_none_or(|limit| records.len() < limit) {
                            records.push(parsed);
                        }
                    }

                    if scanned_total.is_multiple_of(250) {
                        controller.report(EventLogProgress {
                            current_log: query.source.display_name().to_owned(),
                            scanned_records: scanned_total,
                            matched_records: matched_total,
                        });
                    }

                    offset += record_len;
                }
            }
            Err(error) => {
                let error_code = unsafe { GetLastError() };
                if error_code == ERROR_HANDLE_EOF {
                    break;
                }

                if error_code == ERROR_INSUFFICIENT_BUFFER && bytes_needed > buffer.len() as u32 {
                    buffer.resize(bytes_needed as usize, 0);
                    continue;
                }

                return Err(anyhow!(error))
                    .with_context(|| format!("{} günlüğü okunamadı", query.source.display_name()));
            }
        }
    }

    controller.report(EventLogProgress {
        current_log: query.source.display_name().to_owned(),
        scanned_records: scanned_total,
        matched_records: matched_total,
    });

    let queue_size = query
        .max_events
        .map(|limit| matched_total.min(limit))
        .unwrap_or(matched_total);

    Ok(PreviewSnapshot {
        records,
        analytics: AnalyticsSnapshot {
            total_logs_found: matched_total,
            queue_size,
        },
        scanned_records: scanned_total,
        event_id_counts,
        providers: providers.into_iter().collect(),
        machine_names: machine_names.into_iter().collect(),
        duration_ms: started_at.elapsed().as_millis(),
    })
}

#[cfg(target_os = "windows")]
fn read_record_header(chunk: &[u8], offset: usize) -> Option<EVENTLOGRECORD> {
    if offset + size_of::<EVENTLOGRECORD>() > chunk.len() {
        return None;
    }

    let record_ptr = unsafe { chunk.as_ptr().add(offset).cast::<EVENTLOGRECORD>() };
    Some(unsafe { ptr::read_unaligned(record_ptr) })
}

#[cfg(target_os = "windows")]
fn parse_record(raw_record: &[u8], header: &EVENTLOGRECORD) -> Result<EventRecord> {
    let event_id = header.EventID & 0xFFFF;
    let (provider, next_offset) = read_utf16_c_string(raw_record, size_of::<EVENTLOGRECORD>());
    let (computer, _) = read_utf16_c_string(raw_record, next_offset);
    let insertion_strings = read_insertion_strings(raw_record, header);
    let message = if insertion_strings.is_empty() {
        format!(
            "{} kaynağındaki Event ID {} için ileti çözümlenemedi.",
            if provider.is_empty() {
                "Bilinmeyen"
            } else {
                &provider
            },
            event_id
        )
    } else {
        insertion_strings.join(" | ")
    };

    Ok(EventRecord {
        event_id,
        provider: if provider.is_empty() {
            "Bilinmeyen".to_owned()
        } else {
            provider
        },
        level: level_name(header.EventType).to_owned(),
        computer: if computer.is_empty() {
            "Bilinmeyen".to_owned()
        } else {
            computer
        },
        created_at: format_timestamp(header.TimeGenerated),
        message,
    })
}

#[cfg(target_os = "windows")]
fn read_insertion_strings(raw_record: &[u8], header: &EVENTLOGRECORD) -> Vec<String> {
    let mut strings = Vec::with_capacity(header.NumStrings as usize);
    let mut offset = header.StringOffset as usize;

    for _ in 0..header.NumStrings {
        if offset >= raw_record.len() {
            break;
        }

        let (value, next_offset) = read_utf16_c_string(raw_record, offset);
        if !value.is_empty() {
            strings.push(value);
        }
        offset = next_offset;
    }

    strings
}

#[cfg(target_os = "windows")]
fn read_utf16_c_string(buffer: &[u8], start: usize) -> (String, usize) {
    if start >= buffer.len() {
        return (String::new(), start);
    }

    let mut offset = start;
    let mut units = Vec::new();

    while offset + 1 < buffer.len() {
        let value = u16::from_le_bytes([buffer[offset], buffer[offset + 1]]);
        offset += 2;
        if value == 0 {
            break;
        }
        units.push(value);
    }

    (String::from_utf16_lossy(&units), offset)
}

#[cfg(target_os = "windows")]
fn level_name(event_type: REPORT_EVENT_TYPE) -> &'static str {
    match event_type {
        EVENTLOG_ERROR_TYPE => "Hata",
        EVENTLOG_WARNING_TYPE => "Uyarı",
        EVENTLOG_INFORMATION_TYPE => "Bilgi",
        EVENTLOG_AUDIT_SUCCESS => "Denetim Başarılı",
        EVENTLOG_AUDIT_FAILURE => "Denetim Başarısız",
        EVENTLOG_SUCCESS => "Başarılı",
        _ => "Bilinmiyor",
    }
}

#[cfg(target_os = "windows")]
fn format_timestamp(seconds_since_epoch: u32) -> String {
    DateTime::<Utc>::from_timestamp(seconds_since_epoch as i64, 0)
        .map(|value| value.with_timezone(&Local).to_rfc3339())
        .unwrap_or_else(|| seconds_since_epoch.to_string())
}

#[cfg(target_os = "windows")]
struct EventLogHandle(HANDLE);

#[cfg(target_os = "windows")]
impl EventLogHandle {
    fn open(source: &str) -> Result<Self> {
        let source_name = HSTRING::from(source);
        let handle = unsafe { OpenEventLogW(PCWSTR::null(), &source_name) }
            .with_context(|| format!("{} günlüğü açılamadı", source))?;
        Ok(Self(handle))
    }
}

#[cfg(target_os = "windows")]
impl Drop for EventLogHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseEventLog(self.0) };
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::{
        EVENTLOGRECORD, ScanController, format_timestamp, read_record_header, read_utf16_c_string,
    };
    use std::{
        mem::size_of,
        sync::{Arc, atomic::AtomicBool},
    };

    #[test]
    fn utf16_string_reader_advances_to_next_string() {
        let bytes = [
            b'T', 0, b'e', 0, b's', 0, b't', 0, 0, 0, b'O', 0, b'K', 0, 0, 0,
        ];

        let (first, offset) = read_utf16_c_string(&bytes, 0);
        let (second, _) = read_utf16_c_string(&bytes, offset);

        assert_eq!(first, "Test");
        assert_eq!(second, "OK");
    }

    #[test]
    fn timestamp_is_rendered_as_rfc3339() {
        assert!(format_timestamp(1_711_234_567).contains('T'));
    }

    #[test]
    fn scan_controller_starts_not_cancelled() {
        let controller = ScanController::new(Arc::new(AtomicBool::new(false)), None);
        assert!(!controller.is_cancelled());
    }

    #[test]
    fn record_header_can_be_read_from_unaligned_buffer() {
        let mut bytes = vec![0_u8; size_of::<EVENTLOGRECORD>() + 1];
        bytes[1..5].copy_from_slice(&(56_u32).to_le_bytes());

        let header = read_record_header(&bytes, 1).expect("header should be readable");

        assert_eq!(header.Length, 56);
    }
}
