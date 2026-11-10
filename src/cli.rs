use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Disposable virtual desktops for AI agents.
#[derive(Debug, Parser)]
#[command(
    name = "runspace",
    version,
    about = "Disposable virtual desktops for AI agents",
    arg_required_else_help = true
)]
pub struct Cli {
    /// Increase log verbosity (-v, -vv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Create a new runspace
    Create(CreateArgs),

    /// List active runspaces
    #[command(alias = "ls")]
    List(ListArgs),

    /// Show detailed information about a runspace
    Info(InfoArgs),

    /// Destroy one or more runspaces and clean up everything
    #[command(alias = "rm")]
    Destroy(DestroyArgs),

    /// Bring a visible runspace's window to the front
    Attach(AttachArgs),

    /// Show the backend log
    Logs(LogsArgs),

    /// Export a recorded session as a self-contained HTML trace
    Trace(TraceArgs),

    /// Print a text hierarchy of windows and native controls
    Inspect(InspectArgs),

    /// Launch a command inside a runspace
    Launch(LaunchArgs),

    /// Capture the desktop to a PNG file
    Screenshot(ScreenshotArgs),

    /// List visible windows in a runspace
    Windows(WindowsArgs),

    /// Find windows or native controls by text, class, or window
    Find(FindArgs),

    /// Click the center of the first matching UI control or window
    ClickControl(ClickControlArgs),

    /// Click a matching control and type into it
    TypeControl(TypeControlArgs),

    /// Serve the MCP server for a runspace over stdio
    Mcp(McpArgs),

    /// Check system dependencies
    Doctor(DoctorArgs),

    #[command(name = "__watch", hide = true)]
    Watch(WatchArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Show a live viewer window (Xephyr on Linux, native viewer on Windows)
    #[arg(long)]
    pub visible: bool,

    /// Screen size, for example 1280x800
    #[arg(short, long, default_value = "1280x800", value_name = "WxH")]
    pub resolution: String,

    /// Human-friendly name for this runspace
    #[arg(short, long, value_name = "NAME")]
    pub name: Option<String>,

    /// Destroy automatically after this many seconds without MCP activity
    #[arg(long, value_name = "SECONDS")]
    pub idle_timeout: Option<u64>,

    /// Record every MCP tool call and screenshot for `runspace trace`
    #[arg(long)]
    pub record: bool,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DestroyArgs {
    /// Runspace ids, unique prefixes, or names
    #[arg(
        value_name = "ID|NAME",
        num_args = 1..,
        required_unless_present = "all"
    )]
    pub targets: Vec<String>,

    /// Destroy every runspace
    #[arg(long, conflicts_with = "targets")]
    pub all: bool,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct AttachArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,

    /// Follow the log as it grows
    #[arg(short, long)]
    pub follow: bool,

    /// Number of trailing lines to show
    #[arg(short = 'n', long, default_value_t = 200, value_name = "LINES")]
    pub lines: usize,
}

#[derive(Debug, Args)]
pub struct McpArgs {
    /// Runspace id, unique prefix, or name (optional when exactly one is active)
    #[arg(value_name = "ID|NAME")]
    pub target: Option<String>,
}

#[derive(Debug, Args)]
pub struct TraceArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,

    /// Output HTML path (default: ./runspace-<id>-trace.html)
    #[arg(short = 'o', long, value_name = "FILE")]
    pub html: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct InspectArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct LaunchArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,

    /// Shell command string, for example "notepad.exe" or "python app.py"
    #[arg(value_name = "COMMAND", allow_hyphen_values = true)]
    pub command: String,

    /// Working directory for the launched command
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Wait for a window whose title contains this text
    #[arg(long, value_name = "TEXT")]
    pub wait_for: Option<String>,

    /// Wait for a window whose class contains this text
    #[arg(long, value_name = "CLASS")]
    pub wait_class: Option<String>,

    /// Maximum time to wait for the window in milliseconds
    #[arg(long, default_value_t = 10_000, value_name = "MS")]
    pub timeout_ms: u64,

    /// How long to wait for an immediate exit before returning
    #[arg(long, default_value_t = 300, value_name = "MS")]
    pub wait_ms: u64,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ScreenshotArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,

    /// Output PNG path (default: ./runspace-<id>.png)
    #[arg(short = 'o', long, value_name = "FILE")]
    pub out: Option<PathBuf>,

    /// Crop the screenshot to a window id, title, or class
    #[arg(long, value_name = "WINDOW")]
    pub window: Option<String>,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct WindowsArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct FindArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,

    /// Case-insensitive substring of control text or window title
    #[arg(long, value_name = "TEXT")]
    pub text: Option<String>,

    /// Case-insensitive substring of control or window class
    #[arg(long, value_name = "CLASS")]
    pub class: Option<String>,

    /// Restrict results to a window id, title, or class
    #[arg(long, value_name = "WINDOW")]
    pub window: Option<String>,

    /// Maximum number of matches (1-100)
    #[arg(long, default_value_t = 20, value_name = "N")]
    pub limit: usize,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ClickControlArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,

    /// Case-insensitive substring of control text or window title
    #[arg(long, value_name = "TEXT")]
    pub text: Option<String>,

    /// Case-insensitive substring of control or window class
    #[arg(long, value_name = "CLASS")]
    pub class: Option<String>,

    /// Restrict the match to a window id, title, or class
    #[arg(long, value_name = "WINDOW")]
    pub window: Option<String>,

    /// Maximum number of matches to inspect (1-100)
    #[arg(long, default_value_t = 20, value_name = "N")]
    pub limit: usize,

    /// Mouse button: left, middle, or right
    #[arg(long, default_value = "left")]
    pub button: String,

    /// Number of clicks
    #[arg(long, default_value_t = 1)]
    pub count: u32,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct TypeControlArgs {
    /// Runspace id, unique prefix, or name
    #[arg(value_name = "ID|NAME")]
    pub target: String,

    /// Text to type
    #[arg(long, value_name = "TEXT")]
    pub input: String,

    /// Case-insensitive substring of control text or window title
    #[arg(long, value_name = "TEXT")]
    pub text: Option<String>,

    /// Case-insensitive substring of control or window class
    #[arg(long, value_name = "CLASS")]
    pub class: Option<String>,

    /// Restrict the match to a window id, title, or class
    #[arg(long, value_name = "WINDOW")]
    pub window: Option<String>,

    /// Maximum number of matches to inspect (1-100)
    #[arg(long, default_value_t = 20, value_name = "N")]
    pub limit: usize,

    /// Select all existing text before typing
    #[arg(long)]
    pub clear: bool,

    /// Press Enter after typing
    #[arg(long)]
    pub submit: bool,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct WatchArgs {
    #[arg(value_name = "ID")]
    pub target: String,
}
