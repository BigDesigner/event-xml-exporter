use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local};
use quick_xml::{
    Writer,
    events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event},
};
use std::{
    env, fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    domain::{EventRecord, ExportSettings},
    platform::PreviewSnapshot,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportMetadata {
    pub machine_names: Vec<String>,
    pub providers: Vec<String>,
    pub record_count: usize,
    pub filter_ids: Vec<u32>,
    pub export_duration_ms: u128,
    pub scanned_records: usize,
}

pub fn metadata_from_snapshot(
    settings: &ExportSettings,
    snapshot: &PreviewSnapshot,
    export_duration_ms: u128,
) -> ExportMetadata {
    ExportMetadata {
        machine_names: snapshot.machine_names.clone(),
        providers: snapshot.providers.clone(),
        record_count: snapshot.analytics.total_logs_found,
        filter_ids: settings.selected_event_ids.clone(),
        export_duration_ms,
        scanned_records: snapshot.scanned_records,
    }
}

pub fn default_output_path(now: DateTime<Local>) -> PathBuf {
    let base_dir = env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join("Desktop"))
        .filter(|desktop| desktop.exists())
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    base_dir.join(default_file_name(now))
}

pub fn default_file_name(now: DateTime<Local>) -> String {
    format!("GNN_Export_{}.xml", now.format("%Y%m%d_%H%M%S"))
}

pub fn unique_output_path(path: &Path, now: DateTime<Local>) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("GNN_Export");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("xml");

    parent.join(format!(
        "{}_{}.{}",
        stem,
        now.format("%Y%m%d_%H%M%S"),
        extension
    ))
}

pub fn resolve_export_path(path: &Path, now: DateTime<Local>) -> PathBuf {
    let refreshed = refresh_timestamped_name(path, now).unwrap_or_else(|| path.to_path_buf());

    if !refreshed.exists() {
        return refreshed;
    }

    unique_output_path(&refreshed, now)
}

pub fn build_xml_document(
    settings: &ExportSettings,
    records: &[EventRecord],
    exported_at: DateTime<Local>,
    metadata: &ExportMetadata,
) -> Result<String> {
    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

    let mut root = BytesStart::new("events");
    root.push_attribute(("source", settings.source.as_str()));
    let exported_at_text = exported_at.to_rfc3339();
    root.push_attribute(("exported_at", exported_at_text.as_str()));
    writer.write_event(Event::Start(root))?;

    writer.write_event(Event::Start(BytesStart::new("metadata")))?;
    write_text_element(
        &mut writer,
        "selection_count",
        &settings.selected_event_ids.len().to_string(),
    )?;
    write_text_element(
        &mut writer,
        "limit_mode",
        if settings.export_all {
            "all_matching"
        } else {
            "limited"
        },
    )?;
    write_text_element(
        &mut writer,
        "max_events",
        &settings
            .effective_max_events()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unlimited".to_owned()),
    )?;
    write_text_element(
        &mut writer,
        "record_count",
        &metadata.record_count.to_string(),
    )?;
    write_text_element(
        &mut writer,
        "scanned_records",
        &metadata.scanned_records.to_string(),
    )?;
    write_text_element(
        &mut writer,
        "filter_ids",
        &metadata
            .filter_ids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(","),
    )?;
    write_text_element(
        &mut writer,
        "machine_names",
        &metadata.machine_names.join(","),
    )?;
    write_text_element(&mut writer, "providers", &metadata.providers.join(","))?;
    write_text_element(
        &mut writer,
        "export_duration_ms",
        &metadata.export_duration_ms.to_string(),
    )?;
    writer.write_event(Event::End(BytesEnd::new("metadata")))?;

    writer.write_event(Event::Start(BytesStart::new("records")))?;
    for record in records {
        let mut event = BytesStart::new("event");
        let event_id_text = record.event_id.to_string();
        event.push_attribute(("id", event_id_text.as_str()));
        writer.write_event(Event::Start(event))?;
        write_text_element(&mut writer, "provider", &record.provider)?;
        write_text_element(&mut writer, "level", &record.level)?;
        write_text_element(&mut writer, "computer", &record.computer)?;
        write_text_element(&mut writer, "created_at", &record.created_at)?;
        write_text_element(&mut writer, "message", &record.message)?;
        writer.write_event(Event::End(BytesEnd::new("event")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("records")))?;
    writer.write_event(Event::End(BytesEnd::new("events")))?;

    String::from_utf8(writer.into_inner().into_inner()).context("XML çıktısı UTF-8 üretilemedi")
}

pub fn write_xml_file(path: &Path, xml: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Hedef klasör çözümlenemedi"))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Klasör oluşturulamadı: {}", parent.display()))?;
    fs::write(path, xml).with_context(|| format!("XML dosyası yazılamadı: {}", path.display()))
}

