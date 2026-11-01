use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::core::paths::Paths;
use crate::core::state::{self, ClipboardMode, Mode, Record};
use crate::core::types::UiTree;
use crate::platform::{self, CreateContext};
use crate::trace::{self, TraceExport};
use crate::{Error, Result};

pub struct CreateOptions {
    pub visible: bool,
    pub width: u16,
    pub height: u16,
    pub name: Option<String>,
    pub idle_timeout: Option<u64>,
    pub record: bool,
}

pub fn parse_resolution(input: &str) -> Result<(u16, u16)> {
    let input = input.trim();
    let (width, height) = input
        .split_once(['x', 'X'])
        .ok_or_else(|| Error::invalid(format!("invalid resolution '{input}' (expected WxH)")))?;
    let width: u16 = width
        .parse()
        .map_err(|_| Error::invalid(format!("invalid width in '{input}'")))?;
    let height: u16 = height
        .parse()
        .map_err(|_| Error::invalid(format!("invalid height in '{input}'")))?;
    platform::validate_resolution(width, height)?;
    Ok((width, height))
}

pub fn create(options: CreateOptions) -> Result<Record> {
    platform::validate_resolution(options.width, options.height)?;
    if let Some(name) = &options.name {
        state::validate_name(name)?;
    }
    if let Some(timeout) = options.idle_timeout
        && !(1..=604_800).contains(&timeout)
    {
        return Err(Error::invalid(
            "idle timeout must be between 1 second and 7 days",
        ));
    }

    let paths = Paths::discover()?;
    paths.ensure()?;
    let _lock = paths.lock()?;

    let active = cleanup_dead(&paths);
    if let Some(name) = &options.name
        && active.iter().any(|record| {
            record
                .name
                .as_deref()
                .is_some_and(|existing| existing.eq_ignore_ascii_case(name))
        })
    {
        return Err(Error::NameTaken(name.clone()));
    }

    let id = allocate_id(&paths);
    platform::ensure_state_dirs(&paths, &id)?;
    let creating_marker = paths.dir(&id).join("creating");
    fs::write(&creating_marker, b"")?;

    let started = match platform::create_backend(&CreateContext {
        id: &id,
        width: options.width,
        height: options.height,
        visible: options.visible,
        active: &active,
        paths: &paths,
    }) {
        Ok(started) => started,
        Err(error) => {
            let _ = state::remove(&paths, &id);
            return Err(error);
        }
    };

    let mode = if options.visible {
        Mode::Visible
    } else {
        Mode::Invisible
    };
    let record = Record {
        id: id.clone(),
        name: options.name.clone(),
        display: started.display,
        mode,
        width: options.width,
        height: options.height,
        created_at: state::now_secs(),
        backend_pid: started.backend_pid,
        wm_pid: started.wm_pid,
        idle_timeout: options.idle_timeout,
        watcher_pid: None,
        clipboard: started.clipboard,
        desktop: started.desktop,
        job: started.job,
        focus_window: None,
        cursor_x: i32::from(options.width) / 2,
        cursor_y: i32::from(options.height) / 2,
        recording: options.record,
    };

    append_log(
        &paths.log_path(&id),
        &format!(
            "runspace {} | id={id} | backend={} | mode={} | display={} | {}x{}",
            env!("CARGO_PKG_VERSION"),
            platform::backend_name(),
            record.mode.as_str(),
            record.display_name(),
            options.width,
            options.height
        ),
    );

    if let Err(error) = state::save(&paths, &record) {
        platform::destroy_backend(&record, &paths);
        let _ = state::remove(&paths, &id);
        return Err(error);
    }
    let _ = fs::remove_file(&creating_marker);

    if let Err(error) = platform::post_create(&record, &paths) {
        tracing::warn!("post-create backend step failed: {error}");
    }
    if record.recording
        && let Err(error) = trace::record_initial(&paths, &record)
    {
        tracing::warn!("could not start the trace: {error}");
    }

    let watcher_pid = if record.idle_timeout.is_some() {
        match platform::spawn_watchdog(&id) {
            Ok(pid) => Some(pid),
            Err(error) => {
                tracing::warn!("could not start idle watchdog: {error}");
                None
            }
        }
    } else {
        None
    };
    let record = Record {
        watcher_pid,
        ..record
    };

    if let Err(error) = state::save(&paths, &record) {
        platform::stop_watchdog(&record);
        platform::destroy_backend(&record, &paths);
        let _ = state::remove(&paths, &id);
        return Err(error);
    }

    if let Err(error) = state::touch(&paths, &id) {
        tracing::warn!("could not record activity for {id}: {error}");
    }

    Ok(record)
}

