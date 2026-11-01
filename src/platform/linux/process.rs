use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Error, Result};

/// One-liner used in missing-tool errors. Runspace deliberately relies on
/// distro packages instead of shipping its own X server.
pub const X11_INSTALL: &str =
    "sudo apt-get install -y xvfb xserver-xephyr xdotool openbox x11-xserver-utils";

pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub fn send_signal(pid: i32, signal: i32) {
    if pid > 0 {
        unsafe {
            libc::kill(pid, signal);
        }
    }
}

pub fn find_in_path(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let path = PathBuf::from(program);
        return is_executable(&path).then_some(path);
    }

    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

pub fn require_tool(tool: &'static str, install: &'static str) -> Result<PathBuf> {
    find_in_path(tool).ok_or(Error::MissingTool { tool, install })
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy)]
pub struct TaggedProcess {
    pub pid: i32,
    pub rss_kb: u64,
}

/// Every process belonging to a runspace carries `RUNSPACE_ID` in its
/// environment. That lets us find and reap processes that were started by an
/// MCP session which has since exited.
pub fn tagged_processes(runspace_id: &str) -> Vec<TaggedProcess> {
    let needle = format!("RUNSPACE_ID={runspace_id}");
    let needle = needle.as_bytes();
    let me = std::process::id() as i32;

    proc_ids()
        .into_iter()
        .filter(|pid| *pid != me)
        .filter(|pid| environ_contains(*pid, needle))
        .map(|pid| TaggedProcess {
            pid,
            rss_kb: rss_kb(pid).unwrap_or(0),
        })
        .collect()
}

pub fn tagged_pids(runspace_id: &str) -> Vec<i32> {
    tagged_processes(runspace_id)
        .into_iter()
        .map(|p| p.pid)
        .collect()
}

/// Terminate every process tagged with this runspace. Returns the number of
/// processes that were signalled.
pub fn terminate_tagged(runspace_id: &str, grace: Duration) -> usize {
    let mut pids = tagged_pids(runspace_id);
    if pids.is_empty() {
        return 0;
    }
    let count = pids.len();

    for &pid in &pids {
        send_signal(pid, libc::SIGTERM);
    }
    if wait_until(|| tagged_pids(runspace_id).is_empty(), grace) {
        return count;
    }

    pids = tagged_pids(runspace_id);
    for pid in pids {
        send_signal(pid, libc::SIGKILL);
    }
    wait_until(
        || tagged_pids(runspace_id).is_empty(),
        Duration::from_millis(500),
    );
    count
}

/// Kill processes whose `RUNSPACE_ID` no longer has a state record. This is
/// how interrupted creates and crashed destructors are cleaned up.
pub fn sweep_orphans(valid_ids: &HashSet<String>) -> usize {
    let me = std::process::id() as i32;
    let mut victims = Vec::new();

    for pid in proc_ids() {
        if pid == me {
            continue;
        }
        if let Some(id) = runspace_id_of(pid)
            && !valid_ids.contains(&id)
        {
            victims.push(pid);
        }
    }

    if victims.is_empty() {
        return 0;
    }
    for &pid in &victims {
        send_signal(pid, libc::SIGTERM);
    }
    thread::sleep(Duration::from_millis(150));
    for &pid in &victims {
        if pid_alive(pid) {
            send_signal(pid, libc::SIGKILL);
        }
    }
    victims.len()
}

pub fn memory_bytes(runspace_id: &str, extra_pids: &[i32]) -> u64 {
    let mut seen = HashSet::new();
    let mut total = 0u64;

    for process in tagged_processes(runspace_id) {
        if seen.insert(process.pid) {
            total += process.rss_kb;
        }
    }
    for pid in extra_pids {
        if seen.insert(*pid)
            && let Some(kb) = rss_kb(*pid)
        {
            total += kb;
        }
    }
    total * 1024
}

fn proc_ids() -> Vec<i32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .collect()
}

fn environ_contains(pid: i32, needle: &[u8]) -> bool {
    let Ok(bytes) = fs::read(format!("/proc/{pid}/environ")) else {
        return false;
    };
    bytes.split(|b| *b == 0).any(|entry| entry == needle)
}

fn runspace_id_of(pid: i32) -> Option<String> {
    let bytes = fs::read(format!("/proc/{pid}/environ")).ok()?;
    for entry in bytes.split(|b| *b == 0) {
        if let Some(value) = entry.strip_prefix(b"RUNSPACE_ID=")
            && !value.is_empty()
        {
            return String::from_utf8(value.to_vec()).ok();
        }
    }
    None
}

fn rss_kb(pid: i32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// True when the PID is still the X server we started on this display. Guards
/// against signalling an unrelated process after PID reuse.
pub fn is_display_server(pid: i32, display: u16) -> bool {
    if !pid_alive(pid) {
        return false;
    }
    let Ok(bytes) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let args: Vec<String> = bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();

    let program_ok = args
        .first()
        .is_some_and(|arg| arg.ends_with("Xvfb") || arg.ends_with("Xephyr"));
    let display_arg = format!(":{display}");
    program_ok && args.contains(&display_arg)
}

/// True when the PID is the idle watchdog for this runspace.
pub fn is_watchdog(pid: i32, runspace_id: &str) -> bool {
    if !pid_alive(pid) {
        return false;
    }
    let Ok(bytes) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let args: Vec<String> = bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();
    args.iter().any(|arg| arg == "__watch") && args.iter().any(|arg| arg == runspace_id)
}

pub fn remove_display_artifacts(display: u16) {
    let socket = format!("/tmp/.X11-unix/X{display}");
    let lock = format!("/tmp/.X{display}-lock");
    for path in [socket, lock] {
        match fs::remove_file(&path) {
            Ok(()) => tracing::debug!("removed {path}"),
            Err(e) => match e.raw_os_error() {
                Some(libc::ENOENT) => {}
                Some(libc::EROFS | libc::EACCES | libc::EPERM) => {
                    tracing::debug!("could not remove {path}: {e}");
                }
                _ => tracing::warn!("could not remove {path}: {e}"),
            },
        }
    }
}

pub struct SpawnSpec<'a> {
    pub program: &'a str,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
    pub env_remove: Vec<&'a str>,
    pub cwd: Option<PathBuf>,
    pub log_path: &'a Path,
}

/// Spawn a process in its own session with stdio redirected to `log_path`.
/// The child survives this process exiting, which is what makes runspaces
/// outlive the CLI invocation that created them.
pub fn spawn_detached(spec: &SpawnSpec<'_>) -> Result<i32> {
    let child = spawn_detached_child(spec)?;
    Ok(child.id() as i32)
}

pub fn spawn_detached_child(spec: &SpawnSpec<'_>) -> Result<std::process::Child> {
    if let Some(parent) = spec.log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(spec.log_path)?;

    let mut command = Command::new(spec.program);
    command
        .args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    for (key, value) in &spec.envs {
        command.env(key, value);
    }
    for key in &spec.env_remove {
        command.env_remove(key);
    }
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }

    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    match command.spawn() {
        Ok(child) => Ok(child),
        Err(source) => Err(Error::Spawn {
            program: spec.program.to_string(),
            source,
        }),
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
    // If we started mid-file, drop the first partial line.
    let all = if start > 0 && all.len() > 1 {
        &all[1..]
    } else {
        &all[..]
    };
    let from = all.len().saturating_sub(lines);
    Ok(all[from..].join("\n"))
}

pub fn append_log(path: &Path, message: &str) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    writeln!(file, "{message}")
}
