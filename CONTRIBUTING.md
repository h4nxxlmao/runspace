# Contributing to Runspace

Thanks for helping improve Runspace. This project is an open-source, native GUI sandbox for AI agents, and contributions of all sizes are welcome.

By participating, you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to contribute

- Report reproducible bugs with logs and platform details.
- Improve documentation, installation scripts, or platform notes.
- Add tests for a control, app, or backend behavior.
- Fix an existing issue.
- Propose a focused feature that fits the project scope.

Open an issue before starting a large feature so the design can be discussed first.

## Development setup

### Requirements

- Rust 1.88 or newer (the crate uses edition 2024).
- Linux: Xvfb, Xephyr, xdotool, openbox, and x11-xserver-utils for end-to-end tests.
- Windows: Windows 10/11 x64 and the MSVC Rust toolchain.

Install dependencies for Debian/Ubuntu:

```sh
sudo apt-get install -y xvfb xserver-xephyr xdotool openbox x11-xserver-utils
```

### Build and check

```sh
cargo build
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

### End-to-end tests

Linux:

```sh
RUNSPACE_E2E=1 cargo test --test mcp_e2e -- --nocapture
```

Windows:

```powershell
cargo test --test windows_e2e -- --nocapture
```

The CI workflow runs formatting, clippy, tests, and release builds on Ubuntu and Windows.

## Pull requests

1. Keep the change focused and avoid unrelated refactors.
2. Add or update tests when behavior changes.
3. Update the README or platform notes when user-visible behavior changes.
4. Run the relevant checks locally.
5. Describe the problem, the approach, and the test commands in the pull request.

Small, reviewable pull requests are easier to merge than large rewrites.

## Code style

- Run `cargo fmt` before committing.
- Keep clippy clean with `-D warnings`.
- Prefer shared behavior in `src/core/` and platform-specific behavior behind `src/platform/`.
- Keep the CLI, state model, and MCP surface consistent across Linux and Windows.
- Do not add platform setup steps that require WSL, a guest OS, or a container.

## License

By contributing, you agree that your contribution is licensed under the GNU General Public License, version 3 or later (`GPL-3.0-or-later`), the same license as the project. See [LICENSE](LICENSE).
