#![cfg(windows)]

//! Native Windows end-to-end test: hidden desktop, real Win32 GUI app,
//! screenshot, click, typing, Ctrl+A, and cleanup.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Context;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_runspace")
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text())
        .map(|text| text.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn cli(runtime: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .env("RUNSPACE_RUNTIME_DIR", runtime)
        .env("RUST_LOG", "error")
        .output()
        .expect("runspace CLI should run")
}

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = Command::new(binary())
            .args(["destroy", "--all"])
            .env("RUNSPACE_RUNTIME_DIR", &self.0)
            .env("RUST_LOG", "error")
            .output();
    }
}

fn read_until(path: &Path, needle: &str, timeout: Duration) -> anyhow::Result<String> {
    let deadline = Instant::now() + timeout;
    loop {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if text.contains(needle) {
            return Ok(text);
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "timed out waiting for {needle:?} in {}:\n{text}",
                path.display()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn center(log: &str, prefix: &str) -> anyhow::Result<(i32, i32)> {
    let line = log
        .lines()
        .find(|line| line.starts_with(prefix))
        .with_context(|| format!("missing {prefix} in test app log:\n{log}"))?;
    let mut x = None;
    let mut y = None;
    for field in line.split_whitespace().skip(1) {
        if let Some(value) = field.strip_prefix("x=") {
            x = value.parse().ok();
        }
        if let Some(value) = field.strip_prefix("y=") {
            y = value.parse().ok();
        }
    }
    Ok((x.context("missing x")?, y.context("missing y")?))
}

#[tokio::test]
async fn windows_gui_round_trip() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let runtime = temp.path().to_path_buf();
    let _cleanup = Cleanup(runtime.clone());

    let output = cli(
        &runtime,
        &[
            "create",
            "--json",
            "--record",
            "--name",
            "win-e2e",
            "--resolution",
            "1024x768",
        ],
    );
    assert!(
        output.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let created: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let id = created["id"]
        .as_str()
        .context("create --json should return an id")?
        .to_string();

    let log_path = runtime.join("testapp.log");
    let command_line = format!("\"{}\" __testapp \"{}\"", binary(), log_path.display());

    let mut mcp = tokio::process::Command::new(binary());
    mcp.args(["mcp", &id])
        .env("RUNSPACE_RUNTIME_DIR", &runtime)
        .env("RUST_LOG", "error");
    let transport = TokioChildProcess::new(mcp)?;
    let client = ().serve(transport).await?;

    macro_rules! call {
        ($name:expr, $args:expr) => {{
            let result = client
                .call_tool(CallToolRequestParams::new($name).with_arguments($args))
                .await?;
            assert!(
                !result.is_error.unwrap_or(false),
                "{} failed: {}",
                $name,
                text_of(&result)
            );
            result
        }};
    }

    call!(
        "launch_app",
        rmcp::object!({
            "command": command_line,
            "wait_for_window": "Runspace Test App",
            "wait_timeout_ms": 8000
        })
    );
    let waited = call!(
        "wait_for_window",
        rmcp::object!({ "title": "Runspace Test App", "timeout_ms": 5000 })
    );
    assert!(
        text_of(&waited).contains("Runspace Test App"),
        "unexpected wait_for_window result: {}",
        text_of(&waited)
    );
    client
        .call_tool(
            CallToolRequestParams::new("wait_for_idle")
                .with_arguments(rmcp::object!({ "timeout_ms": 5000, "quiet_ms": 400 })),
        )
        .await?;

    let windows = call!("list_windows", rmcp::object!({}));
    let windows: Vec<serde_json::Value> = serde_json::from_str(&text_of(&windows))?;
    assert!(
        windows
            .iter()
            .any(|window| window["title"] == "Runspace Test App"),
        "test app window not found: {windows:?}"
    );

    let tree = call!("get_ui_tree", rmcp::object!({}));
    assert!(
        text_of(&tree).contains("Increment"),
        "UI tree missing the Increment button: {}",
        text_of(&tree)
    );

    let log = read_until(&log_path, "button_center", Duration::from_secs(5))?;
    let (edit_x, edit_y) = center(&log, "edit_center")?;
    let (button_x, button_y) = center(&log, "button_center")?;

    call!("click", rmcp::object!({ "x": edit_x, "y": edit_y }));
    call!("type", rmcp::object!({ "text": "hello windows" }));
    call!("key", rmcp::object!({ "keys": ["ctrl+a"] }));
    call!("type", rmcp::object!({ "text": "replaced" }));
    call!("click", rmcp::object!({ "x": button_x, "y": button_y }));
    tokio::time::sleep(Duration::from_millis(400)).await;

    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log.contains("button clicked count=1 edit=\"replaced\""),
        "unexpected test app log:\n{log}"
    );

    let tree = call!("get_ui_tree", rmcp::object!({}));
    assert!(
        text_of(&tree).contains("\"replaced\""),
        "UI tree did not expose the Edit text: {}",
        text_of(&tree)
    );

    let trace_path = runtime.join("trace.html");
    let trace_output = cli(
        &runtime,
        &["trace", &id, "--html", trace_path.to_str().unwrap()],
    );
    assert!(
        trace_output.status.success(),
        "trace export failed: {}",
        String::from_utf8_lossy(&trace_output.stderr)
    );
    let trace_html = std::fs::read_to_string(&trace_path)?;
    assert!(
        trace_html.contains("data:image/png;base64,"),
        "trace HTML has no embedded screenshots"
    );

    let screenshot = call!("screenshot", rmcp::object!({}));
    assert!(
        screenshot
            .content
            .iter()
            .any(|block| block.as_image().is_some()),
        "screenshot did not return an image: {}",
        text_of(&screenshot)
    );

    client
        .call_tool(CallToolRequestParams::new("destroy"))
        .await?;
    drop(client);
    tokio::time::sleep(Duration::from_millis(800)).await;

    let listed = cli(&runtime, &["list", "--json"]);
    assert!(listed.status.success());
    let records: serde_json::Value = serde_json::from_slice(&listed.stdout)?;
    assert_eq!(
        records.as_array().map(|items| items.len()),
        Some(0),
        "runspace was not cleaned up: {records}"
    );
    Ok(())
}
