use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

use base64::Engine as _;
use serde::Deserialize;

use crate::core::paths::Paths;
use crate::core::state;
use crate::{Error, Result};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Deserialize)]
struct ExecSpec {
    command: String,
    cwd: Option<String>,
}

pub fn run(args: &[String]) -> Result<i32> {
    let id = args
        .first()
        .ok_or_else(|| Error::invalid("__exec requires a runspace id"))?;
    let payload = args
        .get(1)
        .ok_or_else(|| Error::invalid("__exec requires a payload"))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|error| Error::invalid(format!("invalid __exec payload: {error}")))?;
    let spec: ExecSpec = serde_json::from_slice(&decoded)
        .map_err(|error| Error::invalid(format!("invalid __exec spec: {error}")))?;

    let paths = Paths::discover()?;
    let record = state::load(&paths, id)?;
    let log_path = paths.log_path(id);
    append(&log_path, &format!("exec: {}", spec.command));

    let home = paths.home_dir(id);
    let temp = paths.tmp_dir(id);
    std::fs::create_dir_all(home.join("AppData/Local"))?;
    std::fs::create_dir_all(home.join("AppData/Roaming"))?;

    let cwd = spec
        .cwd
        .as_deref()
        .filter(|cwd| std::path::Path::new(cwd).is_dir())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| home.clone());

    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    // cmd.exe's /C parsing mangles nested quotes. A generated batch file is
    // the reliable way to run an arbitrary command string.
    let batch = paths
        .tmp_dir(id)
        .join(format!("exec-{}.cmd", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(paths.tmp_dir(id))?;
    std::fs::write(&batch, format!("@echo off\r\n{}\r\n", spec.command))?;

    let mut command = Command::new("cmd.exe");
    command
        .arg("/D")
        .arg("/S")
        .raw_arg(format!("/C \"\"{}\"\"", batch.display()))
        .env("RUNSPACE_ID", id)
        .env(
            "RUNSPACE_DESKTOP",
            record.desktop.clone().unwrap_or_default(),
        )
        .env("USERPROFILE", &home)
        .env("TEMP", &temp)
        .env("TMP", &temp)
        .env("LOCALAPPDATA", home.join("AppData/Local"))
        .env("APPDATA", home.join("AppData/Roaming"))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .creation_flags(CREATE_NO_WINDOW);

    let mut child = command.spawn().map_err(|error| {
        let _ = std::fs::remove_file(&batch);
        append(
            &log_path,
            &format!("exec: could not start command: {error}"),
        );
        Error::Spawn {
            program: "cmd.exe".into(),
            source: error,
        }
    })?;
    let status = child.wait()?;
    let _ = std::fs::remove_file(&batch);
    let code = status.code().unwrap_or(1);
    append(&log_path, &format!("exec: exited with status {code}"));
    Ok(code)
}

fn append(path: &std::path::Path, message: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{message}");
    }
}
