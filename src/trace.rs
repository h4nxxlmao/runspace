use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::core::paths::Paths;
use crate::core::state::Record;
use crate::platform::Desktop;
use crate::{Error, Result, ops, platform};

static FRAME_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize, Deserialize)]
struct TraceEvent {
    ts: f64,
    tool: String,
    args: serde_json::Value,
    #[serde(default)]
    frame: Option<String>,
}

#[derive(Debug)]
pub struct TraceExport {
    pub path: PathBuf,
    pub events: usize,
    pub frames: usize,
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

fn trace_dir(paths: &Paths, id: &str) -> PathBuf {
    paths.dir(id).join("trace")
}

fn events_path(paths: &Paths, id: &str) -> PathBuf {
    trace_dir(paths, id).join("trace.jsonl")
}

fn capture(record: &Record, paths: &Paths, tool: &str) -> Option<String> {
    let desktop = Desktop::new(record, paths).ok()?;
    let screenshot = desktop.screenshot().ok()?;
    let frames = trace_dir(paths, &record.id).join("frames");
    fs::create_dir_all(&frames).ok()?;
    let millis = (now() * 1000.0) as u64;
    let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let safe_tool = tool
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let name = format!("{millis}-{}-{sequence}-{safe_tool}.png", std::process::id());
    fs::write(frames.join(&name), screenshot.png).ok()?;
    Some(format!("frames/{name}"))
}

fn append(paths: &Paths, id: &str, event: &TraceEvent) -> Result<()> {
    let dir = trace_dir(paths, id);
    fs::create_dir_all(&dir)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(events_path(paths, id))?;
    serde_json::to_writer(&mut file, event)
        .map_err(|error| Error::State(format!("could not write trace event: {error}")))?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

pub fn record_initial(paths: &Paths, record: &Record) -> Result<()> {
    if !record.recording {
        return Ok(());
    }
    let frame = capture(record, paths, "create");
    append(
        paths,
        &record.id,
        &TraceEvent {
            ts: now(),
            tool: "__create".into(),
            args: serde_json::json!({
                "mode": record.mode.as_str(),
                "resolution": record.resolution(),
                "backend": platform::backend_name(),
                "desktop": record.desktop.clone(),
                "job": record.job.clone(),
            }),
            frame,
        },
    )
}

pub fn record_tool_call<T: Serialize + ?Sized>(
    paths: &Paths,
    id: &str,
    tool: &str,
    args: &T,
) -> Result<()> {
    let Some(record) = crate::core::state::try_load(paths, id) else {
        return Ok(());
    };
    if !record.recording {
        return Ok(());
    }
    let args = serde_json::to_value(args).unwrap_or(serde_json::Value::Null);
    let frame = capture(&record, paths, tool);
    append(
        paths,
        id,
        &TraceEvent {
            ts: now(),
            tool: tool.into(),
            args,
            frame,
        },
    )
}

pub fn record_error(paths: &Paths, id: &str, error: &str) -> Result<()> {
    let Some(record) = crate::core::state::try_load(paths, id) else {
        return Ok(());
    };
    if !record.recording {
        return Ok(());
    }
    let mut error = error.to_string();
    if error.chars().count() > 6000 {
        error = error.chars().take(6000).collect();
        error.push_str("...");
    }
    append(
        paths,
        id,
        &TraceEvent {
            ts: now(),
            tool: "__error".into(),
            args: serde_json::json!({ "error": error }),
            frame: None,
        },
    )
}

pub fn export_html(paths: &Paths, record: &Record, output: &Path) -> Result<TraceExport> {
    let file = events_path(paths, &record.id);
    if !file.exists() {
        return Err(Error::invalid(
            "this runspace was not created with --record, so it has no trace",
        ));
    }

    let text = fs::read_to_string(&file)?;
    let mut events = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<TraceEvent>(line).ok())
        .collect::<Vec<_>>();

    if platform::is_alive(record)
        && let Some(frame) = capture(record, paths, "final")
    {
        events.push(TraceEvent {
            ts: now(),
            tool: "__final".into(),
            args: serde_json::json!({ "note": "captured at export" }),
            frame: Some(frame),
        });
    }

    let info = ops::info_for(record, paths);
    let log = ops::log_tail(record, paths, 80).unwrap_or_else(|error| error.to_string());
    let total_frames = events.iter().filter(|event| event.frame.is_some()).count();
    let name = record.name.as_deref().unwrap_or(&record.id);
    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    html.push_str("<title>Runspace Trace - ");
    html.push_str(&escape_html(name));
    html.push_str("</title><style>");
    html.push_str(CSS);
    html.push_str("</style></head><body><div class=\"shell\">");
    html.push_str("<header><div class=\"eyebrow\">RUNSPACE TRACE</div><h1>");
    html.push_str(&escape_html(name));
    html.push_str("</h1><div class=\"sub\"><span>");
    html.push_str(&escape_html(&record.id));
    html.push_str("</span><span>");
    html.push_str(&escape_html(&info.backend));
    html.push_str("</span><span>");
    html.push_str(&escape_html(info.mode.as_str()));
    html.push_str("</span><span>");
    html.push_str(&escape_html(&info.resolution));
    html.push_str("</span></div></header>");

    html.push_str("<section class=\"chips\">");
    chip(&mut html, "events", &events.len().to_string());
    chip(&mut html, "frames", &total_frames.to_string());
    chip(&mut html, "uptime", &format!("{}s", info.uptime_seconds));
    chip(
        &mut html,
        "memory",
        &format!("{:.1} MiB", info.memory_bytes as f64 / 1_048_576.0),
    );
    chip(&mut html, "clipboard", info.clipboard.as_str());
    html.push_str("</section>");

    html.push_str("<section class=\"timeline\">");
    for (index, event) in events.iter().enumerate() {
        let offset = event.ts - record.created_at as f64;
        html.push_str("<article class=\"event\"><div class=\"event-head\"><span class=\"index\">");
        html.push_str(&format!("{:02}", index + 1));
        html.push_str("</span><span class=\"tool\">");
        html.push_str(&escape_html(&event.tool));
        html.push_str("</span><span class=\"time\">");
        html.push_str(&format!("+{offset:.2}s"));
        html.push_str("</span></div>");
        if !event.args.is_null() {
            html.push_str("<pre class=\"args\">");
            html.push_str(&escape_html(
                &serde_json::to_string_pretty(&event.args).unwrap_or_default(),
            ));
            html.push_str("</pre>");
        }
        if let Some(frame) = &event.frame {
            let path = trace_dir(paths, &record.id).join(frame);
            match fs::read(&path) {
                Ok(bytes) => {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                    html.push_str("<figure><img alt=\"");
                    html.push_str(&escape_html(&event.tool));
                    html.push_str(" frame\" src=\"data:image/png;base64,");
                    html.push_str(&encoded);
                    html.push_str("\"></figure>");
                }
                Err(_) => html.push_str("<div class=\"missing\">frame unavailable</div>"),
            }
        }
        html.push_str("</article>");
    }
    html.push_str("</section>");

    html.push_str("<section class=\"logs\"><h2>Log tail</h2><pre>");
    html.push_str(&escape_html(&log));
    html.push_str("</pre></section>");
    html.push_str("<footer>Generated by Runspace ");
    html.push_str(env!("CARGO_PKG_VERSION"));
    html.push_str("</footer></div></body></html>");

    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, html)?;
    Ok(TraceExport {
        path: output.to_path_buf(),
        events: events.len(),
        frames: total_frames,
    })
}

