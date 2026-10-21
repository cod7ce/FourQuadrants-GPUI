//! JSON persistence for FourQuadrants GPUI.
//!
//! Saves are atomic (same-directory temp file + rename), keep one `.bak`
//! copy of the previous good file, and surface every filesystem error. A file
//! that cannot be parsed or validated is never replaced, either by `load`
//! initialization or by `save`.

use anyhow::{Context, Result, bail, ensure};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::model::Document;

/// Location of the application's JSON data store.
#[derive(Debug)]
pub struct Store {
    pub path: PathBuf,
    /// Fingerprint of the bytes observed when this Store was opened/loaded or
    /// last saved. A mismatch at save time means another instance changed the
    /// file, so the stale writer must fail visibly instead of clobbering it.
    baseline: Arc<Mutex<Option<String>>>,
}

impl Clone for Store {
    fn clone(&self) -> Self {
        // A clone is a distinct optimistic-concurrency participant: it starts
        // from the current baseline snapshot, not a shared mutable baseline.
        let baseline = self
            .baseline
            .lock()
            .map(|value| value.clone())
            .unwrap_or(None);
        Self {
            path: self.path.clone(),
            baseline: Arc::new(Mutex::new(baseline)),
        }
    }
}

impl PartialEq for Store {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for Store {}

impl Store {
    /// Opens `~/Library/Application Support/FourQuadrants GPUI/data.json`.
    /// A missing file is initialized with a default document. An existing
    /// unreadable file returns an error and is left byte-for-byte untouched.
    pub fn open_default() -> Result<(Store, Document)> {
        open_store(Store::at(default_data_path()?))
    }

    pub fn at(path: PathBuf) -> Self {
        let baseline = read_optional(&path)
            .ok()
            .flatten()
            .map(|bytes| fingerprint_bytes(&bytes));
        Self {
            path,
            baseline: Arc::new(Mutex::new(baseline)),
        }
    }

    /// Loads and validates the document, then records its fingerprint as the
    /// optimistic-concurrency baseline. Never writes to the file.
    pub fn load(&self) -> Result<Document> {
        let (document, fingerprint) = read_document(&self.path)?;
        self.set_baseline(Some(fingerprint))?;
        Ok(document)
    }

    /// Atomically replaces the data file. The previous valid content is copied
    /// to `<data.json>.bak` first. Saving is refused if the existing file is
    /// unreadable or invalid, or if another Store instance changed it since
    /// this instance loaded it, so neither corruption nor a stale write can be
    /// silently overwritten.
    ///
    /// A persistent `<data.json>.lock` sibling file is held with `File::lock`
    /// for the whole save, eliminating the check-then-rename race between
    /// cooperating Store instances. The lock file is intentionally never
    /// deleted: removing it would allow a new inode to be locked while an old
    /// descriptor is still active.
    pub fn save(&self, document: &Document) -> Result<()> {
        // Capture this call's baseline before waiting for the filesystem lock.
        // Another save on this same Store must not refresh a waiting writer's
        // expected version underneath it.
        let expected = self.baseline()?;
        document
            .validate()
            .context("refusing to save an invalid document")?;

        let parent = parent_dir(&self.path);
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create data directory {}", parent.display()))?;
        let _save_lock = acquire_save_lock(&self.path)?;

        let current_bytes = read_optional(&self.path)?;
        let current = current_bytes.as_deref().map(fingerprint_bytes);
        if current != expected {
            bail!(
                "data file {} changed on disk since this Store was opened; refusing to overwrite (reload and retry)",
                self.path.display()
            );
        }

        if let Some(existing) = current_bytes
            && let Err(error) = parse_document(&existing, &self.path)
        {
            bail!(
                "refusing to overwrite unreadable data file {}: {error:#}",
                self.path.display()
            );
        }

        let bytes = document_bytes(document)?;

        // External, non-cooperating writers can still bypass the lock; narrow
        // the remaining window before the atomic rename.
        let latest = read_optional(&self.path)?;
        let latest_fingerprint = latest.as_deref().map(fingerprint_bytes);
        if latest_fingerprint != current {
            bail!(
                "data file {} changed on disk during save; refusing to overwrite",
                self.path.display()
            );
        }

        atomic_write(&self.path, &bytes, true)
            .with_context(|| format!("failed to save data file {}", self.path.display()))?;
        self.set_baseline(Some(fingerprint_bytes(&bytes)))
            .context("saved data but failed to update in-memory store baseline")?;
        Ok(())
    }