pub fn open_file(path: &Path) -> Result<()> {
    ensure_path_exists(path)?;

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path.display().to_string()])
            .spawn()
            .with_context(|| format!("Dosya açılamadı: {}", path.display()))?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        bail!("Dosya açma işlemi yalnızca Windows üzerinde destekleniyor")
    }
}

pub fn open_folder(path: &Path) -> Result<()> {
    ensure_path_exists(path)?;
    let folder = path
        .parent()
        .ok_or_else(|| anyhow!("Dosya klasörü çözümlenemedi"))?;

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(folder)
            .spawn()
            .with_context(|| format!("Klasör açılamadı: {}", folder.display()))?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        bail!("Klasör açma işlemi yalnızca Windows üzerinde destekleniyor")
    }
}

fn write_text_element(writer: &mut Writer<Cursor<Vec<u8>>>, name: &str, value: &str) -> Result<()> {
    writer.write_event(Event::Start(BytesStart::new(name)))?;
    writer.write_event(Event::Text(BytesText::new(value)))?;
    writer.write_event(Event::End(BytesEnd::new(name)))?;
    Ok(())
}

fn ensure_path_exists(path: &Path) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        bail!("İstenen yol bulunamadı: {}", path.display())
    }
}

fn refresh_timestamped_name(path: &Path, now: DateTime<Local>) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_str()?;
    let base_stem = strip_timestamp_suffix(stem)?;
    Some(build_path_with_stem(path, base_stem, now))
}

fn strip_timestamp_suffix(stem: &str) -> Option<&str> {
    if stem.len() <= 16 {
        return None;
    }

    let split_at = stem.len() - 16;
    let suffix = &stem[split_at..];
    let bytes = suffix.as_bytes();

    let is_timestamp = bytes.first() == Some(&b'_')
        && bytes.get(9) == Some(&b'_')
        && bytes[1..9].iter().all(u8::is_ascii_digit)
        && bytes[10..16].iter().all(u8::is_ascii_digit);

    if is_timestamp {
        Some(&stem[..split_at])
    } else {
        None
    }
}