fn chip(html: &mut String, label: &str, value: &str) {
    html.push_str("<div class=\"chip\"><span>");
    html.push_str(&escape_html(label));
    html.push_str("</span><strong>");
    html.push_str(&escape_html(value));
    html.push_str("</strong></div>");
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const CSS: &str = r#"
:root { color-scheme: dark; --bg:#14161b; --panel:#1b1e25; --panel-2:#22262f; --line:#303542; --text:#e6e9ef; --muted:#9aa4b2; --accent:#8ab4f8; --good:#8fd694; }
* { box-sizing: border-box; }
body { margin:0; background:radial-gradient(circle at 20% -10%, #202838 0, transparent 45%), var(--bg); color:var(--text); font:14px/1.5 ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif; }
.shell { max-width:1120px; margin:0 auto; padding:48px 24px 80px; }
header { border-bottom:1px solid var(--line); padding-bottom:28px; margin-bottom:24px; }
.eyebrow { color:var(--accent); font:600 11px/1 ui-monospace, SFMono-Regular, Consolas, monospace; letter-spacing:.16em; }
h1 { font-size:38px; line-height:1.1; margin:10px 0 14px; letter-spacing:-.03em; }
h2 { font-size:18px; margin:0 0 12px; }
.sub { display:flex; gap:10px; flex-wrap:wrap; color:var(--muted); font:12px/1.4 ui-monospace, SFMono-Regular, Consolas, monospace; }
.sub span { background:var(--panel); border:1px solid var(--line); border-radius:999px; padding:5px 9px; }
.chips { display:grid; grid-template-columns:repeat(auto-fit,minmax(130px,1fr)); gap:10px; margin:20px 0 34px; }
.chip { background:var(--panel); border:1px solid var(--line); border-radius:12px; padding:12px 14px; }
.chip span { display:block; color:var(--muted); font:10px/1.2 ui-monospace, SFMono-Regular, Consolas, monospace; text-transform:uppercase; letter-spacing:.1em; }
.chip strong { display:block; margin-top:6px; font-size:17px; font-weight:600; }
.timeline { display:flex; flex-direction:column; gap:18px; }
.event { background:var(--panel); border:1px solid var(--line); border-radius:16px; padding:18px; box-shadow:0 16px 40px rgba(0,0,0,.18); }
.event-head { display:flex; align-items:center; gap:10px; margin-bottom:10px; }
.index { color:var(--muted); font:11px/1 ui-monospace, SFMono-Regular, Consolas, monospace; }
.tool { font:600 14px/1 ui-monospace, SFMono-Regular, Consolas, monospace; color:var(--accent); }
.time { margin-left:auto; color:var(--muted); font:11px/1 ui-monospace, SFMono-Regular, Consolas, monospace; }
pre { margin:0; padding:13px 14px; background:#111319; border:1px solid #272c37; border-radius:10px; overflow:auto; color:#c7d0dd; font:12px/1.5 ui-monospace, SFMono-Regular, Consolas, monospace; }
.args { margin-bottom:12px; max-height:220px; }
figure { margin:0; border:1px solid var(--line); border-radius:12px; overflow:hidden; background:#0e1015; }
figure img { display:block; width:100%; height:auto; }
.missing { color:var(--muted); padding:18px; border:1px dashed var(--line); border-radius:10px; }
.logs { margin-top:34px; }
.logs pre { max-height:420px; }
footer { margin-top:40px; color:var(--muted); font:11px/1.4 ui-monospace, SFMono-Regular, Consolas, monospace; }
"#;