    /// The backup written before the most recent successful save.
    pub fn backup_path(&self) -> PathBuf {
        backup_path_for(&self.path)
    }

    fn baseline(&self) -> Result<Option<String>> {
        self.baseline
            .lock()
            .map(|value| value.clone())
            .map_err(|_| anyhow::anyhow!("store baseline lock was poisoned"))
    }

    fn set_baseline(&self, value: Option<String>) -> Result<()> {
        *self
            .baseline
            .lock()
            .map_err(|_| anyhow::anyhow!("store baseline lock was poisoned"))? = value;
        Ok(())
    }
}

/// Default data path: `~/Library/Application Support/FourQuadrants GPUI/data.json`.
pub fn default_data_path() -> Result<PathBuf> {
    let home = env::var_os("HOME")
        .ok_or_else(|| anyhow::anyhow!("HOME is not set; cannot locate Application Support"))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("FourQuadrants GPUI")
        .join("data.json"))
}

/// Reads, parses and validates a document without merging it.
pub fn import(path: &Path) -> Result<Document> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read import file {}", path.display()))?;
    parse_document(&bytes, path).with_context(|| format!("failed to import {}", path.display()))
}

/// Writes a document as pretty JSON atomically. This is intended for explicit
/// export destinations; it does not create a `.bak` file.
pub fn export(path: &Path, document: &Document) -> Result<()> {
    document
        .validate()
        .context("refusing to export an invalid document")?;
    let bytes = document_bytes(document)?;
    atomic_write(path, &bytes, false)
        .with_context(|| format!("failed to export JSON to {}", path.display()))
}

fn open_store(store: Store) -> Result<(Store, Document)> {
    if store.path.exists() {
        let document = store
            .load()
            .with_context(|| format!("cannot open data file {}", store.path.display()))?;
        Ok((store, document))
    } else {
        let document = Document::default();
        store
            .save(&document)
            .with_context(|| format!("cannot initialize data file {}", store.path.display()))?;
        Ok((store, document))
    }
}

fn read_document(path: &Path) -> Result<(Document, String)> {
    let bytes = read_optional(path)?.ok_or_else(|| {
        anyhow::anyhow!(
            "data file not found at {} (Store::open_default initializes a new one)",
            path.display()
        )
    })?;
    let fingerprint = fingerprint_bytes(&bytes);
    let document = parse_document(&bytes, path)
        .with_context(|| format!("failed to load data file {}", path.display()))?;
    Ok((document, fingerprint))
}

/// Persistence formats must look like a real Document envelope. Unlike the
/// in-memory serde defaults, `{}` is not accepted as an empty document.
fn parse_document(bytes: &[u8], path: &Path) -> Result<Document> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    let object = value.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "invalid document in {}: top level must be an object",
            path.display()
        )
    })?;
    for field in ["schemaVersion", "tasks", "tags", "weekNotes"] {
        ensure!(
            object.contains_key(field),
            "invalid document in {}: missing required field {field}",
            path.display()
        );
    }
    ensure!(
        object
            .get("schemaVersion")
            .is_some_and(serde_json::Value::is_u64),
        "invalid document in {}: schemaVersion must be an unsigned integer",
        path.display()
    );
    for field in ["tasks", "tags", "weekNotes"] {
        ensure!(
            object.get(field).is_some_and(serde_json::Value::is_array),
            "invalid document in {}: {field} must be an array",
            path.display()
        );
    }

    let document: Document = serde_json::from_value(value)
        .with_context(|| format!("invalid document shape in {}", path.display()))?;
    document
        .validate()
        .with_context(|| format!("document validation failed for {}", path.display()))?;
    Ok(document)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read data file {}", path.display()))
        }
    }
}

fn fingerprint_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{}:{hash:016x}", bytes.len())
}

fn document_bytes(document: &Document) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(document).context("failed to serialize document as JSON")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn backup_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "data.json".to_string());
    path.with_file_name(format!("{file_name}.bak"))
}

fn lock_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    path.with_file_name(name)
}

