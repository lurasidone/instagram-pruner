use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::util::truncate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteOutcome {
    Deleted,
    AlreadyGone,
    Forbidden,
    DryRun,
}

impl DeleteOutcome {
    /// Short tag for the activity log, e.g. `GONE` for items already deleted.
    pub fn log_prefix(self) -> &'static str {
        match self {
            Self::Deleted => "DEL",
            Self::AlreadyGone => "GONE",
            Self::Forbidden => "SKIP",
            Self::DryRun => "WOULD-DEL",
        }
    }

    /// Dry runs and live deletions live in separate trees so a clean-up never
    /// mixes the two when the archive is read back.
    pub fn bucket(self) -> HistoryBucket {
        match self {
            Self::DryRun => HistoryBucket::DryRun,
            Self::Deleted | Self::AlreadyGone | Self::Forbidden => HistoryBucket::Live,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryBucket {
    Live,
    DryRun,
}

impl HistoryBucket {
    pub const ALL: [Self; 2] = [Self::Live, Self::DryRun];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::DryRun => "dry-run",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryScope {
    #[default]
    All,
    Live,
    DryRun,
}

impl HistoryScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Live => "live",
            Self::DryRun => "dry-run",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Live,
            Self::Live => Self::DryRun,
            Self::DryRun => Self::All,
        }
    }

    fn buckets(self) -> &'static [HistoryBucket] {
        match self {
            Self::All => &HistoryBucket::ALL,
            Self::Live => &[HistoryBucket::Live],
            Self::DryRun => &[HistoryBucket::DryRun],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentMeta {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedRecord {
    pub deleted_at: DateTime<Utc>,
    pub outcome: DeleteOutcome,
    pub target_id: String,
    pub target_name: String,
    pub item_id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub author_id: String,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentMeta>,
    #[serde(default)]
    pub preview: String,
}

#[derive(Debug, Clone)]
pub struct HistoryStore {
    root: PathBuf,
}

impl HistoryStore {
    pub fn from_env() -> Self {
        match env::var("PRUNER_HISTORY_DIR") {
            Ok(dir) if !dir.trim().is_empty() => Self::with_root(dir),
            _ => Self::with_root("history"),
        }
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `live/2024/07/09` or `dry-run/2024/07/09`.
    fn day_dir(&self, bucket: HistoryBucket, day: NaiveDate) -> PathBuf {
        self.root
            .join(bucket.as_str())
            .join(format!("{:04}", day.year()))
            .join(format!("{:02}", day.month()))
            .join(format!("{:02}", day.day()))
    }

    fn day_messages_path(&self, bucket: HistoryBucket, day: NaiveDate) -> PathBuf {
        self.day_dir(bucket, day).join("messages.jsonl")
    }

    fn target_path(
        &self,
        bucket: HistoryBucket,
        day: NaiveDate,
        record: &DeletedRecord,
    ) -> PathBuf {
        self.day_dir(bucket, day)
            .join("channels")
            .join(target_file_name(&record.target_name, &record.target_id))
    }

    /// Records written before the per-bucket layout: `<day>.jsonl` at the root.
    fn legacy_day_path(&self, day: NaiveDate) -> PathBuf {
        self.root.join(format!("{}.jsonl", day.format("%Y-%m-%d")))
    }

    /// Each record is appended twice: to the day log and to the per-target shard.
    pub fn append(&self, record: &DeletedRecord) -> std::io::Result<()> {
        let bucket = record.outcome.bucket();
        let day = record.deleted_at.date_naive();
        fs::create_dir_all(self.day_dir(bucket, day).join("channels"))?;
        append_jsonl(&self.day_messages_path(bucket, day), record)?;
        append_jsonl(&self.target_path(bucket, day, record), record)?;
        Ok(())
    }

    pub fn list_days(&self, scope: HistoryScope) -> std::io::Result<Vec<NaiveDate>> {
        let mut days: BTreeSet<NaiveDate> = BTreeSet::new();

        for bucket in scope.buckets() {
            let bucket_root = self.root.join(bucket.as_str());
            if bucket_root.is_dir() {
                collect_days_under(&bucket_root, &mut days)?;
            }
        }

        if matches!(scope, HistoryScope::All | HistoryScope::Live) && self.root.is_dir() {
            for entry in fs::read_dir(&self.root)? {
                let name = entry?.file_name();
                let name = name.to_string_lossy();
                if let Some(day) = name
                    .strip_suffix(".jsonl")
                    .and_then(|stem| NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok())
                {
                    days.insert(day);
                }
            }
        }

        let mut out: Vec<NaiveDate> = days.into_iter().collect();
        out.sort_unstable_by(|a, b| b.cmp(a));
        Ok(out)
    }

    pub fn list_channels(
        &self,
        day: NaiveDate,
        scope: HistoryScope,
    ) -> std::io::Result<Vec<ChannelRef>> {
        let mut by_id: BTreeMap<String, ChannelRef> = BTreeMap::new();

        for bucket in scope.buckets() {
            let channels_dir = self.day_dir(*bucket, day).join("channels");
            if !channels_dir.is_dir() {
                continue;
            }
            for entry in fs::read_dir(&channels_dir)? {
                let name = entry?.file_name();
                let name = name.to_string_lossy();
                if let Some((label, id)) = name
                    .strip_suffix(".jsonl")
                    .and_then(|stem| stem.rsplit_once("__"))
                {
                    by_id.entry(id.to_string()).or_insert(ChannelRef {
                        id: id.to_string(),
                        name: label.replace('_', " "),
                    });
                }
            }
        }

        if by_id.is_empty() {
            for rec in self.load_day(day, scope, None)? {
                by_id.entry(rec.target_id.clone()).or_insert(ChannelRef {
                    id: rec.target_id.clone(),
                    name: rec.target_name.clone(),
                });
            }
        }

        Ok(by_id.into_values().collect())
    }

    pub fn load_day(
        &self,
        day: NaiveDate,
        scope: HistoryScope,
        target_id: Option<&str>,
    ) -> std::io::Result<Vec<DeletedRecord>> {
        let mut out = Vec::new();

        for bucket in scope.buckets() {
            if let Some(tid) = target_id {
                if let Some(path) = self.channel_shard(*bucket, day, tid)? {
                    out.extend(read_jsonl_file(&path)?);
                    continue;
                }
            }
            let path = self.day_messages_path(*bucket, day);
            if path.exists() {
                let mut recs = read_jsonl_file(&path)?;
                if let Some(tid) = target_id {
                    recs.retain(|r| r.target_id == tid);
                }
                out.extend(recs);
            }
        }

        if matches!(scope, HistoryScope::All | HistoryScope::Live) {
            let legacy = self.legacy_day_path(day);
            if legacy.exists() {
                let mut recs = read_jsonl_file(&legacy)?;
                if matches!(scope, HistoryScope::Live) {
                    recs.retain(|r| r.outcome != DeleteOutcome::DryRun);
                }
                if let Some(tid) = target_id {
                    recs.retain(|r| r.target_id == tid);
                }
                out.extend(recs);
            }
        }

        out.sort_by(|a, b| {
            a.deleted_at
                .cmp(&b.deleted_at)
                .then_with(|| a.item_id.cmp(&b.item_id))
        });
        out.dedup_by(|a, b| {
            a.item_id == b.item_id
                && a.target_id == b.target_id
                && a.outcome == b.outcome
                && a.deleted_at == b.deleted_at
        });
        Ok(out)
    }

    fn channel_shard(
        &self,
        bucket: HistoryBucket,
        day: NaiveDate,
        target_id: &str,
    ) -> std::io::Result<Option<PathBuf>> {
        let channels_dir = self.day_dir(bucket, day).join("channels");
        if !channels_dir.is_dir() {
            return Ok(None);
        }
        let suffix = format!("__{target_id}.jsonl");
        for entry in fs::read_dir(&channels_dir)? {
            let path = entry?.path();
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(&suffix))
            {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }
}

fn target_file_name(target_name: &str, target_id: &str) -> String {
    format!(
        "{}__{}.jsonl",
        sanitize_component(target_name),
        sanitize_component(target_id)
    )
}

fn append_jsonl(path: &Path, record: &DeletedRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    serde_json::to_writer(&mut file, record).map_err(json_io_err)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn read_jsonl_file(path: &Path) -> std::io::Result<Vec<DeletedRecord>> {
    let reader = BufReader::new(fs::File::open(path)?);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<DeletedRecord>(line) {
            Ok(record) => out.push(record),
            Err(e) => tracing::warn!("skipping corrupt history line in {}: {e}", path.display()),
        }
    }
    Ok(out)
}

/// Walks `<bucket>/<year>/<month>/<day>` and keeps the days that hold records.
fn collect_days_under(bucket_root: &Path, days: &mut BTreeSet<NaiveDate>) -> std::io::Result<()> {
    for year_dir in subdirs(bucket_root)? {
        let Some(year) = parse_dir_component(&year_dir) else {
            continue;
        };
        for month_dir in subdirs(&year_dir)? {
            let Some(month) = parse_dir_component::<u32>(&month_dir) else {
                continue;
            };
            for day_dir in subdirs(&month_dir)? {
                let Some(day) = parse_dir_component::<u32>(&day_dir) else {
                    continue;
                };
                let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
                    continue;
                };
                if day_dir.join("messages.jsonl").exists() || day_dir.join("channels").is_dir() {
                    days.insert(date);
                }
            }
        }
    }
    Ok(())
}

fn subdirs(path: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            out.push(entry.path());
        }
    }
    Ok(out)
}

fn parse_dir_component<T: std::str::FromStr>(dir: &Path) -> Option<T> {
    dir.file_name()?.to_string_lossy().parse().ok()
}

/// Keeps history file names inside a safe ASCII range.
fn sanitize_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(48));
    for c in s.chars().take(48) {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if c == '-' || c == '.' {
            out.push(c);
        } else if out.is_empty() {
            if !c.is_whitespace() {
                out.push('_');
            }
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        "target".to_string()
    } else {
        out.to_string()
    }
}

fn json_io_err(e: serde_json::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
}

pub fn text_preview(text: &str, fallback: &str) -> String {
    let content = text.trim();
    if content.is_empty() {
        truncate(fallback, 60)
    } else {
        truncate(content, 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch() -> (Tmp, HistoryStore) {
        let dir = std::env::temp_dir().join(format!("pruner-hist-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let store = HistoryStore::with_root(&dir);
        (Tmp(dir), store)
    }

    fn sample_record(id: &str, outcome: DeleteOutcome, target: &str) -> DeletedRecord {
        DeletedRecord {
            deleted_at: Utc::now(),
            outcome,
            target_id: target.into(),
            target_name: format!("name-{target}"),
            item_id: id.into(),
            kind: "dm".into(),
            author_id: "me".into(),
            author_name: "me".into(),
            content: format!("hello {id}"),
            timestamp: "2024-01-01T00:00:00+00:00".into(),
            media_type: None,
            attachments: vec![],
            preview: format!("hello {id}"),
        }
    }

    #[test]
    fn append_writes_bucket_day_and_target_files() {
        let (tmp, store) = scratch();
        let record = sample_record("m1", DeleteOutcome::Deleted, "c1");
        store.append(&record).unwrap();

        let day = record.deleted_at.date_naive();
        let day_dir = tmp.0.join(format!(
            "live/{}/{:02}/{:02}",
            day.year(),
            day.month(),
            day.day()
        ));
        assert!(day_dir.join("messages.jsonl").is_file());
        let channels: Vec<_> = fs::read_dir(day_dir.join("channels"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(channels.len(), 1);
        assert!(channels[0].ends_with("__c1.jsonl"));
    }

    #[test]
    fn dry_run_goes_under_dry_run_bucket() {
        let (tmp, store) = scratch();
        let record = sample_record("m1", DeleteOutcome::DryRun, "c1");
        store.append(&record).unwrap();

        let day = record.deleted_at.date_naive();
        assert!(tmp
            .0
            .join(format!(
                "dry-run/{}/{:02}/{:02}/messages.jsonl",
                day.year(),
                day.month(),
                day.day()
            ))
            .is_file());
    }

    #[test]
    fn append_and_load_roundtrip() {
        let (_tmp, store) = scratch();
        let first = sample_record("m1", DeleteOutcome::Deleted, "c1");
        let second = sample_record("m2", DeleteOutcome::AlreadyGone, "c1");
        store.append(&first).unwrap();
        store.append(&second).unwrap();

        let loaded = store
            .load_day(first.deleted_at.date_naive(), HistoryScope::Live, None)
            .unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].item_id, "m1");
        assert_eq!(loaded[1].item_id, "m2");
    }

    #[test]
    fn load_day_filters_by_target() {
        let (_tmp, store) = scratch();
        store
            .append(&sample_record("a", DeleteOutcome::Deleted, "c1"))
            .unwrap();
        store
            .append(&sample_record("b", DeleteOutcome::Deleted, "c2"))
            .unwrap();

        let only_c1 = store
            .load_day(Utc::now().date_naive(), HistoryScope::All, Some("c1"))
            .unwrap();
        assert_eq!(only_c1.len(), 1);
        assert_eq!(only_c1[0].item_id, "a");
    }

    #[test]
    fn list_days_and_scope_separation() {
        let (_tmp, store) = scratch();
        store
            .append(&sample_record("a", DeleteOutcome::Deleted, "c1"))
            .unwrap();
        store
            .append(&sample_record("b", DeleteOutcome::DryRun, "c1"))
            .unwrap();

        assert_eq!(store.list_days(HistoryScope::Live).unwrap().len(), 1);
        assert_eq!(store.list_days(HistoryScope::DryRun).unwrap().len(), 1);
        assert_eq!(store.list_days(HistoryScope::All).unwrap().len(), 1);
    }

    #[test]
    fn list_channels_from_shards() {
        let (_tmp, store) = scratch();
        store
            .append(&sample_record("a", DeleteOutcome::Deleted, "c1"))
            .unwrap();
        store
            .append(&sample_record("b", DeleteOutcome::Deleted, "c2"))
            .unwrap();

        let channels = store
            .list_channels(Utc::now().date_naive(), HistoryScope::Live)
            .unwrap();
        assert_eq!(channels.len(), 2);
        assert!(channels.iter().any(|c| c.id == "c1"));
        assert!(channels.iter().any(|c| c.id == "c2"));
    }

    #[test]
    fn sanitize_component_strips_junk() {
        assert_eq!(sanitize_component("Alice / DM"), "alice_dm");
        assert_eq!(sanitize_component("!!!"), "target");
        assert_eq!(sanitize_component("General"), "general");
    }

    #[test]
    fn text_preview_truncates() {
        let p = text_preview(&"a".repeat(100), "fallback");
        assert!(p.chars().count() <= 61);
        assert!(p.ends_with('\u{2026}'));
    }

    #[test]
    fn text_preview_uses_fallback() {
        assert_eq!(text_preview("  ", "photo"), "photo");
    }
}
