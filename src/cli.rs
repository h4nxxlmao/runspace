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

    /// Serve the MCP server for a runspace over stdio
    Mcp(McpArgs),

    /// Check system dependencies
    Doctor,

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
pub struct WatchArgs {
    #[arg(value_name = "ID")]
    pub target: String,
}
