use std::process::ExitCode;

use crate::cli::{
    AttachArgs, Commands, CreateArgs, DestroyArgs, InfoArgs, InspectArgs, ListArgs, LogsArgs,
    McpArgs, TraceArgs, WatchArgs,
};
use crate::{mcp, ops, ui, watch};

pub async fn run(command: Commands, quiet: bool) -> anyhow::Result<ExitCode> {
    match command {
        Commands::Create(args) => create(args, quiet),
        Commands::List(args) => list(args, quiet),
        Commands::Info(args) => info(args),
        Commands::Destroy(args) => destroy(args, quiet),
        Commands::Attach(args) => attach(args),
        Commands::Logs(args) => logs(args),
        Commands::Trace(args) => trace(args),
        Commands::Inspect(args) => inspect(args),
        Commands::Mcp(args) => mcp_command(args).await,
        Commands::Doctor => Ok(doctor()),
        Commands::Watch(args) => watch_command(args).await,
    }
}

fn create(args: CreateArgs, quiet: bool) -> anyhow::Result<ExitCode> {
    let (width, height) = ops::parse_resolution(&args.resolution)?;
    let record = ops::create(ops::CreateOptions {
        visible: args.visible,
        width,
        height,
        name: args.name,
        idle_timeout: args.idle_timeout,
        record: args.record,
    })?;

    if args.json {
        let info = ops::info(&record.id)?;
        println!("{}", serde_json::to_string_pretty(&info)?);
    } else if quiet {
        println!("{}", record.id);
    } else {
        println!();
        println!("{}", ui::bold(&format!("Runspace {} is ready", record.id)));
        let mut rows: Vec<(&str, String)> = Vec::new();
        if let Some(name) = &record.name {
            rows.push(("name", name.clone()));
        }
        rows.extend([
            ("mode", record.mode.as_str().to_string()),
            ("display", record.display_name()),
            ("resolution", record.resolution()),
            ("pid", record.backend_pid.to_string()),
        ]);
        if let Some(timeout) = record.idle_timeout {
            rows.push(("idle timeout", format!("{timeout}s")));
        }
        println!("{}", ui::kv(&rows));
        println!();
        println!("{}", ui::dim("Connect an MCP client with:"));
        println!("  runspace mcp {}", record.id);
        if record.mode == crate::state::Mode::Visible {
            println!(
                "{}",
                ui::dim(&format!("Watch it with: runspace attach {}", record.id))
            );
        }
        println!(
            "{}",
            ui::dim(&format!("Destroy it with: runspace destroy {}", record.id))
        );
        if record.recording {
            println!(
                "{}",
                ui::dim(&format!(
                    "Export the session trace with: runspace trace {} --html trace.html",
                    record.id
                ))
            );
        }
        println!();
    }
    Ok(ExitCode::SUCCESS)
}

fn list(args: ListArgs, quiet: bool) -> anyhow::Result<ExitCode> {
    let infos = ops::list()?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&infos)?);
        return Ok(ExitCode::SUCCESS);
    }

    if infos.is_empty() {
        if quiet {
            return Ok(ExitCode::SUCCESS);
        }
        println!("No active runspaces.");
        println!("{}", ui::dim("Create one with: runspace create"));
        return Ok(ExitCode::SUCCESS);
    }

    if quiet {
        for info in infos {
            println!("{}", info.id);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let rows = infos
        .iter()
        .map(|info| {
            vec![
                info.id.clone(),
                info.name.clone().unwrap_or_else(|| "-".into()),
                info.mode.as_str().to_string(),
                info.display.clone(),
                info.resolution.clone(),
                ui::duration(info.uptime_seconds),
                info.backend_pid.to_string(),
                ui::memory(info.memory_bytes),
            ]
        })
        .collect::<Vec<_>>();

    print!(
        "{}",
        ui::table(
            &[
                "ID",
                "NAME",
                "MODE",
                "DISPLAY",
                "RESOLUTION",
                "UPTIME",
                "PID",
                "MEMORY",
            ],
            &rows
        )
    );
    Ok(ExitCode::SUCCESS)
}

fn info(args: InfoArgs) -> anyhow::Result<ExitCode> {
    let info = ops::info(&args.target)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(ExitCode::SUCCESS);
    }

    println!("{}", ui::bold(&format!("Runspace {}", info.id)));
    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(name) = &info.name {
        rows.push(("name", name.clone()));
    }
    rows.extend([
        ("status", "running".into()),
        ("mode", info.mode.as_str().to_string()),
        ("backend", info.backend.clone()),
        ("display", info.display.clone()),
        ("resolution", info.resolution.clone()),
        ("uptime", ui::duration(info.uptime_seconds)),
        ("memory", ui::memory(info.memory_bytes)),
        ("backend pid", info.backend_pid.to_string()),
        (
            "wm pid",
            info.wm_pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "none".into()),
        ),
        (
            "idle timeout",
            info.idle_timeout_seconds
                .map(|seconds| format!("{seconds}s"))
                .unwrap_or_else(|| "none".into()),
        ),
        ("clipboard", info.clipboard.as_str().to_string()),
        (
            "recording",
            if info.recording { "yes" } else { "no" }.to_string(),
        ),
        ("log", info.log_path.clone()),
        ("mcp", info.mcp_command.clone()),
    ]);
    if let Some(desktop) = &info.desktop {
        rows.push(("desktop", desktop.clone()));
    }
    if let Some(job) = &info.job {
        rows.push(("job", job.clone()));
    }
    println!("{}", ui::kv(&rows));
    Ok(ExitCode::SUCCESS)
}