fn append_log(path: &Path, message: &str) {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(path) {
        let _ = writeln!(file, "{message}");
    }
}

fn allocate_id(paths: &Paths) -> String {
    for _ in 0..16 {
        let id = state::new_id();
        if !paths.dir(&id).exists() {
            return id;
        }
    }
    uuid::Uuid::new_v4().simple().to_string()
}

fn cleanup_dead(paths: &Paths) -> Vec<Record> {
    let records = state::load_all(paths);
    let mut alive = Vec::new();

    for record in records {
        if platform::is_alive(&record) {
            alive.push(record);
        } else {
            tracing::info!("cleaning up stale runspace {}", record.id);
            destroy_record(paths, &record);
        }
    }

    let valid: HashSet<String> = alive.iter().map(|record| record.id.clone()).collect();
    remove_orphan_dirs(paths, &valid);
    platform::cleanup_orphans(&valid);
    alive
}

/// Remove runtime directories that have no readable state record.
fn remove_orphan_dirs(paths: &Paths, valid: &HashSet<String>) {
    let Ok(entries) = fs::read_dir(paths.runspaces_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !state::is_valid_id(&id) || valid.contains(&id) || !entry.path().is_dir() {
            continue;
        }
        tracing::info!("removing orphaned runspace directory {id}");
        platform::destroy_orphan(&id, paths);
        let _ = state::remove(paths, &id);
    }
}

pub fn list() -> Result<Vec<Info>> {
    let paths = Paths::discover()?;
    let _lock = paths.lock()?;
    let active = cleanup_dead(&paths);
    Ok(active
        .into_iter()
        .map(|rec| info_for(&rec, &paths))
        .collect())
}

pub fn info(target: &str) -> Result<Info> {
    let paths = Paths::discover()?;
    let record = resolve_active(&paths, Some(target))?;
    Ok(info_for(&record, &paths))
}

pub fn resolve_active(paths: &Paths, target: Option<&str>) -> Result<Record> {
    let _lock = paths.lock()?;
    let active = cleanup_dead(paths);
    match target {
        Some(target) => state::resolve(&active, target).cloned(),
        None => match active.len() {
            0 => Err(Error::NoRunspaces),
            1 => Ok(active[0].clone()),
            _ => Err(Error::TargetRequired(
                active
                    .iter()
                    .map(|record| record.label())
                    .collect::<Vec<_>>()
                    .join(", "),
            )),
        },
    }
}

pub fn destroy_targets(targets: &[String]) -> Result<Vec<Record>> {
    let paths = Paths::discover()?;
    let _lock = paths.lock()?;
    let records = state::load_all(&paths);

    let mut selected: Vec<Record> = Vec::new();
    for target in targets {
        let record = state::resolve(&records, target)?;
        if !selected.iter().any(|item| item.id == record.id) {
            selected.push(record.clone());
        }
    }

    for record in &selected {
        destroy_record(&paths, record);
    }
    state::prune_runtime_dir(&paths);
    Ok(selected)
}

pub fn destroy_all() -> Result<Vec<Record>> {
    let paths = Paths::discover()?;
    let _lock = paths.lock()?;
    let records = state::load_all(&paths);
    for record in &records {
        destroy_record(&paths, record);
    }
    let none = HashSet::new();
    remove_orphan_dirs(&paths, &none);
    platform::cleanup_orphans(&none);
    platform::cleanup_helpers(&paths);
    state::prune_runtime_dir(&paths);
    Ok(records)
}

/// Stop the watchdog, tear down the platform backend, then delete state.
pub fn destroy_record(paths: &Paths, record: &Record) {
    platform::stop_watchdog(record);
    platform::destroy_backend(record, paths);
    if let Err(error) = state::remove(paths, &record.id) {
        tracing::warn!("could not remove state for {}: {error}", record.id);
    }
}

pub fn attach(target: &str) -> Result<String> {
    let paths = Paths::discover()?;
    let record = resolve_active(&paths, Some(target))?;
    platform::attach(&record)
}

