use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::core::paths::Paths;
use crate::core::state::{self, ClipboardMode, Mode, Record};
use crate::core::types::{
    LaunchOutcome, RawImage, Screenshot, UiControl, UiMatch, UiQueryResult, UiTree, WindowInfo,
};
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

/// Filters for the UI tree. At least one field must be present.
pub struct UiQuery<'a> {
    pub text: Option<&'a str>,
    pub class: Option<&'a str>,
    pub window: Option<&'a str>,
    pub limit: usize,
}

#[derive(Debug)]
struct QueryTerms {
    text: Option<String>,
    class: Option<String>,
    window: Option<String>,
    limit: usize,
}

impl UiQuery<'_> {
    fn terms(&self) -> Result<QueryTerms> {
        let normalize = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase)
        };
        let terms = QueryTerms {
            text: normalize(self.text),
            class: normalize(self.class),
            window: normalize(self.window),
            limit: self.limit.clamp(1, 100),
        };
        if terms.text.is_none() && terms.class.is_none() && terms.window.is_none() {
            return Err(Error::invalid(
                "provide at least one of text, class, or window",
            ));
        }
        Ok(terms)
    }
}

pub fn query_ui(target: &str, query: &UiQuery<'_>) -> Result<UiQueryResult> {
    let terms = query.terms()?;
    let (_record, tree) = ui_tree(target)?;
    query_ui_tree(&tree, &terms)
}

pub fn click_ui(
    target: &str,
    query: &UiQuery<'_>,
    button: &str,
    count: u32,
) -> Result<(UiMatch, String)> {
    let (desktop, matched) = resolve_ui_match(target, query)?;
    let message = desktop.click(button, Some(matched.x), Some(matched.y), count)?;
    Ok((matched, message))
}

pub fn type_ui(
    target: &str,
    query: &UiQuery<'_>,
    input: &str,
    clear: bool,
    submit: bool,
) -> Result<(UiMatch, String)> {
    if input.is_empty() {
        return Err(Error::invalid("input cannot be empty"));
    }
    let (desktop, matched) = resolve_ui_match(target, query)?;
    let mut messages = vec![desktop.click("left", Some(matched.x), Some(matched.y), 1)?];
    if clear {
        messages.push(desktop.key(&["ctrl+a".into()])?);
    }
    messages.push(desktop.type_text(input)?);
    if submit {
        messages.push(desktop.key(&["enter".into()])?);
    }
    Ok((matched, messages.join("; ")))
}

fn resolve_ui_match(target: &str, query: &UiQuery<'_>) -> Result<(platform::Desktop, UiMatch)> {
    let terms = query.terms()?;
    let paths = Paths::discover()?;
    let record = resolve_active(&paths, Some(target))?;
    let desktop = platform::Desktop::new(&record, &paths)?;
    let tree = desktop.ui_tree()?;
    let result = query_ui_tree(&tree, &terms)?;
    let matched = result
        .matches
        .into_iter()
        .find(|item| item.control.as_ref().is_none_or(|control| control.enabled))
        .ok_or_else(|| {
            Error::invalid(format!(
                "no enabled UI match for {}; use find_ui or inspect to see the current tree",
                describe_terms(&terms)
            ))
        })?;
    Ok((desktop, matched))
}

fn query_ui_tree(tree: &UiTree, terms: &QueryTerms) -> Result<UiQueryResult> {
    let windows_scanned = tree.windows.len();
    let controls_scanned = tree
        .windows
        .iter()
        .map(|window| window.controls.len())
        .sum();
    let mut window_matches = Vec::new();
    let mut control_matches = Vec::new();

    for window in &tree.windows {
        if let Some(filter) = &terms.window
            && !window_matches_filter(&window.window, filter)
        {
            continue;
        }

        if terms.text.is_some() || terms.class.is_some() {
            for control in &window.controls {
                if matches_text_class(control, terms) {
                    control_matches.push(UiMatch {
                        window: window.window.clone(),
                        control: Some(control.clone()),
                        x: control.x + (control.width as i32 / 2),
                        y: control.y + (control.height as i32 / 2),
                    });
                }
            }
        }

        let title_matches = terms
            .text
            .as_ref()
            .is_none_or(|text| window.window.title.to_ascii_lowercase().contains(text));
        let class_matches = terms
            .class
            .as_ref()
            .is_none_or(|class| window.window.class.to_ascii_lowercase().contains(class));
        if title_matches && class_matches {
            window_matches.push(UiMatch {
                x: window.window.x + (window.window.width as i32 / 2),
                y: window.window.y + (window.window.height as i32 / 2),
                window: window.window.clone(),
                control: None,
            });
        }
    }

    let mut matches = if control_matches.is_empty() {
        window_matches
    } else {
        control_matches
    };
    let truncated = matches.len() > terms.limit;
    matches.truncate(terms.limit);

    Ok(UiQueryResult {
        backend: tree.backend.clone(),
        matches,
        windows_scanned,
        controls_scanned,
        truncated,
    })
}

