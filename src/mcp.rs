use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use base64::Engine as _;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::transport::stdio;
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

use crate::core::paths::Paths;
use crate::ops;
use crate::platform::{self, Desktop, Screenshot};
use crate::state::{self, Record};

static MCP_CONTEXT: OnceLock<(Paths, String)> = OnceLock::new();

#[derive(Clone)]
pub struct RunspaceServer {
    paths: Paths,
    id: String,
}

#[tool_router]
impl RunspaceServer {
    pub fn new(paths: Paths, id: String) -> Self {
        Self { paths, id }
    }

    fn trace_tool<T: Serialize + ?Sized>(&self, tool: &str, args: &T) {
        if let Err(error) = crate::trace::record_tool_call(&self.paths, &self.id, tool, args) {
            tracing::debug!("trace recording failed: {error}");
        }
    }

    #[tool(
        description = "Capture the entire desktop as a PNG image. Use this to see the current state before and after actions. Coordinates are in the returned image."
    )]
    async fn screenshot(&self) -> Result<CallToolResult, McpError> {
        self.trace_tool("screenshot", &serde_json::json!({}));
        let paths = self.paths.clone();
        let id = self.id.clone();
        let result = tokio::task::spawn_blocking(move || -> crate::Result<(Record, Screenshot)> {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let shot = Desktop::new(&record, &paths)?.screenshot()?;
            Ok((record, shot))
        })
        .await;

        match result {
            Ok(Ok((record, shot))) => {
                let data = base64::engine::general_purpose::STANDARD.encode(&shot.png);
                Ok(CallToolResult::success(vec![
                    ContentBlock::image(data, "image/png"),
                    ContentBlock::text(format!(
                        "{}x{} screenshot of display {} ({} bytes PNG)",
                        shot.width,
                        shot.height,
                        record.display_name(),
                        shot.png.len()
                    )),
                ]))
            }
            Ok(Err(error)) => Ok(tool_error(error)),
            Err(error) => Ok(tool_error(format!("internal error: {error}"))),
        }
    }

    #[tool(
        description = "Move the mouse cursor to absolute coordinates. (0,0) is the top-left corner of the virtual screen."
    )]
    async fn mouse_move(
        &self,
        Parameters(params): Parameters<MouseMoveParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("mouse_move", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.mouse_move(params.x, params.y)
        })
        .await
    }

    #[tool(
        description = "Click a mouse button, optionally moving to (x, y) first. Button is left, middle or right; count defaults to 1."
    )]
    async fn click(
        &self,
        Parameters(params): Parameters<ClickParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("click", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.click(
                params.button.as_deref().unwrap_or("left"),
                params.x,
                params.y,
                params.count.unwrap_or(1),
            )
        })
        .await
    }

    #[tool(description = "Double-click the left mouse button, optionally moving to (x, y) first.")]
    async fn double_click(
        &self,
        Parameters(params): Parameters<PointParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("double_click", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.click("left", params.x, params.y, 2)
        })
        .await
    }

    #[tool(
        description = "Right-click, optionally moving to (x, y) first. Useful for context menus."
    )]
    async fn right_click(
        &self,
        Parameters(params): Parameters<PointParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("right_click", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.click("right", params.x, params.y, 1)
        })
        .await
    }

    #[tool(
        description = "Press a mouse button at (from_x, from_y), drag to (to_x, to_y), and release. Duration defaults to 300 ms."
    )]
    async fn drag(
        &self,
        Parameters(params): Parameters<DragParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("drag", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.drag(
                params.from_x,
                params.from_y,
                params.to_x,
                params.to_y,
                params.button.as_deref().unwrap_or("left"),
                params.duration_ms.unwrap_or(300),
            )
        })
        .await
    }

    #[tool(
        description = "Scroll up, down, left or right by a number of wheel steps, optionally at (x, y)."
    )]
    async fn scroll(
        &self,
        Parameters(params): Parameters<ScrollParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("scroll", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.scroll(
                &params.direction,
                params.amount.unwrap_or(1),
                params.x,
                params.y,
            )
        })
        .await
    }

    #[tool(
        name = "type",
        description = "Type text into the currently focused window. Focus a window first, or launch an app that grabs input."
    )]
    async fn type_text(
        &self,
        Parameters(params): Parameters<TypeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("type_text", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.type_text(&params.text)
        })
        .await
    }

    #[tool(
        description = "Press one or more key combinations, for example \"ctrl+s\", \"alt+F4\", \"super+Return\". Multiple combinations are pressed in order."
    )]
    async fn key(
        &self,
        Parameters(params): Parameters<KeyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("key", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        let keys = match params.keys {
            Keys::One(combo) => vec![combo],
            Keys::Many(combos) => combos,
        };
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.key(&keys)
        })
        .await
    }

    #[tool(description = "Return the current mouse cursor position on the virtual screen.")]
    async fn get_cursor_position(&self) -> Result<CallToolResult, McpError> {
        self.trace_tool("get_cursor_position", &serde_json::json!({}));
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let (x, y) = Desktop::new(&record, &paths)?.cursor_position()?;
            Ok(format!("cursor at {x},{y} on {}", record.display_name()))
        })
        .await
    }

    #[tool(description = "Wait for a fixed number of milliseconds (maximum 60000).")]
    async fn wait(
        &self,
        Parameters(params): Parameters<WaitParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("wait", &params);
        if let Err(error) = state::load(&self.paths, &self.id) {
            return Ok(tool_error(error));
        }
        let ms = params.ms.min(60_000);
        let deadline = Instant::now() + Duration::from_millis(ms);
        touch(&self.paths, &self.id);
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            tokio::time::sleep(remaining.min(Duration::from_secs(1))).await;
            touch(&self.paths, &self.id);
        }
        Ok(tool_success(format!("waited {ms}ms")))
    }

    #[tool(
        description = "Wait until the desktop stops changing. Returns once no pixels have changed for quiet_ms (default 500), or after timeout_ms (default 5000). Useful after launching an app."
    )]
    async fn wait_for_idle(
        &self,
        Parameters(params): Parameters<WaitForIdleParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("wait_for_idle", &params);
        let record = match state::load(&self.paths, &self.id) {
            Ok(record) => record,
            Err(error) => return Ok(tool_error(error)),
        };
        let desktop = match Desktop::new(&record, &self.paths) {
            Ok(desktop) => desktop,
            Err(error) => return Ok(tool_error(error)),
        };
        let timeout = params.timeout_ms.unwrap_or(5000).clamp(100, 120_000);
        let quiet = params.quiet_ms.unwrap_or(500).clamp(50, 10_000);
        let started = Instant::now();
        let mut last: Option<u64> = None;
        let mut stable_since = Instant::now();
        let mut last_touch = Instant::now();
        touch(&self.paths, &self.id);

        loop {
            let desktop_for_capture = desktop.clone();
            let fingerprint = match tokio::task::spawn_blocking(move || {
                desktop_for_capture.fingerprint()
            })
            .await
            {
                Ok(Ok(value)) => value,
                Ok(Err(error)) => return Ok(tool_error(error)),
                Err(error) => return Ok(tool_error(format!("internal error: {error}"))),
            };

            if last_touch.elapsed() >= Duration::from_secs(1) {
                touch(&self.paths, &self.id);
                last_touch = Instant::now();
            }

            match last {
                Some(previous) if previous == fingerprint => {
                    if stable_since.elapsed() >= Duration::from_millis(quiet) {
                        touch(&self.paths, &self.id);
                        return Ok(tool_success(format!(
                            "desktop has been idle for at least {quiet}ms"
                        )));
                    }
                }
                _ => {
                    last = Some(fingerprint);
                    stable_since = Instant::now();
                }
            }

            if started.elapsed() >= Duration::from_millis(timeout) {
                touch(&self.paths, &self.id);
                return Ok(tool_success(format!(
                    "timed out after {timeout}ms; the desktop is still changing"
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[tool(
        description = "Wait until a visible window matching a title substring and/or class substring exists. Returns the window as JSON. Use this instead of fixed sleeps after launch."
    )]
    async fn wait_for_window(
        &self,
        Parameters(params): Parameters<WaitForWindowParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("wait_for_window", &params);
        if params.title.is_none() && params.class.is_none() {
            return Ok(tool_error("provide at least one of title or class"));
        }
        let paths = self.paths.clone();
        let id = self.id.clone();
        let title = params.title;
        let class = params.class;
        let timeout = params.timeout_ms.unwrap_or(10_000);
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let window = wait_for_window_blocking(
                &paths,
                &record,
                title.as_deref(),
                class.as_deref(),
                timeout,
            )?;
            serde_json::to_string(&window).map_err(Into::into)
        })
        .await
    }

    #[tool(
        description = "Launch an application or shell command inside the runspace, for example \"xterm\" or \"python3 app.py\". The process stays alive between tool calls. Optionally wait for a window whose title/class matches before returning."
    )]
    async fn launch_app(
        &self,
        Parameters(params): Parameters<LaunchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("launch_app", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        let command = params.command;
        let cwd = params.cwd.map(PathBuf::from);
        let wait_ms = params.wait_ms.unwrap_or(300);
        let wait_title = params.wait_for_window;
        let wait_class = params.wait_for_window_class;
        let wait_timeout = params.wait_timeout_ms.unwrap_or(10_000);
        let wait_requested = wait_title.is_some() || wait_class.is_some();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let outcome =
                platform::launch_application(&record, &paths, &command, cwd.as_deref(), wait_ms)?;
            if wait_requested {
                let window = wait_for_window_blocking(
                    &paths,
                    &record,
                    wait_title.as_deref(),
                    wait_class.as_deref(),
                    wait_timeout,
                )?;
                Ok(format!(
                    "{}; window {} \"{}\" is ready",
                    outcome.message, window.id, window.title
                ))
            } else {
                Ok(outcome.message)
            }
        })
        .await
    }

    #[tool(
        description = "List visible top-level windows as JSON: id, title, class, position, size and pid. Use the id or title with focus_window and the other window tools."
    )]
    async fn list_windows(&self) -> Result<CallToolResult, McpError> {
        self.trace_tool("list_windows", &serde_json::json!({}));
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let windows = Desktop::new(&record, &paths)?.list_windows()?;
            serde_json::to_string(&windows).map_err(Into::into)
        })
        .await
    }

    #[tool(
        description = "Return a text hierarchy of visible windows and native controls as JSON. Use this instead of screenshots when you cannot see images. Windows includes control class, text, bounds, enabled/focused state; Linux exposes top-level windows."
    )]
    async fn get_ui_tree(&self) -> Result<CallToolResult, McpError> {
        self.trace_tool("get_ui_tree", &serde_json::json!({}));
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let tree = Desktop::new(&record, &paths)?.ui_tree()?;
            serde_json::to_string(&tree).map_err(Into::into)
        })
        .await
    }

    #[tool(
        description = "Capture one visible window as a PNG image. The window can be selected by id, exact title, title substring, or class. The image is cropped from the desktop canvas, so overlapping windows may appear."
    )]
    async fn screenshot_window(
        &self,
        Parameters(params): Parameters<WindowParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("screenshot_window", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        let target = params.id_or_title;
        let result = tokio::task::spawn_blocking(
            move || -> crate::Result<(platform::WindowInfo, Screenshot)> {
                let _record = state::load(&paths, &id)?;
                touch(&paths, &id);
                ops::capture_window(&id, &target)
            },
        )
        .await;

        match result {
            Ok(Ok((window, shot))) => {
                let data = base64::engine::general_purpose::STANDARD.encode(&shot.png);
                Ok(CallToolResult::success(vec![
                    ContentBlock::image(data, "image/png"),
                    ContentBlock::text(format!(
                        "window {} \"{}\" {}x{} ({} bytes PNG)",
                        window.id,
                        window.title,
                        shot.width,
                        shot.height,
                        shot.png.len()
                    )),
                ]))
            }
            Ok(Err(error)) => Ok(tool_error(error)),
            Err(error) => Ok(tool_error(format!("internal error: {error}"))),
        }
    }

    #[tool(
        description = "Find visible windows and native controls by case-insensitive text, class, or window filters. Returns JSON matches with absolute click coordinates. Control matches are preferred; when a platform exposes no controls, the window itself is matched. Use this for non-vision automation before clicking."
    )]
    async fn find_ui(
        &self,
        Parameters(params): Parameters<UiQueryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("find_ui", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        let text = params.text;
        let class = params.class;
        let window = params.window;
        let limit = params.limit.unwrap_or(20);
        blocking(move || {
            let _record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let query = ops::UiQuery {
                text: text.as_deref(),
                class: class.as_deref(),
                window: window.as_deref(),
                limit,
            };
            let result = ops::query_ui(&id, &query)?;
            serde_json::to_string(&result).map_err(Into::into)
        })
        .await
    }

    #[tool(
        description = "Wait until a visible window or native control matches case-insensitive text, class, or window filters. Returns the same JSON as find_ui once a match exists, or an error after timeout_ms (default 10000). Use this for non-vision automation instead of fixed sleeps."
    )]
    async fn wait_for_ui(
        &self,
        Parameters(params): Parameters<WaitForUiParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("wait_for_ui", &params);
        if params.text.is_none() && params.class.is_none() && params.window.is_none() {
            return Ok(tool_error("provide at least one of text, class, or window"));
        }
        let paths = self.paths.clone();
        let id = self.id.clone();
        let text = params.text;
        let class = params.class;
        let window = params.window;
        let limit = params.limit.unwrap_or(20);
        let timeout = params.timeout_ms.unwrap_or(10_000).clamp(100, 120_000);
        let result = tokio::task::spawn_blocking(move || -> crate::Result<String> {
            let _record = state::load(&paths, &id)?;
            let query = ops::UiQuery {
                text: text.as_deref(),
                class: class.as_deref(),
                window: window.as_deref(),
                limit,
            };
            let deadline = Instant::now() + Duration::from_millis(timeout);
            loop {
                touch(&paths, &id);
                let result = ops::query_ui(&id, &query)?;
                if !result.matches.is_empty() {
                    return serde_json::to_string(&result).map_err(Into::into);
                }
                if Instant::now() >= deadline {
                    return Err(crate::Error::invalid(format!(
                        "no UI match within {timeout}ms; use find_ui or get_ui_tree to inspect the current tree"
                    )));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .await;

        match result {
            Ok(Ok(json)) => Ok(tool_success(json)),
            Ok(Err(error)) => Ok(tool_error(error)),
            Err(error) => Ok(tool_error(format!("internal error: {error}"))),
        }
    }

    #[tool(
        description = "Click the center of the first enabled window or native control matching case-insensitive text, class, or window filters. Returns the matched element and click result as JSON. Use find_ui first when the target is ambiguous."
    )]
    async fn click_ui(
        &self,
        Parameters(params): Parameters<ClickUiParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("click_ui", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        let text = params.text;
        let class = params.class;
        let window = params.window;
        let limit = params.limit.unwrap_or(20);
        let button = params.button.unwrap_or_else(|| "left".into());
        let count = params.count.unwrap_or(1);
        blocking(move || {
            let _record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let query = ops::UiQuery {
                text: text.as_deref(),
                class: class.as_deref(),
                window: window.as_deref(),
                limit,
            };
            let (matched, message) = ops::click_ui(&id, &query, &button, count)?;
            serde_json::to_string(&serde_json::json!({
                "match": matched,
                "message": message,
            }))
            .map_err(Into::into)
        })
        .await
    }

    #[tool(
        description = "Click the first enabled window or control matching text, class, or window filters, then type input into it. Optionally select all existing text first and press Enter after typing. Returns the matched element and action log as JSON."
    )]
    async fn type_ui(
        &self,
        Parameters(params): Parameters<TypeUiParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("type_ui", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        let text = params.text;
        let class = params.class;
        let window = params.window;
        let limit = params.limit.unwrap_or(20);
        let input = params.input;
        let clear = params.clear.unwrap_or(false);
        let submit = params.submit.unwrap_or(false);
        blocking(move || {
            let _record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let query = ops::UiQuery {
                text: text.as_deref(),
                class: class.as_deref(),
                window: window.as_deref(),
                limit,
            };
            let (matched, message) = ops::type_ui(&id, &query, &input, clear, submit)?;
            serde_json::to_string(&serde_json::json!({
                "match": matched,
                "message": message,
            }))
            .map_err(Into::into)
        })
        .await
    }

    #[tool(description = "Focus (activate and raise) a window by id or by (part of) its title.")]
    async fn focus_window(
        &self,
        Parameters(params): Parameters<WindowParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("focus_window", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.focus_window(&params.id_or_title)
        })
        .await
    }

    #[tool(
        description = "Close a window by id or title. Returns an error if the application refuses to close (for example, an unsaved-changes prompt); retry with force=true to kill it."
    )]
    async fn close_window(
        &self,
        Parameters(params): Parameters<CloseWindowParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("close_window", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?
                .close_window(&params.id_or_title, params.force.unwrap_or(false))
        })
        .await
    }

    #[tool(description = "Move a window to absolute screen coordinates (x, y).")]
    async fn move_window(
        &self,
        Parameters(params): Parameters<MoveWindowParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("move_window", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.move_window(&params.id_or_title, params.x, params.y)
        })
        .await
    }

    #[tool(description = "Resize a window to width x height pixels.")]
    async fn resize_window(
        &self,
        Parameters(params): Parameters<ResizeWindowParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("resize_window", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            Desktop::new(&record, &paths)?.resize_window(
                &params.id_or_title,
                params.width,
                params.height,
            )
        })
        .await
    }

    #[tool(
        description = "Return metadata about this runspace as JSON: id, display, resolution, uptime, pids, memory usage, log path and idle timeout."
    )]
    async fn get_info(&self) -> Result<CallToolResult, McpError> {
        self.trace_tool("get_info", &serde_json::json!({}));
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let info = ops::info_for(&record, &paths);
            serde_json::to_string_pretty(&info).map_err(Into::into)
        })
        .await
    }

    #[tool(
        description = "Change the virtual screen resolution at runtime. Supported by invisible Xvfb runspaces and by the Windows backend. Visible Xephyr runspaces keep their creation size."
    )]
    async fn set_resolution(
        &self,
        Parameters(params): Parameters<ResolutionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("set_resolution", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            let message =
                Desktop::new(&record, &paths)?.set_resolution(params.width, params.height)?;
            state::update_resolution(&paths, &id, params.width, params.height)?;
            Ok(format!("{message}; runspace record updated"))
        })
        .await
    }

    #[tool(
        description = "Read the tail of the backend/application log. Use this to debug an app that failed to launch."
    )]
    async fn get_logs(
        &self,
        Parameters(params): Parameters<LogsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.trace_tool("get_logs", &params);
        let paths = self.paths.clone();
        let id = self.id.clone();
        blocking(move || {
            let record = state::load(&paths, &id)?;
            touch(&paths, &id);
            ops::log_tail(&record, &paths, params.lines.unwrap_or(200))
        })
        .await
    }

    #[tool(
        description = "Permanently destroy this runspace: kill every process, remove its display and delete all runtime data. The MCP server exits shortly after replying."
    )]
    async fn destroy(&self) -> Result<CallToolResult, McpError> {
        self.trace_tool("destroy", &serde_json::json!({}));
        let id = self.id.clone();
        let paths = self.paths.clone();
        let result = tokio::task::spawn_blocking(move || -> crate::Result<Vec<Record>> {
            let records = ops::destroy_targets(&[id])?;
            if let Ok(_lock) = paths.lock()
                && state::load_all(&paths).is_empty()
            {
                platform::cleanup_helpers(&paths);
            }
            Ok(records)
        })
        .await;
        match result {
            Ok(Ok(records)) => {
                // Give the JSON-RPC response time to reach the client before
                // this process (the transport) goes away.
                tokio::spawn(async {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    std::process::exit(0);
                });
                Ok(tool_success(format!(
                    "destroyed runspace {}; all processes and runtime data removed",
                    records
                        .first()
                        .map(|record| record.id.clone())
                        .unwrap_or_default()
                )))
            }
            Ok(Err(error)) => Ok(tool_error(error)),
            Err(error) => Ok(tool_error(format!("internal error: {error}"))),
        }
    }
}

