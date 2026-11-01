use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{Error, Paths, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Invisible,
    Visible,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invisible => "invisible",
            Self::Visible => "visible",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardMode {
    #[default]
    Private,
    Shared,
}

impl ClipboardMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Shared => "shared",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub name: Option<String>,
    pub display: u16,
    pub mode: Mode,
    pub width: u16,
    pub height: u16,
    pub created_at: u64,
    pub backend_pid: i32,
    #[serde(default)]
    pub wm_pid: Option<i32>,
    #[serde(default)]
    pub idle_timeout: Option<u64>,
    #[serde(default)]
    pub watcher_pid: Option<i32>,
    #[serde(default)]
    pub clipboard: ClipboardMode,
    #[serde(default)]
    pub desktop: Option<String>,
    #[serde(default)]
    pub job: Option<String>,
    #[serde(default)]
    pub focus_window: Option<u64>,
    #[serde(default)]
    pub cursor_x: i32,
    #[serde(default)]
    pub cursor_y: i32,
    #[serde(default)]
    pub recording: bool,
}

impl Record {
    pub fn display_name(&self) -> String {
        if self.display > 0 {
            format!(":{}", self.display)
        } else {
            "win32".to_string()
        }
    }

    pub fn title(&self) -> String {
        format!("Runspace {}", self.id)
    }

    pub fn resolution(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }

    pub fn uptime_secs(&self) -> u64 {
        now_secs().saturating_sub(self.created_at)
    }

    pub fn label(&self) -> String {
        match &self.name {
            Some(name) => format!("{name} ({})", self.id),
            None => self.id.clone(),
        }
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect()
}

pub fn is_valid_id(id: &str) -> bool {
    (4..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

pub fn validate_name(name: &str) -> Result<()> {
    let reason = if name.is_empty() {
        Some("names cannot be empty")
    } else if name.len() > 32 {
        Some("names are limited to 32 characters")
    } else if name != name.trim() {
        Some("leading or trailing whitespace is not allowed")
    } else if name == "." || name == ".." {
        Some("that name is reserved")
    } else if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        Some("use ASCII letters, digits, '-', '_' or '.'")
    } else {
        None
    };

    match reason {
        Some(reason) => Err(Error::InvalidName {
            name: name.to_string(),
            reason: reason.to_string(),
        }),
        None => Ok(()),
    }
}

pub fn load(paths: &Paths, id: &str) -> Result<Record> {
    if !is_valid_id(id) {
        return Err(Error::State(format!("invalid runspace id '{id}'")));
    }
    let path = paths.record_path(id);
    let bytes = fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound(id.to_string())
        } else {
            Error::Io(e)
        }
    })?;
    let record: Record = serde_json::from_slice(&bytes)
        .map_err(|e| Error::State(format!("{} is corrupt: {e}", path.display())))?;
    if record.id != id {
        return Err(Error::State(format!(
            "{} contains record for '{}'",
            path.display(),
            record.id
        )));
    }
    Ok(record)
}

pub fn try_load(paths: &Paths, id: &str) -> Option<Record> {
    load(paths, id).ok()
}

pub fn load_all(paths: &Paths) -> Vec<Record> {
    let dir = paths.runspaces_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut records = Vec::new();
    for entry in entries.flatten() {
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !is_valid_id(&id) || !entry.path().is_dir() {
            continue;
        }
        match load(paths, &id) {
            Ok(record) => records.push(record),
            Err(Error::NotFound(_)) => {}
            Err(e) => tracing::warn!("skipping {id}: {e}"),
        }
    }

    records.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    records
}

pub fn save(paths: &Paths, record: &Record) -> Result<()> {
    let dir = paths.dir(&record.id);
    fs::create_dir_all(&dir)?;
    let path = paths.record_path(&record.id);
    let tmp = dir.join("state.json.tmp");

    let bytes = serde_json::to_vec_pretty(record)?;
    let mut file = write_options().open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn write_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

pub fn update_resolution(paths: &Paths, id: &str, width: u16, height: u16) -> Result<()> {
    let mut record = load(paths, id)?;
    record.width = width;
    record.height = height;
    save(paths, &record)
}

pub fn touch(paths: &Paths, id: &str) -> Result<()> {
    let path = paths.activity_path(id);
    let mut file: File = write_options().open(path)?;
    file.write_all(now_secs().to_string().as_bytes())?;
    Ok(())
}

pub fn last_activity(paths: &Paths, record: &Record) -> u64 {
    fs::read_to_string(paths.activity_path(&record.id))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(record.created_at)
}

pub fn resolve<'a>(records: &'a [Record], target: &str) -> Result<&'a Record> {
    if let Some(record) = records.iter().find(|r| r.id == target) {
        return Ok(record);
    }

    let named: Vec<&Record> = records
        .iter()
        .filter(|r| {
            r.name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(target))
        })
        .collect();
    match named.len() {
        0 => {}
        1 => return Ok(named[0]),
        _ => return Err(ambiguous(target, named)),
    }

    if target.len() >= 3 {
        let prefixed: Vec<&Record> = records
            .iter()
            .filter(|r| r.id.starts_with(target))
            .collect();
        match prefixed.len() {
            0 => {}
            1 => return Ok(prefixed[0]),
            _ => return Err(ambiguous(target, prefixed)),
        }
    }

    Err(Error::NotFound(target.to_string()))
}

fn ambiguous(target: &str, records: Vec<&Record>) -> Error {
    let matches = records
        .iter()
        .map(|r| r.label())
        .collect::<Vec<_>>()
        .join(", ");
    Error::Ambiguous {
        target: target.to_string(),
        matches,
    }
}

pub fn remove(paths: &Paths, id: &str) -> Result<()> {
    let dir = paths.dir(id);
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Remove empty per-user runtime directories. The zero-byte lock file and its
/// parent stay behind so subsequent creates reuse the same inode.
pub fn prune_runtime_dir(paths: &Paths) {
    let _ = fs::remove_dir(paths.runspaces_dir());
}
