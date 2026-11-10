# Changelog

All notable changes to Runspace are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-11-01

### Added

- CLI `launch`, `screenshot`, `windows`, `find`, `click-control`, and `type-control` commands.
- Window-cropped screenshots through `runspace screenshot --window` and MCP `screenshot_window`.
- `runspace doctor --json` for machine-readable dependency reports.
- MCP `find_ui`, `wait_for_ui`, `click_ui`, and `type_ui` tools for text-only agents.
- Shared UI matcher with absolute click coordinates and enabled-control filtering.
- One-call click-and-type workflows with optional select-all and submit.

## [0.1.0] - 2026-11-01

### Added

- Native Linux backend using Xvfb/Xephyr, xdotool, openbox, and in-process x11rb capture.
- Native Windows backend using hidden Win32 desktops, job objects, PrintWindow, and posted input messages.
- CLI lifecycle: `create`, `list`, `info`, `attach`, `logs`, `trace`, `inspect`, `destroy`, `mcp`, and `doctor`.
- 24 MCP tools for desktop input, windows, UI trees, app launch, logs, and runspace lifecycle.
- Disposable process trees with zero per-runspace disk footprint after destroy.
- `wait_for_window` and launch-time window waiting for startup races.
- `get_ui_tree` for non-vision agents.
- Automatic failure context with log tail, UI tree, and screenshot.
- Opt-in session recording and self-contained HTML trace export.
- Visible Windows viewer and attach-on-demand promotion.
- Linux and Windows end-to-end tests.
- GPL-3.0-or-later open-source licensing.

[Unreleased]: https://github.com/h4nxxlmao/runspace/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/h4nxxlmao/runspace/releases/tag/v0.2.0
[0.1.0]: https://github.com/h4nxxlmao/runspace/releases/tag/v0.1.0