fn build_path_with_stem(path: &Path, stem: &str, now: DateTime<Local>) -> PathBuf {
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let extension = path.extension().and_then(|value| value.to_str());
    let timestamp = now.format("%Y%m%d_%H%M%S");

    match extension {
        Some(extension) => parent.join(format!("{stem}_{timestamp}.{extension}")),
        None => parent.join(format!("{stem}_{timestamp}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExportMetadata, build_xml_document, default_file_name, metadata_from_snapshot,
        resolve_export_path, unique_output_path, write_xml_file,
    };
    use crate::{
        domain::{EventRecord, ExportSettings, LogSource},
        platform::PreviewSnapshot,
    };
    use chrono::{Local, TimeZone};
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn xml_document_contains_root_metadata() {
        let settings = ExportSettings {
            source: LogSource::System,
            max_events: Some(50),
            export_all: false,
            output_path: PathBuf::from("demo.xml"),
            selected_event_ids: vec![41, 55],
        };

        let records = vec![EventRecord {
            event_id: 41,
            provider: "Kernel-Power".to_owned(),
            level: "Error".to_owned(),
            computer: "LAB-PC".to_owned(),
            created_at: "2026-03-24T01:27:36+03:00".to_owned(),
            message: "Sistem beklenmedik bir şekilde kapandı.".to_owned(),
        }];

        let metadata = ExportMetadata {
            machine_names: vec!["LAB-PC".to_owned()],
            providers: vec!["Kernel-Power".to_owned()],
            record_count: 1,
            filter_ids: vec![41, 55],
            export_duration_ms: 145,
            scanned_records: 30,
        };

        let xml = build_xml_document(
            &settings,
            &records,
            Local.with_ymd_and_hms(2026, 3, 24, 1, 27, 36).unwrap(),
            &metadata,
        )
        .expect("xml should be created");

        assert!(xml.contains("source=\"System\""));
        assert!(xml.contains("exported_at=\"2026-03-24T01:27:36"));
        assert!(xml.contains("<record_count>1</record_count>"));
        assert!(xml.contains("<filter_ids>41,55</filter_ids>"));
    }

    #[test]
    fn metadata_from_snapshot_collects_summary_fields() {
        let settings = ExportSettings {
            source: LogSource::System,
            max_events: Some(50),
            export_all: false,
            output_path: PathBuf::from("demo.xml"),
            selected_event_ids: vec![41, 55],
        };

        let snapshot = PreviewSnapshot {
            records: vec![],
            analytics: Default::default(),
            scanned_records: 90,
            event_id_counts: BTreeMap::new(),
            providers: vec!["Kernel-Power".to_owned()],
            machine_names: vec!["LAB-PC".to_owned()],
            duration_ms: 33,
        };

        let metadata = metadata_from_snapshot(&settings, &snapshot, 88);
        assert_eq!(metadata.scanned_records, 90);
        assert_eq!(metadata.providers, vec!["Kernel-Power"]);
        assert_eq!(metadata.filter_ids, vec![41, 55]);
        assert_eq!(metadata.export_duration_ms, 88);
    }

    #[test]
    fn default_file_name_uses_expected_timestamp_pattern() {
        let timestamp = Local.with_ymd_and_hms(2026, 3, 24, 1, 27, 36).unwrap();
        assert_eq!(
            default_file_name(timestamp),
            "GNN_Export_20260324_012736.xml"
        );
    }

    #[test]
    fn unique_output_path_adds_suffix_when_file_exists() {
        let base_dir = std::env::temp_dir().join(format!(
            "event_xml_exporter_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));

        fs::create_dir_all(&base_dir).expect("temp directory should be created");
        let existing_path = base_dir.join("demo.xml");
        fs::write(&existing_path, "demo").expect("temp file should be written");

        let unique = unique_output_path(
            &existing_path,
            Local.with_ymd_and_hms(2026, 3, 24, 1, 27, 36).unwrap(),
        );

        assert_eq!(
            unique.file_name().and_then(|name| name.to_str()),
            Some("demo_20260324_012736.xml")
        );

        fs::remove_dir_all(base_dir).expect("temp directory should be removed");
    }

    #[test]
    fn resolve_export_path_refreshes_existing_timestamp_without_chaining() {
        let path = PathBuf::from(r"C:\Temp\GNN_Export_20260324_024734.xml");
        let resolved = resolve_export_path(
            &path,
            Local.with_ymd_and_hms(2026, 3, 24, 2, 48, 14).unwrap(),
        );

        assert_eq!(
            resolved.file_name().and_then(|name| name.to_str()),
            Some("GNN_Export_20260324_024814.xml")
        );
    }

    #[test]
    fn write_xml_file_creates_parent_directories() {
        let base_dir = std::env::temp_dir().join(format!(
            "event_xml_exporter_write_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        let file_path = base_dir.join("nested").join("result.xml");

        write_xml_file(&file_path, "<events />").expect("xml should be written");

        assert_eq!(
            fs::read_to_string(&file_path).expect("file should exist"),
            "<events />"
        );

        fs::remove_dir_all(base_dir).expect("temp directory should be removed");
    }
}