/// Return the locked file so its descriptor holds the lock until save exits.
/// Never unlink this file: all writers must continue to lock the same inode.
fn acquire_save_lock(path: &Path) -> Result<File> {
    let lock_path = lock_path_for(path);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open data lock {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("failed to acquire data lock {}", lock_path.display()))?;
    Ok(file)
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Writes `bytes` to a sibling temp file, fsyncs it, optionally backs up the
/// current file, then renames the temp file over the destination.
fn atomic_write(path: &Path, bytes: &[u8], backup_existing: bool) -> Result<()> {
    let parent = parent_dir(path);
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create directory {}", parent.display()))?;
    let temp = temp_sibling(path, "tmp")?;

    let result = (|| -> Result<()> {
        write_new_file(&temp, bytes)?;

        if backup_existing && path.exists() {
            let previous = fs::read(path)
                .with_context(|| format!("failed to read previous file {}", path.display()))?;
            let backup = backup_path_for(path);
            let backup_temp = temp_sibling(&backup, "tmp")?;
            let backup_result = (|| -> Result<()> {
                write_new_file(&backup_temp, &previous)?;
                fs::rename(&backup_temp, &backup)
                    .with_context(|| format!("failed to install backup {}", backup.display()))?;
                Ok(())
            })();
            if backup_result.is_err() {
                let _ = fs::remove_file(&backup_temp);
                backup_result?;
            }
        }

        fs::rename(&temp, path).with_context(|| format!("failed to replace {}", path.display()))?;
        // Directory fsync is best-effort: the rename is already durable on
        // macOS and a failure here must not report a successful save as failed.
        let _ = sync_directory(parent);
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn temp_sibling(path: &Path, tag: &str) -> Result<PathBuf> {
    let parent = parent_dir(path);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "data.json".to_string());
    Ok(parent.join(format!(".{file_name}.{tag}-{}", Uuid::new_v4())))
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("failed to create temporary file {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write temporary file {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to flush temporary file {}", path.display()))?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("failed to open directory {}", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to fsync directory {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Document, MarkdownNote, NoteAttachment, ProgressNote, Tag, WeekNote, week_start,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use chrono::NaiveDate;
    use std::sync::{Barrier, mpsc};
    use std::thread;
    use std::time::Duration;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path = env::temp_dir().join(format!("fourquadrants-gpui-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn temp_names(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn default_path_targets_application_support() {
        let path = default_data_path().unwrap();
        assert!(path.ends_with("Library/Application Support/FourQuadrants GPUI/data.json"));
    }

    #[test]
    fn save_load_roundtrip_creates_backup_and_no_temp_files() {
        let dir = TempDir::new();
        let path = dir.path.join("data.json");
        let store = Store::at(path.clone());

        let mut document = Document::default();
        let task_id = document.add_task("first", date(2026, 7, 6), 0);
        document.toggle_complete(&task_id);
        document.tags.push(Tag {
            id: "tag-1".to_string(),
            name: "Work".to_string(),
            color_hex: "#112233".to_string(),
            sort_order: 0.0,
        });
        document.tasks[0].tags.push("tag-1".to_string());
        document.week_notes.push(WeekNote {
            week_start: week_start(date(2026, 7, 6)),
            text: "note".to_string(),
            rich_content: None,
            markdown: None,
        });
        document.archived_week_notes.push(WeekNote {
            week_start: week_start(date(2026, 7, 6)),
            text: "archived conflict".to_string(),
            rich_content: Some("YXJjaGl2ZWQ=".to_string()),
            markdown: None,
        });
        document.orphan_progress_notes.push(ProgressNote {
            id: "orphan-1".to_string(),
            text: "orphan".to_string(),
            ..ProgressNote::default()
        });

        store.save(&document).unwrap();
        assert!(path.exists());
        assert!(!store.backup_path().exists());
        assert_eq!(store.load().unwrap(), document);

        document.settings.theme = "forest".to_string();
        document.add_task("second", date(2026, 7, 6), 2);
        store.save(&document).unwrap();
        assert!(store.backup_path().exists());
        assert_eq!(store.load().unwrap(), document);

        let previous = import(&store.backup_path()).unwrap();
        assert_eq!(previous.settings.theme, "system");
        assert_eq!(previous.tasks.len(), 1);

        let names = temp_names(&dir.path);
        assert!(names.iter().any(|name| name == "data.json"));
        assert!(names.iter().any(|name| name == "data.json.bak"));
        assert!(
            !names.iter().any(|name| name.contains(".tmp-")),
            "temporary files should be cleaned up: {names:?}"
        );
    }

    #[test]
    fn schema_one_load_and_save_preserve_version_and_exact_legacy_fields() {
        let dir = TempDir::new();
        let path = dir.path.join("legacy.json");
        let raw = r#"{
            "schemaVersion": 1,
            "tasks": [],
            "tags": [],
            "weekNotes": [{
                "weekStart": "2026-07-06",
                "text": "  legacy\r\n\t￼  ",
                "richContent": " bGVnYWN5\r\n"
            }],
            "archivedWeekNotes": [{
                "weekStart": "2026-07-06",
                "text": "  archive\r\n",
                "richContent": " YXJjaGl2ZQ== "
            }]
        }"#
        .as_bytes();
        fs::write(&path, raw).unwrap();
        let (store, document) = open_store(Store::at(path.clone())).unwrap();
        assert_eq!(fs::read(&path).unwrap(), raw);
        assert_eq!(document.schema_version, 1);
        assert_eq!(document.week_notes[0].text, "  legacy\r\n\t\u{fffc}  ");
        assert_eq!(
            document.week_notes[0].rich_content.as_deref(),
            Some(" bGVnYWN5\r\n")
        );
        assert!(document.week_notes[0].markdown.is_none());
        assert!(document.archived_week_notes[0].markdown.is_none());
        store.save(&document).unwrap();
        assert_eq!(store.load().unwrap(), document);
        assert_eq!(fs::read(store.backup_path()).unwrap(), raw);
        assert_eq!(import(&store.backup_path()).unwrap(), document);
    }

    #[test]
    fn markdown_roundtrip_keeps_original_bytes_and_both_schema_backups() {
        let dir = TempDir::new();
        let path = dir.path.join("data.json");
        let store = Store::at(path.clone());
        let mut document = Document {
            week_notes: vec![WeekNote {
                week_start: date(2026, 7, 6),
                text: "  preserved\r\n原文\t\u{fffc}  ".into(),
                rich_content: Some(" original RTFD bytes\r\n".into()),
                ..WeekNote::default()
            }],
            ..Document::default()
        };
        store.save(&document).unwrap();
        let original = document.clone();
        let schema_one_bytes = fs::read(&path).unwrap();

        document.schema_version = 2;
        document.week_notes[0].markdown = Some(MarkdownNote {
            source:
                "# Inline Markdown\n\n![image](attachment:12345678-abcd-4321-9876-123456789abc)"
                    .into(),
            attachments: vec![NoteAttachment {
                id: "12345678-abcd-4321-9876-123456789abc".into(),
                name: "原图.png".into(),
                mime_type: "image/png".into(),
                data: BASE64.encode([0, 255, 1, 13, 10]),
            }],
        });
        let mut archive = document.week_notes[0].clone();
        archive.markdown.as_mut().unwrap().source = "Archived Markdown".into();
        archive.markdown.as_mut().unwrap().attachments[0].mime_type =
            "application/octet-stream".into();
        document.archived_week_notes.push(archive);
        store.save(&document).unwrap();
        assert_eq!(store.load().unwrap(), document);
        assert_eq!(fs::read(store.backup_path()).unwrap(), schema_one_bytes);
        assert_eq!(Store::at(store.backup_path()).load().unwrap(), original);
        assert_eq!(document.week_notes[0].text, original.week_notes[0].text);
        assert_eq!(
            document.week_notes[0].rich_content,
            original.week_notes[0].rich_content
        );
        assert_eq!(
            BASE64
                .decode(
                    &store.load().unwrap().week_notes[0]
                        .markdown
                        .as_ref()
                        .unwrap()
                        .attachments[0]
                        .data
                )
                .unwrap(),
            [0, 255, 1, 13, 10]
        );

        let export_path = dir.path.join("markdown-export.json");
        export(&export_path, &document).unwrap();
        assert_eq!(import(&export_path).unwrap(), document);
        let first_markdown = document.clone();
        let first_markdown_bytes = fs::read(&path).unwrap();
        document.week_notes[0]
            .markdown
            .as_mut()
            .unwrap()
            .source
            .clear();
        store.save(&document).unwrap();
        assert_eq!(store.load().unwrap(), document);
        assert_eq!(fs::read(store.backup_path()).unwrap(), first_markdown_bytes);
        assert_eq!(
            Store::at(store.backup_path()).load().unwrap(),
            first_markdown
        );
    }

    #[test]
    fn invalid_markdown_is_rejected_on_load_import_save_and_export() {
        let dir = TempDir::new();
        let path = dir.path.join("data.json");
        let store = Store::at(path.clone());
        let good = Document::default();
        store.save(&good).unwrap();
        let original = fs::read(&path).unwrap();
        let invalid_path = dir.path.join("invalid.json");
        for archived in [false, true] {
            for invalid_schema in [false, true] {
                let mut invalid = Document {
                    schema_version: 2,
                    ..Document::default()
                };
                let note = WeekNote {
                    week_start: date(2026, 7, 6),
                    markdown: Some(MarkdownNote {
                        attachments: vec![NoteAttachment {
                            data: if invalid_schema {
                                BASE64.encode(b"valid bytes")
                            } else {
                                "bad base64!".into()
                            },
                            ..NoteAttachment::default()
                        }],
                        ..MarkdownNote::default()
                    }),
                    ..WeekNote::default()
                };
                if archived {
                    invalid.archived_week_notes.push(note);
                } else {
                    invalid.week_notes.push(note);
                }
                if invalid_schema {
                    invalid.schema_version = 1;
                }
                let expected = if invalid_schema {
                    "requires schema version 2"
                } else {
                    "invalid base64"
                };
                let error = store.save(&invalid).unwrap_err();
                assert!(format!("{error:#}").contains(expected));
                assert_eq!(fs::read(&path).unwrap(), original);
                assert!(!store.backup_path().exists());
                let export_path = dir.path.join("invalid-export.json");
                assert!(export(&export_path, &invalid).is_err());
                assert!(!export_path.exists());

                let bytes = document_bytes(&invalid).unwrap();
                fs::write(&invalid_path, &bytes).unwrap();
                assert!(format!("{:#}", import(&invalid_path).unwrap_err()).contains(expected));
                let invalid_store = Store::at(invalid_path.clone());
                assert!(format!("{:#}", invalid_store.load().unwrap_err()).contains(expected));
                assert!(format!("{:#}", invalid_store.save(&good).unwrap_err()).contains(expected));
                assert_eq!(fs::read(&invalid_path).unwrap(), bytes);
                assert!(!invalid_store.backup_path().exists());
            }
        }
    }

    #[test]
    fn corrupt_file_is_never_overwritten() {
        let dir = TempDir::new();
        let path = dir.path.join("data.json");
        let corrupt = b"{ this is not valid JSON".to_vec();
        fs::write(&path, &corrupt).unwrap();
        let store = Store::at(path.clone());

        let load_error = store.load().unwrap_err();
        assert!(format!("{load_error:#}").contains("failed to load data file"));
        assert_eq!(fs::read(&path).unwrap(), corrupt);

        let save_error = store.save(&Document::default()).unwrap_err();
        assert!(format!("{save_error:#}").contains("refusing to overwrite unreadable data file"));
        assert_eq!(fs::read(&path).unwrap(), corrupt);
        assert!(!store.backup_path().exists());
    }

    #[test]
    fn invalid_document_is_not_written() {
        let dir = TempDir::new();
        let path = dir.path.join("data.json");
        let store = Store::at(path.clone());
        let mut document = Document::default();
        let id = document.add_task("duplicate", date(2026, 7, 6), 0);
        document.tasks.push(document.task(&id).unwrap().clone());

        let error = store.save(&document).unwrap_err();
        assert!(format!("{error:#}").contains("invalid document"));
        assert!(!path.exists());
    }

    #[test]
    fn open_missing_initializes_but_corrupt_file_stays_untouched() {
        let dir = TempDir::new();
        let missing = dir.path.join("nested").join("data.json");
        let (store, document) = open_store(Store::at(missing.clone())).unwrap();
        assert!(missing.exists());
        assert_eq!(store.load().unwrap(), document);
        assert_eq!(document.schema_version, 1);

        let corrupt_path = dir.path.join("corrupt.json");
        let corrupt = b"not json".to_vec();
        fs::write(&corrupt_path, &corrupt).unwrap();
        let error = open_store(Store::at(corrupt_path.clone())).unwrap_err();
        assert!(format!("{error:#}").contains("cannot open data file"));
        assert_eq!(fs::read(&corrupt_path).unwrap(), corrupt);
    }

    #[test]
    fn import_and_export_roundtrip_and_validate() {
        let dir = TempDir::new();
        let export_path = dir.path.join("export").join("tasks.json");
        let mut document = Document::default();
        document.add_task("exported", date(2026, 7, 6), 1);
        document.settings.language = "en".to_string();

        export(&export_path, &document).unwrap();
        assert!(export_path.exists());
        let imported = import(&export_path).unwrap();
        assert_eq!(imported, document);

        let bad_path = dir.path.join("bad.json");
        fs::write(
            &bad_path,
            br#"{
                "schemaVersion": 1,
                "tasks": [{"id": "x", "parentId": "nope"}],
                "tags": [],
                "weekNotes": []
            }"#,
        )
        .unwrap();
        let error = import(&bad_path).unwrap_err();
        assert!(format!("{error:#}").contains("dangling parent"));
    }

    #[test]
    fn persistence_requires_a_document_envelope() {
        let dir = TempDir::new();
        let path = dir.path.join("candidate.json");

        for raw in [
            "{}",
            r#"{"settings":{}}"#,
            r#"{"schemaVersion":1,"tasks":[],"tags":[]}"#,
            r#"{"schemaVersion":"1","tasks":[],"tags":[],"weekNotes":[]}"#,
            r#"{"schemaVersion":1,"tasks":{},"tags":[],"weekNotes":[]}"#,
            r#"[]"#,
        ] {
            fs::write(&path, raw).unwrap();
            let error = import(&path).unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains("missing required field")
                    || message.contains("must be an array")
                    || message.contains("must be an unsigned integer")
                    || message.contains("top level must be an object"),
                "unexpected envelope error for {raw}: {message}"
            );
        }

        for raw in [
            r#"{"schemaVersion":1,"tasks":[],"tags":[],"weekNotes":[],"settings":{}}"#,
            r#"{"schemaVersion":1,"tasks":[],"tags":[],"weekNotes":[]}"#,
        ] {
            fs::write(&path, raw).unwrap();
            let document = import(&path).unwrap();
            assert_eq!(document.schema_version, 1);
            assert_eq!(document.settings.theme, "system");
        }

        fs::write(&path, "{}").unwrap();
        let store = Store::at(path.clone());
        let error = store.load().unwrap_err();
        assert!(format!("{error:#}").contains("missing required field schemaVersion"));
        let error = store.save(&Document::default()).unwrap_err();
        assert!(format!("{error:#}").contains("refusing to overwrite unreadable data file"));
        assert_eq!(fs::read(&path).unwrap(), b"{}");
    }

    #[test]
    fn stale_store_instances_cannot_overwrite_newer_saves() {
        let dir = TempDir::new();
        let path = dir.path.join("data.json");

        let mut first = Document::default();
        first.add_task("first", date(2026, 7, 6), 0);

        let store_a = Store::at(path.clone());
        let store_b = Store::at(path.clone());
        store_a.save(&first).unwrap();

        let error = store_b.save(&first).unwrap_err();
        assert!(format!("{error:#}").contains("changed on disk"));
        assert_eq!(import(&path).unwrap(), first);

        // Both instances load the same version; the first saves, the second is
        // stale and must fail rather than silently replace the first update.
        store_b.load().unwrap();
        let mut second = first.clone();
        second.add_task("second", date(2026, 7, 6), 1);
        store_a.save(&second).unwrap();
        let error = store_b.save(&first).unwrap_err();
        assert!(format!("{error:#}").contains("changed on disk"));
        assert_eq!(import(&path).unwrap(), second);

        // External writers are detected too, even when the bytes remain valid.
        store_b.load().unwrap();
        let mut external = second.clone();
        external.settings.theme = "terminal".to_string();
        fs::write(&path, document_bytes(&external).unwrap()).unwrap();
        let error = store_b.save(&second).unwrap_err();
        assert!(format!("{error:#}").contains("changed on disk"));
        assert_eq!(fs::read(&path).unwrap(), document_bytes(&external).unwrap());

        // Reloading refreshes the baseline and makes the instance usable.
        let loaded = store_b.load().unwrap();
        assert_eq!(loaded, external);
        let mut next = loaded;
        next.add_task("after reload", date(2026, 7, 6), 2);
        store_b.save(&next).unwrap();
        assert_eq!(import(&path).unwrap(), next);
    }

    #[test]
    fn concurrent_saves_from_one_baseline_have_exactly_one_winner() {
        // Exercise both first creation and replacement of an existing file.
        for initialized in [false, true] {
            let dir = TempDir::new();
            let path = dir.path.join("data.json");
            let initial = Document::default();
            if initialized {
                Store::at(path.clone()).save(&initial).unwrap();
            }
            let original_bytes = read_optional(&path).unwrap();
            let barrier = Arc::new(Barrier::new(3));
            let mut handles = Vec::new();

            for title in ["writer A", "writer B"] {
                // Both instances capture the same baseline before either saves.
                let store = Store::at(path.clone());
                let mut document = initial.clone();
                document.add_task(title, date(2026, 7, 6), 0);
                let barrier = Arc::clone(&barrier);
                handles.push(thread::spawn(move || {
                    barrier.wait();
                    let result = store.save(&document);
                    (document, result)
                }));
            }

            barrier.wait();
            let outcomes: Vec<_> = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect();
            assert_eq!(
                outcomes.iter().filter(|(_, result)| result.is_ok()).count(),
                1,
                "exactly one writer may commit from the shared baseline: {outcomes:?}"
            );
            let (winner, _) = outcomes.iter().find(|(_, result)| result.is_ok()).unwrap();
            assert_eq!(&import(&path).unwrap(), winner);
            let (_, loser) = outcomes.iter().find(|(_, result)| result.is_err()).unwrap();
            assert!(format!("{:#}", loser.as_ref().unwrap_err()).contains("changed on disk"));
            assert_eq!(
                read_optional(&backup_path_for(&path)).unwrap(),
                original_bytes
            );
            assert!(lock_path_for(&path).exists());
            assert!(
                !temp_names(&dir.path)
                    .iter()
                    .any(|name| name.contains(".tmp-"))
            );
        }
    }

    #[test]
    fn save_waits_for_lock_then_checks_disk_and_releases_on_error() {
        let dir = TempDir::new();
        let path = dir.path.join("data.json");
        let initial = Document::default();
        Store::at(path.clone()).save(&initial).unwrap();
        let store = Store::at(path.clone());
        let lock_path = lock_path_for(&path);
        let marker = b"persistent lock file";
        fs::write(&lock_path, marker).unwrap();
        let held_lock = acquire_save_lock(&path).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = store.save(&initial);
            result_tx.send(result).unwrap();
            store
        });

        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            matches!(
                result_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "save must wait while another descriptor holds the file lock"
        );

        // Simulate the current lock owner committing before the waiting save.
        let mut newer = Document::default();
        newer.add_task("lock owner's update", date(2026, 7, 6), 1);
        atomic_write(&path, &document_bytes(&newer).unwrap(), true).unwrap();
        drop(held_lock);

        let error = result_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap_err();
        assert!(format!("{error:#}").contains("changed on disk"));
        let store = worker.join().unwrap();
        assert_eq!(store.load().unwrap(), newer);
        assert_eq!(import(&store.backup_path()).unwrap(), Document::default());

        // The failed save released its descriptor; retrying can acquire it.
        let released = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        released.try_lock().unwrap();
        drop(released);
        newer.add_task("after reload", date(2026, 7, 6), 2);
        store.save(&newer).unwrap();
        assert_eq!(import(&path).unwrap(), newer);
        assert_eq!(fs::read(&lock_path).unwrap(), marker);
    }

    #[test]
    fn cloned_store_keeps_an_independent_stale_baseline() {
        let dir = TempDir::new();
        let path = dir.path.join("data.json");
        let original = Store::at(path.clone());
        let mut first = Document::default();
        first.add_task("first", date(2026, 7, 6), 0);
        original.save(&first).unwrap();

        let clone = original.clone();
        let mut second = first.clone();
        second.add_task("second", date(2026, 7, 6), 0);
        original.save(&second).unwrap();

        let error = clone.save(&first).unwrap_err();
        assert!(format!("{error:#}").contains("changed on disk"));
        assert_eq!(import(&path).unwrap(), second);
    }

    #[test]
    fn backup_can_be_loaded_as_a_document() {
        let dir = TempDir::new();
        let path = dir.path.join("data.json");
        let store = Store::at(path.clone());

        let mut first = Document::default();
        first.add_task("v1", date(2026, 7, 6), 0);
        store.save(&first).unwrap();

        let mut second = first.clone();
        second.add_task("v2", date(2026, 7, 6), 0);
        store.save(&second).unwrap();

        let backup = import(&store.backup_path()).unwrap();
        assert_eq!(backup, first);
        let reloaded = Store::at(store.backup_path()).load().unwrap();
        assert_eq!(reloaded, first);
    }
}