pub fn log_tail(record: &Record, paths: &Paths, lines: usize) -> Result<String> {
    let lines = lines.clamp(1, 2000);
    let path = paths.log_path(&record.id);
    match tail_lines(&path, lines) {
        Ok(text) if text.trim().is_empty() => Ok("(log is empty)".into()),
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok("(log is empty)".into()),
        Err(error) => Err(error.into()),
    }
}

pub fn tail_lines(path: &Path, lines: usize) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let max = 512 * 1024;
    let start = len.saturating_sub(max);
    if start > 0 {
        file.seek(SeekFrom::Start(start))?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    let all = if start > 0 && all.len() > 1 {
        &all[1..]
    } else {
        &all[..]
    };
    let from = all.len().saturating_sub(lines);
    Ok(all[from..].join("\n"))
}

pub fn logs(target: &str, lines: usize) -> Result<String> {
    let paths = Paths::discover()?;
    let record = resolve_any(&paths, target)?;
    log_tail(&record, &paths, lines)
}

fn resolve_any(paths: &Paths, target: &str) -> Result<Record> {
    let records = state::load_all(paths);
    state::resolve(&records, target).cloned()
}

pub fn follow_logs(target: &str) -> Result<()> {
    let paths = Paths::discover()?;
    let record = resolve_any(&paths, target)?;
    let path = paths.log_path(&record.id);

    if !path.exists() {
        return Err(Error::invalid(format!(
            "no log file for {} yet",
            record.label()
        )));
    }

    let mut file = fs::File::open(&path)?;
    let mut position = file.metadata()?.len();
    let mut stdout = io::stdout().lock();
    loop {
        let len = match fs::metadata(&path) {
            Ok(meta) => meta.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                writeln!(stdout, "\n(runspace removed; stopped following)")?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };

        if len < position {
            position = 0;
            file.seek(SeekFrom::Start(0))?;
        }
        if len > position {
            file.seek(SeekFrom::Start(position))?;
            let mut buffer = Vec::new();
            (&mut file).take(len - position).read_to_end(&mut buffer)?;
            stdout.write_all(&buffer)?;
            stdout.flush()?;
            position = len;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[derive(Debug, Serialize)]
pub struct Info {
    pub id: String,
    pub name: Option<String>,
    pub mode: Mode,
    pub display: String,
    pub backend: String,
    pub desktop: Option<String>,
    pub job: Option<String>,
    pub resolution: String,
    pub width: u16,
    pub height: u16,
    pub backend_pid: i32,
    pub wm_pid: Option<i32>,
    pub created_at: u64,
    pub uptime_seconds: u64,
    pub idle_timeout_seconds: Option<u64>,
    pub memory_bytes: u64,
    pub clipboard: ClipboardMode,
    pub recording: bool,
    pub log_path: String,
    pub mcp_command: String,
}

pub fn info_for(record: &Record, paths: &Paths) -> Info {
    Info {
        id: record.id.clone(),
        name: record.name.clone(),
        mode: record.mode,
        display: record.display_name(),
        backend: platform::backend_name().to_string(),
        desktop: record.desktop.clone(),
        job: record.job.clone(),
        resolution: record.resolution(),
        width: record.width,
        height: record.height,
        backend_pid: record.backend_pid,
        wm_pid: record.wm_pid,
        created_at: record.created_at,
        uptime_seconds: record.uptime_secs(),
        idle_timeout_seconds: record.idle_timeout,
        memory_bytes: platform::memory(record),
        clipboard: record.clipboard,
        recording: record.recording,
        log_path: paths.log_path(&record.id).display().to_string(),
        mcp_command: format!("runspace mcp {}", record.id),
    }
}

pub fn ui_tree(target: &str) -> Result<(Record, UiTree)> {
    let paths = Paths::discover()?;
    let record = resolve_active(&paths, Some(target))?;
    let tree = platform::Desktop::new(&record, &paths)?.ui_tree()?;
    Ok((record, tree))
}

pub fn export_trace(target: &str, output: Option<PathBuf>) -> Result<TraceExport> {
    let paths = Paths::discover()?;
    let record = resolve_active(&paths, Some(target))?;
    let output = output.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(format!("runspace-{}-trace.html", record.id))
    });
    trace::export_html(&paths, &record, &output)
}

pub fn doctor() -> platform::DoctorReport {
    platform::doctor()
}