#[tool_handler(
    name = "runspace",
    version = "0.2.0",
    instructions = "You control a disposable desktop. Screenshots are PNG images; screenshot_window captures one window. If you cannot see images, call get_ui_tree for a text hierarchy, find_ui and wait_for_ui to locate controls by text or class, click_ui to activate a control, and type_ui to click and type in one step. get_logs returns application output. Coordinates are absolute pixels with (0,0) at the top-left. wait_for_window removes startup races; launch_app can wait for a window directly. The desktop is isolated from the host and is destroyed with the destroy tool."
)]
impl ServerHandler for RunspaceServer {}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct MouseMoveParams {
    /// X coordinate in pixels
    pub x: i32,
    /// Y coordinate in pixels
    pub y: i32,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct PointParams {
    /// X coordinate in pixels (optional)
    pub x: Option<i32>,
    /// Y coordinate in pixels (optional)
    pub y: Option<i32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ClickParams {
    /// Mouse button: left, middle or right (default: left)
    pub button: Option<String>,
    /// X coordinate in pixels; provide together with y to move first
    pub x: Option<i32>,
    /// Y coordinate in pixels; provide together with x to move first
    pub y: Option<i32>,
    /// Number of clicks (default: 1)
    pub count: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct DragParams {
    /// Starting X coordinate
    pub from_x: i32,
    /// Starting Y coordinate
    pub from_y: i32,
    /// Ending X coordinate
    pub to_x: i32,
    /// Ending Y coordinate
    pub to_y: i32,
    /// Mouse button: left, middle or right (default: left)
    pub button: Option<String>,
    /// Duration in milliseconds (default: 300)
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ScrollParams {
    /// Direction: up, down, left or right
    pub direction: String,
    /// Number of wheel steps (default: 1)
    pub amount: Option<u32>,
    /// X coordinate to scroll at (optional)
    pub x: Option<i32>,
    /// Y coordinate to scroll at (optional)
    pub y: Option<i32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TypeParams {
    /// Text to type
    pub text: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct KeyParams {
    /// Key combination or list of combinations, for example "ctrl+s" or ["ctrl+c", "ctrl+v"]
    pub keys: Keys,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum Keys {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WaitParams {
    /// Milliseconds to wait (maximum 60000)
    pub ms: u64,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WaitForIdleParams {
    /// Maximum time to wait in milliseconds (default: 5000)
    pub timeout_ms: Option<u64>,
    /// How long the screen must stay unchanged in milliseconds (default: 500)
    pub quiet_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WaitForWindowParams {
    /// Case-insensitive substring of the window title
    pub title: Option<String>,
    /// Case-insensitive substring of the window class
    pub class: Option<String>,
    /// Maximum time to wait in milliseconds (default: 10000)
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LaunchParams {
    /// Shell command to run, for example "xterm" or "python3 app.py"
    pub command: String,
    /// Working directory (defaults to the MCP server's working directory)
    pub cwd: Option<String>,
    /// How long to wait for an immediate exit before returning (default: 300 ms)
    pub wait_ms: Option<u64>,
    /// Optional window title substring to wait for after launching
    pub wait_for_window: Option<String>,
    /// Optional window class substring to wait for after launching
    pub wait_for_window_class: Option<String>,
    /// Maximum time to wait for the window in milliseconds (default: 10000)
    pub wait_timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct UiQueryParams {
    /// Case-insensitive substring of control text or window title
    pub text: Option<String>,
    /// Case-insensitive substring of control or window class
    pub class: Option<String>,
    /// Restrict matches to a window id, title, or class
    pub window: Option<String>,
    /// Maximum number of matches (1-100, default 20)
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WaitForUiParams {
    /// Case-insensitive substring of control text or window title
    pub text: Option<String>,
    /// Case-insensitive substring of control or window class
    pub class: Option<String>,
    /// Restrict matches to a window id, title, or class
    pub window: Option<String>,
    /// Maximum number of matches (1-100, default 20)
    pub limit: Option<usize>,
    /// Maximum time to wait in milliseconds (default 10000)
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ClickUiParams {
    /// Case-insensitive substring of control text or window title
    pub text: Option<String>,
    /// Case-insensitive substring of control or window class
    pub class: Option<String>,
    /// Restrict matches to a window id, title, or class
    pub window: Option<String>,
    /// Maximum number of matches to inspect (1-100, default 20)
    pub limit: Option<usize>,
    /// Mouse button: left, middle or right (default: left)
    pub button: Option<String>,
    /// Number of clicks (default: 1)
    pub count: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TypeUiParams {
    /// Case-insensitive substring of control text or window title
    pub text: Option<String>,
    /// Case-insensitive substring of control or window class
    pub class: Option<String>,
    /// Restrict matches to a window id, title, or class
    pub window: Option<String>,
    /// Maximum number of matches to inspect (1-100, default 20)
    pub limit: Option<usize>,
    /// Text to type after clicking the match
    pub input: String,
    /// Select all existing text before typing (default: false)
    pub clear: Option<bool>,
    /// Press Enter after typing (default: false)
    pub submit: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WindowParams {
    /// Window id or part of its title
    pub id_or_title: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct CloseWindowParams {
    /// Window id or part of its title
    pub id_or_title: String,
    /// Kill the client if the window does not close gracefully (default: false)
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct MoveWindowParams {
    /// Window id or part of its title
    pub id_or_title: String,
    /// X coordinate
    pub x: i32,
    /// Y coordinate
    pub y: i32,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ResizeWindowParams {
    /// Window id or part of its title
    pub id_or_title: String,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ResolutionParams {
    /// Screen width in pixels
    pub width: u16,
    /// Screen height in pixels
    pub height: u16,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LogsParams {
    /// Number of trailing lines (default: 200)
    pub lines: Option<usize>,
}

fn touch(paths: &Paths, id: &str) {
    let _ = state::touch(paths, id);
}

fn wait_for_window_blocking(
    paths: &Paths,
    record: &Record,
    title: Option<&str>,
    class: Option<&str>,
    timeout_ms: u64,
) -> crate::Result<platform::WindowInfo> {
    ops::wait_for_window_on(record, paths, title, class, timeout_ms)
}

fn tool_success(message: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(message.into())])
}

fn tool_error(message: impl std::fmt::Display) -> CallToolResult {
    let raw = message.to_string();
    let mut text = raw.clone();
    let mut content = Vec::new();
    if let Some((paths, id)) = MCP_CONTEXT.get()
        && let Some(record) = state::try_load(paths, id)
    {
        if let Ok(log) = ops::log_tail(&record, paths, 40) {
            text.push_str("\n\n--- recent log ---\n");
            text.push_str(&log);
        }
        if let Ok(desktop) = Desktop::new(&record, paths) {
            if let Ok(tree) = desktop.ui_tree()
                && let Ok(tree) = serde_json::to_string(&tree)
            {
                let mut tree = tree;
                if tree.chars().count() > 8000 {
                    tree = tree.chars().take(8000).collect();
                    tree.push_str("...");
                }
                text.push_str("\n\n--- ui tree ---\n");
                text.push_str(&tree);
            }
            if let Ok(shot) = desktop.screenshot() {
                let data = base64::engine::general_purpose::STANDARD.encode(shot.png);
                content.push(ContentBlock::image(data, "image/png"));
            }
        }
        let _ = crate::trace::record_error(paths, id, &raw);
    }
    content.insert(0, ContentBlock::text(text));
    CallToolResult::error(content)
}

async fn blocking<F>(operation: F) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> crate::Result<String> + Send + 'static,
{
    match tokio::task::spawn_blocking(operation).await {
        Ok(Ok(message)) => Ok(tool_success(message)),
        Ok(Err(error)) => Ok(tool_error(error)),
        Err(error) => Ok(tool_error(format!("internal error: {error}"))),
    }
}

pub async fn run(target: Option<String>) -> anyhow::Result<()> {
    let paths = Paths::discover()?;
    let record = ops::resolve_active(&paths, target.as_deref())?;
    tracing::info!(
        "serving MCP for runspace {} on {}",
        record.id,
        record.display_name()
    );

    let server = RunspaceServer::new(paths.clone(), record.id.clone());
    let _ = MCP_CONTEXT.set((paths, record.id));
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
