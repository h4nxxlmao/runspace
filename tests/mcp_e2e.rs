//! End-to-end test of the MCP server against a real desktop backend.
//!
//! On Linux this requires `Xvfb`, `xdotool` and `openbox`. On Windows it runs
//! against the native hidden-desktop backend with no external dependencies.
//!
//! ```sh
//! RUNSPACE_E2E=1 cargo test --test mcp_e2e -- --nocapture
//! ```

use std::env;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_runspace")
}

#[cfg(target_os = "linux")]
fn have(tool: &str) -> bool {
    env::var_os("PATH").is_some_and(|path| {
        env::split_paths(&path).any(|dir| {
            let candidate = dir.join(tool);
            candidate.is_file()
        })
    })
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

#[tokio::test]
async fn mcp_controls_a_runspace() -> anyhow::Result<()> {
    if env::var("RUNSPACE_E2E").ok().as_deref() != Some("1") {
        eprintln!("skipping MCP e2e; set RUNSPACE_E2E=1 to enable it");
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if !have("Xvfb") || !have("xdotool") {
        eprintln!("skipping MCP e2e; Xvfb and xdotool are required");
        return Ok(());
    }

    let temp = tempfile::tempdir()?;
    let runtime = temp.path();

    let output = cli(runtime, &["create", "--json", "--record", "--name", "e2e"]);
    assert!(
        output.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let created: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let id = created["id"]
        .as_str()
        .expect("create --json should return an id")
        .to_string();

    let mut command = tokio::process::Command::new(binary());
    command
        .args(["mcp", &id])
        .env("RUNSPACE_RUNTIME_DIR", runtime)
        .env("RUST_LOG", "error");
    let transport = TokioChildProcess::new(command)?;
    let client = ().serve(transport).await?;

    let tools = client.list_all_tools().await?;
    let names = tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();
    for required in [
        "screenshot",
        "mouse_move",
        "click",
        "drag",
        "scroll",
        "type",
        "key",
        "launch_app",
        "list_windows",
        "wait_for_window",
        "get_ui_tree",
        "focus_window",
        "close_window",
        "get_info",
        "set_resolution",
        "get_logs",
        "destroy",
    ] {
        assert!(
            names.iter().any(|name| name == required),
            "missing MCP tool {required}: {names:?}"
        );
    }

    let screenshot = client
        .call_tool(CallToolRequestParams::new("screenshot"))
        .await?;
    assert!(
        screenshot
            .content
            .iter()
            .any(|block| block.as_image().is_some()),
        "screenshot did not return an image: {}",
        text_of(&screenshot)
    );

    let moved = client
        .call_tool(
            CallToolRequestParams::new("mouse_move")
                .with_arguments(rmcp::object!({ "x": 111, "y": 77 })),
        )
        .await?;
    assert!(
        !moved.is_error.unwrap_or(false),
        "mouse_move failed: {}",
        text_of(&moved)
    );

    let cursor = client
        .call_tool(CallToolRequestParams::new("get_cursor_position"))
        .await?;
    assert!(
        text_of(&cursor).contains("111,77"),
        "unexpected cursor position: {}",
        text_of(&cursor)
    );

    let info = client
        .call_tool(CallToolRequestParams::new("get_info"))
        .await?;
    let info_text = text_of(&info);
    assert!(
        info_text.contains(&id) && info_text.contains("\"resolution\""),
        "unexpected info: {info_text}"
    );

    let tree = client
        .call_tool(CallToolRequestParams::new("get_ui_tree"))
        .await?;
    assert!(
        !tree.is_error.unwrap_or(false),
        "get_ui_tree failed: {}",
        text_of(&tree)
    );
    let tree_text = text_of(&tree);
    assert!(
        tree_text.contains("\"backend\"") && tree_text.contains("\"windows\""),
        "unexpected UI tree: {tree_text}"
    );

    let failed = client
        .call_tool(
            CallToolRequestParams::new("close_window")
                .with_arguments(rmcp::object!({ "id_or_title": "definitely-not-a-window" })),
        )
        .await?;
    assert!(
        failed.is_error.unwrap_or(false),
        "close_window unexpectedly succeeded"
    );
    assert!(
        text_of(&failed).contains("recent log"),
        "failure context missing log: {}",
        text_of(&failed)
    );
    assert!(
        failed
            .content
            .iter()
            .any(|block| block.as_image().is_some()),
        "failure context missing screenshot"
    );

    let trace_path = runtime.join("trace.html");
    let trace = cli(
        runtime,
        &["trace", &id, "--html", trace_path.to_str().unwrap()],
    );
    assert!(
        trace.status.success(),
        "trace export failed: {}",
        String::from_utf8_lossy(&trace.stderr)
    );
    let trace_html = std::fs::read_to_string(&trace_path)?;
    assert!(
        trace_html.contains("data:image/png;base64,"),
        "trace HTML has no embedded frames"
    );

    let destroyed = client
        .call_tool(CallToolRequestParams::new("destroy"))
        .await?;
    assert!(
        !destroyed.is_error.unwrap_or(false),
        "destroy failed: {}",
        text_of(&destroyed)
    );
    drop(client);
    tokio::time::sleep(Duration::from_millis(800)).await;

    let listed = cli(runtime, &["list", "--json"]);
    assert!(listed.status.success());
    let records: serde_json::Value = serde_json::from_slice(&listed.stdout)?;
    assert_eq!(
        records.as_array().map(|items| items.len()),
        Some(0),
        "runspace was not cleaned up: {records}"
    );

    Ok(())
}