fn destroy(args: DestroyArgs, quiet: bool) -> anyhow::Result<ExitCode> {
    let destroyed = if args.all {
        ops::destroy_all()?
    } else {
        ops::destroy_targets(&args.targets)?
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&destroyed)?);
        return Ok(ExitCode::SUCCESS);
    }
    if quiet {
        return Ok(ExitCode::SUCCESS);
    }
    if destroyed.is_empty() {
        println!("No active runspaces.");
        return Ok(ExitCode::SUCCESS);
    }
    for record in destroyed {
        match record.name {
            Some(name) => println!("Destroyed {} {}", record.id, ui::dim(&format!("({name})"))),
            None => println!("Destroyed {}", record.id),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn attach(args: AttachArgs) -> anyhow::Result<ExitCode> {
    println!("{}", ops::attach(&args.target)?);
    Ok(ExitCode::SUCCESS)
}

fn logs(args: LogsArgs) -> anyhow::Result<ExitCode> {
    let text = ops::logs(&args.target, args.lines)?;
    println!("{text}");
    if args.follow {
        ops::follow_logs(&args.target)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn trace(args: TraceArgs) -> anyhow::Result<ExitCode> {
    let export = ops::export_trace(&args.target, args.html)?;
    println!("Wrote {}", export.path.display());
    println!("{} events, {} frames", export.events, export.frames);
    Ok(ExitCode::SUCCESS)
}

fn inspect(args: InspectArgs) -> anyhow::Result<ExitCode> {
    let (record, tree) = ops::ui_tree(&args.target)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&tree)?);
        return Ok(ExitCode::SUCCESS);
    }

    println!(
        "Runspace {}  backend={}  cursor={},{}  focused={}",
        record.id,
        tree.backend,
        tree.cursor_x,
        tree.cursor_y,
        tree.focused_window
            .map(|id| format!("{id}"))
            .unwrap_or_else(|| "none".into())
    );
    for window in &tree.windows {
        println!(
            "[{}] \"{}\" <{}> {}x{} at {},{} pid={}{}",
            window.window.id,
            window.window.title,
            window.window.class,
            window.window.width,
            window.window.height,
            window.window.x,
            window.window.y,
            window
                .window
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".into()),
            if window.focused { "  focused" } else { "" }
        );
        for control in &window.controls {
            let indent = "  ".repeat(control.depth as usize);
            println!(
                "  {}#{} <{}> \"{}\" {}x{} at {},{} enabled={} visible={} focused={}",
                indent,
                control.control_id,
                control.class,
                control.text,
                control.width,
                control.height,
                control.x,
                control.y,
                control.enabled,
                control.visible,
                control.focused
            );
        }
        if window.controls.is_empty() {
            println!("  (no native controls exposed)");
        } else if window.controls_truncated {
            println!("  (controls truncated)");
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn mcp_command(args: McpArgs) -> anyhow::Result<ExitCode> {
    mcp::run(args.target).await?;
    Ok(ExitCode::SUCCESS)
}

async fn watch_command(args: WatchArgs) -> anyhow::Result<ExitCode> {
    watch::run(args.target).await?;
    Ok(ExitCode::SUCCESS)
}

fn doctor() -> ExitCode {
    let report = ops::doctor();

    println!();
    println!("{}", ui::bold("runspace doctor"));
    println!();

    let rows = report
        .checks
        .iter()
        .map(|check| {
            let status = if check.ok {
                "ok"
            } else if check.required {
                "missing"
            } else {
                "optional"
            };
            vec![
                check.name.to_string(),
                status.to_string(),
                check.detail.clone(),
                check.purpose.to_string(),
            ]
        })
        .collect::<Vec<_>>();
    print!(
        "{}",
        ui::table(&["DEPENDENCY", "STATUS", "DETAIL", "PURPOSE"], &rows)
    );
    println!();

    for note in &report.notes {
        println!("  {note}");
    }
    println!();

    if report.ok {
        println!("{}", ui::green("All required dependencies are available."));
        println!(
            "Create your first runspace with: {}",
            ui::bold("runspace create")
        );
        ExitCode::SUCCESS
    } else {
        println!("{}", ui::red("Missing required dependencies."));
        if let Some(hint) = &report.install_hint {
            println!("Install them with:");
            println!("  {hint}");
        }
        ExitCode::FAILURE
    }
}