fn window_matches_filter(window: &WindowInfo, filter: &str) -> bool {
    filter.parse::<u64>().ok() == Some(window.id)
        || window.title.to_ascii_lowercase().contains(filter)
        || window.class.to_ascii_lowercase().contains(filter)
}

fn matches_text_class(control: &UiControl, terms: &QueryTerms) -> bool {
    terms
        .text
        .as_ref()
        .is_none_or(|text| control.text.to_ascii_lowercase().contains(text))
        && terms
            .class
            .as_ref()
            .is_none_or(|class| control.class.to_ascii_lowercase().contains(class))
}

fn describe_terms(terms: &QueryTerms) -> String {
    let mut filters = Vec::new();
    if let Some(text) = &terms.text {
        filters.push(format!("text=\"{text}\""));
    }
    if let Some(class) = &terms.class {
        filters.push(format!("class=\"{class}\""));
    }
    if let Some(window) = &terms.window {
        filters.push(format!("window=\"{window}\""));
    }
    filters.join(", ")
}

pub fn list_windows(target: &str) -> Result<Vec<WindowInfo>> {
    let paths = Paths::discover()?;
    let record = resolve_active(&paths, Some(target))?;
    platform::Desktop::new(&record, &paths)?.list_windows()
}

pub fn launch(
    target: &str,
    command: &str,
    cwd: Option<&Path>,
    wait_ms: u64,
) -> Result<LaunchOutcome> {
    let paths = Paths::discover()?;
    let record = resolve_active(&paths, Some(target))?;
    platform::launch_application(&record, &paths, command, cwd, wait_ms)
}

pub fn wait_for_window_on(
    record: &Record,
    paths: &Paths,
    title: Option<&str>,
    class: Option<&str>,
    timeout_ms: u64,
) -> Result<WindowInfo> {
    let timeout = timeout_ms.clamp(100, 120_000);
    let desktop = platform::Desktop::new(record, paths)?;
    let deadline = Instant::now() + Duration::from_millis(timeout);
    let title = title.map(str::to_ascii_lowercase);
    let class = class.map(str::to_ascii_lowercase);
    loop {
        let windows = desktop.list_windows()?;
        if let Some(window) = windows.iter().find(|window| {
            title
                .as_deref()
                .is_none_or(|title| window.title.to_ascii_lowercase().contains(title))
                && class
                    .as_deref()
                    .is_none_or(|class| window.class.to_ascii_lowercase().contains(class))
        }) {
            return Ok(window.clone());
        }
        if Instant::now() >= deadline {
            let open = if windows.is_empty() {
                "none".to_string()
            } else {
                windows
                    .iter()
                    .map(|window| format!("\"{}\" <{}>", window.title, window.class))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            return Err(Error::invalid(format!(
                "no window matched within {timeout}ms; open windows: {open}"
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn wait_for_window(
    target: &str,
    title: Option<&str>,
    class: Option<&str>,
    timeout_ms: u64,
) -> Result<WindowInfo> {
    let paths = Paths::discover()?;
    let record = resolve_active(&paths, Some(target))?;
    wait_for_window_on(&record, &paths, title, class, timeout_ms)
}

#[derive(Debug, Serialize)]
pub struct ScreenshotExport {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
    pub window: Option<WindowInfo>,
}

pub fn screenshot(target: &str, output: Option<PathBuf>) -> Result<ScreenshotExport> {
    let paths = Paths::discover()?;
    let record = resolve_active(&paths, Some(target))?;
    let shot = platform::Desktop::new(&record, &paths)?.screenshot()?;
    let path = output.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(format!("runspace-{}.png", record.id))
    });
    write_screenshot(path, shot, None)
}

pub fn capture_window(target: &str, filter: &str) -> Result<(WindowInfo, Screenshot)> {
    let paths = Paths::discover()?;
    let record = resolve_active(&paths, Some(target))?;
    let desktop = platform::Desktop::new(&record, &paths)?;
    let windows = desktop.list_windows()?;
    let window = find_window(&windows, filter)?.clone();
    let screen = desktop.capture_rgba()?;
    let cropped = crop_window(&screen, &window)?;
    let png = crate::core::image::encode_png(&cropped)?;
    Ok((
        window,
        Screenshot {
            width: cropped.width,
            height: cropped.height,
            png,
        },
    ))
}

pub fn screenshot_window(
    target: &str,
    filter: &str,
    output: Option<PathBuf>,
) -> Result<ScreenshotExport> {
    let (window, shot) = capture_window(target, filter)?;
    let path = output.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(format!("runspace-window-{}.png", window.id))
    });
    write_screenshot(path, shot, Some(window))
}

fn write_screenshot(
    path: PathBuf,
    shot: Screenshot,
    window: Option<WindowInfo>,
) -> Result<ScreenshotExport> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, &shot.png)?;
    Ok(ScreenshotExport {
        path: path.display().to_string(),
        width: shot.width,
        height: shot.height,
        bytes: shot.png.len(),
        window,
    })
}

fn find_window<'a>(windows: &'a [WindowInfo], filter: &str) -> Result<&'a WindowInfo> {
    let filter = filter.trim();
    if filter.is_empty() {
        return Err(Error::invalid("window id, title, or class cannot be empty"));
    }
    if let Ok(id) = filter.parse::<u64>()
        && let Some(window) = windows.iter().find(|window| window.id == id)
    {
        return Ok(window);
    }
    if let Some(window) = windows
        .iter()
        .find(|window| window.title.eq_ignore_ascii_case(filter))
    {
        return Ok(window);
    }
    if let Some(window) = windows.iter().find(|window| {
        window
            .title
            .to_ascii_lowercase()
            .contains(&filter.to_ascii_lowercase())
    }) {
        return Ok(window);
    }
    if let Some(window) = windows
        .iter()
        .find(|window| window.class.eq_ignore_ascii_case(filter))
    {
        return Ok(window);
    }
    Err(Error::invalid(format!(
        "no window matching '{filter}'; use `runspace windows` or list_windows"
    )))
}

fn crop_window(screen: &RawImage, window: &WindowInfo) -> Result<RawImage> {
    let left = window.x.max(0) as u32;
    let top = window.y.max(0) as u32;
    let right = (window.x.saturating_add(window.width as i32)).clamp(0, screen.width as i32) as u32;
    let bottom =
        (window.y.saturating_add(window.height as i32)).clamp(0, screen.height as i32) as u32;
    if right <= left || bottom <= top {
        return Err(Error::invalid(
            "window is outside the runspace capture canvas",
        ));
    }
    let width = right - left;
    let height = bottom - top;
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for row in 0..height {
        let source = ((top + row) * screen.width + left) as usize * 4;
        let target = (row * width) as usize * 4;
        let count = width as usize * 4;
        rgba[target..target + count].copy_from_slice(&screen.rgba[source..source + count]);
    }
    Ok(RawImage {
        width,
        height,
        rgba,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{UiControl, UiTree, UiWindow};

    fn window(controls: Vec<UiControl>) -> UiTree {
        UiTree {
            backend: "win32".into(),
            windows: vec![UiWindow {
                window: WindowInfo {
                    id: 1,
                    title: "Editor".into(),
                    class: "EditorWindow".into(),
                    x: 100,
                    y: 50,
                    width: 400,
                    height: 300,
                    pid: Some(42),
                },
                focused: true,
                controls,
                controls_truncated: false,
            }],
            focused_window: Some(1),
            cursor_x: 0,
            cursor_y: 0,
        }
    }

    fn control(id: u64, class: &str, text: &str) -> UiControl {
        UiControl {
            id,
            parent: Some(1),
            class: class.into(),
            text: text.into(),
            x: 120,
            y: 80,
            width: 80,
            height: 24,
            enabled: true,
            visible: true,
            focused: false,
            control_id: id as i32,
            depth: 1,
        }
    }

    fn query(tree: &UiTree, query: UiQuery<'_>) -> UiQueryResult {
        query_ui_tree(tree, &query.terms().unwrap()).unwrap()
    }

    #[test]
    fn control_matches_use_absolute_center() {
        let tree = window(vec![
            control(10, "Button", "Save"),
            control(11, "Static", "Ready"),
        ]);
        let result = query(
            &tree,
            UiQuery {
                text: Some("save"),
                class: Some("button"),
                window: None,
                limit: 20,
            },
        );
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].x, 160);
        assert_eq!(result.matches[0].y, 92);
        assert_eq!(result.matches[0].control.as_ref().unwrap().class, "Button");
        assert_eq!(result.controls_scanned, 2);
    }

    #[test]
    fn windows_are_matched_when_no_controls_match() {
        let tree = window(Vec::new());
        let result = query(
            &tree,
            UiQuery {
                text: Some("editor"),
                class: None,
                window: None,
                limit: 20,
            },
        );
        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].control.is_none());
        assert_eq!(result.matches[0].x, 300);
        assert_eq!(result.matches[0].y, 200);
    }

    #[test]
    fn window_filter_is_applied_without_selecting_controls() {
        let tree = window(vec![control(10, "Button", "Save")]);
        let matching = query(
            &tree,
            UiQuery {
                text: None,
                class: None,
                window: Some("editorwindow"),
                limit: 20,
            },
        );
        assert_eq!(matching.matches.len(), 1);
        assert!(matching.matches[0].control.is_none());

        let missing = query(
            &tree,
            UiQuery {
                text: None,
                class: None,
                window: Some("terminal"),
                limit: 20,
            },
        );
        assert!(missing.matches.is_empty());
    }

    #[test]
    fn empty_query_is_rejected() {
        let error = UiQuery {
            text: None,
            class: None,
            window: None,
            limit: 20,
        }
        .terms()
        .unwrap_err();
        assert!(error.to_string().contains("at least one"));
    }

    #[test]
    fn limits_are_reported_as_truncated() {
        let tree = window(vec![
            control(10, "Button", "Save"),
            control(11, "Button", "Save as"),
        ]);
        let result = query(
            &tree,
            UiQuery {
                text: Some("save"),
                class: None,
                window: None,
                limit: 1,
            },
        );
        assert_eq!(result.matches.len(), 1);
        assert!(result.truncated);
    }
}
