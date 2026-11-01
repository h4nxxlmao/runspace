use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, ImageFormat, VisualClass};

use super::process;
use crate::core::state::{Mode, Record};
use crate::core::types::{RawImage, Screenshot, UiTree, UiWindow, WindowInfo};
use crate::{Error, Paths, Result};

#[derive(Debug, Clone)]
pub struct Desktop {
    display: u16,
    mode: Mode,
    id: String,
}

impl Desktop {
    pub fn new(record: &Record, _paths: &Paths) -> Result<Self> {
        Ok(Self {
            display: record.display,
            mode: record.mode,
            id: record.id.clone(),
        })
    }

    pub fn display(&self) -> u16 {
        self.display
    }

    fn display_name(&self) -> String {
        format!(":{}", self.display)
    }

    fn xdotool(&self, args: &[String]) -> Result<String> {
        let output = self.xdotool_raw(args)?;
        if !output.status.success() {
            return Err(Error::command_failed("xdotool", &output));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }

    fn xdotool_raw(&self, args: &[String]) -> Result<Output> {
        Command::new("xdotool")
            .args(args)
            .env("DISPLAY", self.display_name())
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    Error::MissingTool {
                        tool: "xdotool",
                        install: process::X11_INSTALL,
                    }
                } else {
                    Error::Spawn {
                        program: "xdotool".into(),
                        source,
                    }
                }
            })
    }

    pub fn mouse_move(&self, x: i32, y: i32) -> Result<String> {
        validate_point(x, y)?;
        self.xdotool(&["mousemove".into(), x.to_string(), y.to_string()])?;
        Ok(format!(
            "moved cursor to {x},{y} on {}",
            self.display_name()
        ))
    }

    pub fn click(
        &self,
        button: &str,
        x: Option<i32>,
        y: Option<i32>,
        count: u32,
    ) -> Result<String> {
        let button = button_code(button)?;
        if x.is_some() != y.is_some() {
            return Err(Error::invalid("provide both x and y, or neither"));
        }
        if let (Some(x), Some(y)) = (x, y) {
            self.mouse_move(x, y)?;
        }
        let count = count.clamp(1, 10);
        self.xdotool(&[
            "click".into(),
            "--repeat".into(),
            count.to_string(),
            "--delay".into(),
            "50".into(),
            button.to_string(),
        ])?;
        Ok(format!(
            "clicked button {button} {count} time(s) on {}",
            self.display_name()
        ))
    }

    pub fn drag(
        &self,
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
        button: &str,
        duration_ms: u64,
    ) -> Result<String> {
        validate_point(from_x, from_y)?;
        validate_point(to_x, to_y)?;
        let button = button_code(button)?;
        let duration_ms = duration_ms.clamp(0, 10_000);
        let steps: i32 = 12;

        let result = (|| -> Result<()> {
            self.mouse_move(from_x, from_y)?;
            self.xdotool(&["mousedown".into(), button.to_string()])?;
            for step in 1..=steps {
                let x = from_x + (to_x - from_x) * step / steps;
                let y = from_y + (to_y - from_y) * step / steps;
                self.xdotool(&["mousemove".into(), x.to_string(), y.to_string()])?;
                if duration_ms > 0 {
                    thread::sleep(Duration::from_millis(duration_ms / steps as u64));
                }
            }
            Ok(())
        })();

        let release = self.xdotool(&["mouseup".into(), button.to_string()]);
        result?;
        release?;
        Ok(format!(
            "dragged from {from_x},{from_y} to {to_x},{to_y} on {}",
            self.display_name()
        ))
    }

    pub fn scroll(
        &self,
        direction: &str,
        amount: u32,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<String> {
        let direction = direction.to_ascii_lowercase();
        let button = match direction.as_str() {
            "up" => 4,
            "down" => 5,
            "left" => 6,
            "right" => 7,
            other => {
                return Err(Error::invalid(format!(
                    "unknown scroll direction '{other}' (use up, down, left or right)"
                )));
            }
        };
        if x.is_some() != y.is_some() {
            return Err(Error::invalid("provide both x and y, or neither"));
        }
        if let (Some(x), Some(y)) = (x, y) {
            self.mouse_move(x, y)?;
        }
        let amount = amount.clamp(1, 100);
        self.xdotool(&[
            "click".into(),
            "--repeat".into(),
            amount.to_string(),
            "--delay".into(),
            "8".into(),
            button.to_string(),
        ])?;
        Ok(format!(
            "scrolled {direction} {amount} step(s) on {}",
            self.display_name()
        ))
    }

    pub fn type_text(&self, text: &str) -> Result<String> {
        if text.is_empty() {
            return Err(Error::invalid("text cannot be empty"));
        }
        if text.len() > 100_000 {
            return Err(Error::invalid("text is too long (limit: 100000 bytes)"));
        }
        self.xdotool(&[
            "type".into(),
            "--clearmodifiers".into(),
            "--delay".into(),
            "12".into(),
            "--".into(),
            text.to_string(),
        ])?;
        Ok(format!(
            "typed {} character(s) into the focused window on {}",
            text.chars().count(),
            self.display_name()
        ))
    }

    pub fn key(&self, combos: &[String]) -> Result<String> {
        if combos.is_empty() {
            return Err(Error::invalid("provide at least one key combination"));
        }
        for combo in combos {
            validate_key_combo(combo)?;
        }
        for combo in combos {
            self.xdotool(&["key".into(), "--clearmodifiers".into(), combo.clone()])?;
            thread::sleep(Duration::from_millis(20));
        }
        Ok(format!(
            "pressed {} on {}",
            combos.join(", "),
            self.display_name()
        ))
    }

    pub fn cursor_position(&self) -> Result<(i32, i32)> {
        let output = self.xdotool(&["getmouselocation".into(), "--shell".into()])?;
        let map = parse_shell(&output);
        let x = map
            .get("X")
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| Error::invalid("could not read cursor position"))?;
        let y = map
            .get("Y")
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| Error::invalid("could not read cursor position"))?;
        Ok((x, y))
    }

    pub fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        let output = self.xdotool_raw(&[
            "search".into(),
            "--onlyvisible".into(),
            "--name".into(),
            ".*".into(),
        ])?;
        if !output.status.success() {
            return Ok(Vec::new());
        }

        let mut ids: Vec<u64> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect();
        ids.sort_unstable();
        ids.dedup();

        let mut windows = Vec::new();
        for id in ids {
            let Ok(geometry) = self.window_geometry(id) else {
                continue;
            };
            if geometry.width <= 1 || geometry.height <= 1 {
                continue;
            }
            let title = self
                .xdotool(&["getwindowname".into(), id.to_string()])
                .unwrap_or_default();
            let class = self
                .xdotool(&["getwindowclassname".into(), id.to_string()])
                .unwrap_or_default();
            if title.is_empty() && class.is_empty() {
                continue;
            }
            let pid = self
                .xdotool(&["getwindowpid".into(), id.to_string()])
                .ok()
                .and_then(|pid| pid.trim().parse::<u32>().ok());
            windows.push(WindowInfo {
                id,
                title,
                class,
                x: geometry.x,
                y: geometry.y,
                width: geometry.width,
                height: geometry.height,
                pid,
            });
        }
        Ok(windows)
    }

    pub fn ui_tree(&self) -> Result<UiTree> {
        let windows = self.list_windows()?;
        let active = self.active_window().ok().flatten();
        let (cursor_x, cursor_y) = self.cursor_position()?;
        let windows = windows
            .into_iter()
            .map(|window| UiWindow {
                focused: active == Some(window.id),
                window,
                controls: Vec::new(),
                controls_truncated: false,
            })
            .collect();
        Ok(UiTree {
            backend: "x11".into(),
            windows,
            focused_window: active,
            cursor_x,
            cursor_y,
        })
    }

    fn active_window(&self) -> Result<Option<u64>> {
        let (conn, screen_num) =
            x11rb::connect(Some(&self.display_name())).map_err(|error| Error::Display {
                display: self.display,
                reason: error.to_string(),
            })?;
        let root = conn.setup().roots[screen_num].root;
        if let Ok(atom) = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")
            .map_err(|e| x11_error(self.display, e))?
            .reply()
            && let Ok(property) = conn
                .get_property(false, root, atom.atom, AtomEnum::WINDOW, 0, 1)
                .map_err(|e| x11_error(self.display, e))?
                .reply()
            && let Some(active) = property.value32().and_then(|mut values| values.next())
            && active != 0
        {
            return Ok(Some(u64::from(active)));
        }

        let focus = conn
            .get_input_focus()
            .map_err(|e| x11_error(self.display, e))?
            .reply()
            .map_err(|e| x11_error(self.display, e))?
            .focus;
        if focus == 0 {
            return Ok(None);
        }
        let mut current = focus;
        for _ in 0..32 {
            let tree = conn
                .query_tree(current)
                .map_err(|e| x11_error(self.display, e))?
                .reply()
                .map_err(|e| x11_error(self.display, e))?;
            if tree.parent == root || tree.parent == 0 {
                return Ok(Some(u64::from(current)));
            }
            current = tree.parent;
        }
        Ok(None)
    }

    pub fn focus_window(&self, target: &str) -> Result<String> {
        let id = self.resolve_window(target)?;
        let title = self
            .xdotool(&["getwindowname".into(), id.to_string()])
            .unwrap_or_else(|_| target.to_string());
        let _ = self.xdotool(&["windowmap".into(), id.to_string()]);
        if self
            .xdotool(&["windowactivate".into(), id.to_string()])
            .is_err()
        {
            self.xdotool(&["windowfocus".into(), id.to_string()])?;
        }
        let _ = self.xdotool(&["windowraise".into(), id.to_string()]);
        Ok(format!("focused window {id} \"{title}\""))
    }

    pub fn close_window(&self, target: &str, force: bool) -> Result<String> {
        let id = self.resolve_window(target)?;
        let title = self
            .xdotool(&["getwindowname".into(), id.to_string()])
            .unwrap_or_else(|_| target.to_string());

        let _ = self.xdotool(&["windowclose".into(), id.to_string()]);
        thread::sleep(Duration::from_millis(300));
        if !self.window_exists(id) {
            return Ok(format!("closed window {id} \"{title}\""));
        }

        if force {
            self.xdotool(&["windowkill".into(), id.to_string()])?;
            thread::sleep(Duration::from_millis(150));
            if !self.window_exists(id) {
                return Ok(format!("force-closed window {id} \"{title}\""));
            }
        }

        Err(Error::invalid(format!(
            "window {id} \"{title}\" is still open; it may be asking to save changes. Retry with force=true to kill it."
        )))
    }

    pub fn move_window(&self, target: &str, x: i32, y: i32) -> Result<String> {
        let id = self.resolve_window(target)?;
        self.xdotool(&[
            "windowmove".into(),
            id.to_string(),
            x.to_string(),
            y.to_string(),
        ])?;
        let geometry = self.window_geometry(id)?;
        Ok(format!(
            "moved window {id} to {},{}, size {}x{}",
            geometry.x, geometry.y, geometry.width, geometry.height
        ))
    }

    pub fn resize_window(&self, target: &str, width: u32, height: u32) -> Result<String> {
        if !(1..=10000).contains(&width) || !(1..=10000).contains(&height) {
            return Err(Error::invalid(
                "width and height must be between 1 and 10000",
            ));
        }
        let id = self.resolve_window(target)?;
        self.xdotool(&[
            "windowsize".into(),
            id.to_string(),
            width.to_string(),
            height.to_string(),
        ])?;
        let geometry = self.window_geometry(id)?;
        Ok(format!(
            "resized window {id} to {}x{} at {},{}",
            geometry.width, geometry.height, geometry.x, geometry.y
        ))
    }

    fn resolve_window(&self, target: &str) -> Result<u64> {
        let target = target.trim();
        if target.is_empty() {
            return Err(Error::invalid("window id or title cannot be empty"));
        }

        if let Ok(id) = target.parse::<u64>() {
            if self.window_exists(id) {
                return Ok(id);
            }
            return Err(Error::invalid(format!(
                "window {id} does not exist on {}; use list_windows to see what is open",
                self.display_name()
            )));
        }

        let pattern = escape_regex(target);
        let ids = self.search(&pattern);
        if !ids.is_empty() {
            if let Some(exact) = ids.iter().copied().find(|id| {
                self.xdotool(&["getwindowname".into(), id.to_string()])
                    .is_ok_and(|title| title.eq_ignore_ascii_case(target))
            }) {
                return Ok(exact);
            }
            return Ok(ids[0]);
        }

        let by_class = self.search_class(&pattern);
        if let Some(id) = by_class.first() {
            return Ok(*id);
        }

        Err(Error::invalid(format!(
            "no window matching '{target}' on {}; use list_windows to see what is open",
            self.display_name()
        )))
    }

    fn search(&self, pattern: &str) -> Vec<u64> {
        self.search_with_flag("--name", pattern)
    }

    fn search_class(&self, pattern: &str) -> Vec<u64> {
        self.search_with_flag("--class", pattern)
    }

    fn search_with_flag(&self, flag: &str, pattern: &str) -> Vec<u64> {
        let output = self.xdotool_raw(&[
            "search".into(),
            "--onlyvisible".into(),
            flag.into(),
            pattern.to_string(),
        ]);
        let Ok(output) = output else {
            return Vec::new();
        };
        let mut ids: Vec<u64> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn window_exists(&self, id: u64) -> bool {
        self.window_geometry(id).is_ok()
    }

    fn window_geometry(&self, id: u64) -> Result<Geometry> {
        let output =
            self.xdotool(&["getwindowgeometry".into(), "--shell".into(), id.to_string()])?;
        Geometry::parse(&output)
    }

    pub fn set_resolution(&self, width: u16, height: u16) -> Result<String> {
        validate_resolution(width, height)?;
        if self.mode == Mode::Visible {
            return Err(Error::Unsupported(format!(
                "{} is a visible Xephyr runspace, which cannot resize its nested screen at runtime.\nDestroy and recreate with the desired --resolution, or use invisible mode for live resizing.",
                self.id
            )));
        }
        process::require_tool("xrandr", "sudo apt-get install -y x11-xserver-utils")?;

        let display_name = self.display_name();
        let size = format!("{width}x{height}");
        let output = Command::new("xrandr")
            .args(["--display", &display_name, "--fb", &size])
            .output()
            .map_err(|source| Error::Spawn {
                program: "xrandr".into(),
                source,
            })?;

        // Xvfb accepts shrinking the framebuffer but still exits non-zero when
        // the old output mode no longer fits. The geometry is the ground truth.
        match self.screen_size() {
            Ok((actual_width, actual_height))
                if actual_width == width && actual_height == height =>
            {
                return Ok(format!(
                    "resolution is now {width}x{height} on {}",
                    self.display_name()
                ));
            }
            Ok((actual_width, actual_height)) if output.status.success() => {
                return Err(Error::invalid(format!(
                    "xrandr reported success but the screen is still {actual_width}x{actual_height}"
                )));
            }
            Ok(_) => {}
            Err(error) if output.status.success() => return Err(error),
            Err(_) => {}
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::Command {
            program: "xrandr".into(),
            message: format!(
                "{}\nThe screen can only grow up to the resolution chosen at creation; recreate the runspace with a larger --resolution to allow that.",
                stderr.trim()
            ),
        })
    }

    pub fn screen_size(&self) -> Result<(u16, u16)> {
        let (conn, screen_num) =
            x11rb::connect(Some(&self.display_name())).map_err(|error| Error::Display {
                display: self.display,
                reason: error.to_string(),
            })?;
        let screen = &conn.setup().roots[screen_num];
        let geometry = conn
            .get_geometry(screen.root)
            .map_err(|e| x11_error(self.display, e))?
            .reply()
            .map_err(|e| x11_error(self.display, e))?;
        Ok((geometry.width, geometry.height))
    }

    /// Give the virtual desktop a quiet dark background instead of the default
    /// pure black. Purely cosmetic, but it makes screenshots easier to read.
    pub fn set_root_background(&self, red: u8, green: u8, blue: u8) -> Result<()> {
        set_root_background(self.display, red, green, blue)
    }

    /// Capture the entire virtual screen as RGBA pixels.
    pub fn capture_rgba(&self) -> Result<RawImage> {
        let (conn, screen_num) =
            x11rb::connect(Some(&self.display_name())).map_err(|error| Error::Display {
                display: self.display,
                reason: error.to_string(),
            })?;

        let screen = &conn.setup().roots[screen_num];
        let geometry = conn
            .get_geometry(screen.root)
            .map_err(|e| x11_error(self.display, e))?
            .reply()
            .map_err(|e| x11_error(self.display, e))?;

        let image = conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                screen.root,
                0,
                0,
                geometry.width,
                geometry.height,
                u32::MAX,
            )
            .map_err(|e| x11_error(self.display, e))?
            .reply()
            .map_err(|e| x11_error(self.display, e))?;

        let visual = screen
            .allowed_depths
            .iter()
            .flat_map(|depth| depth.visuals.iter())
            .find(|visual| visual.visual_id == screen.root_visual)
            .ok_or_else(|| Error::Display {
                display: self.display,
                reason: "root visual not found".into(),
            })?;

        if visual.class != VisualClass::TRUE_COLOR {
            return Err(Error::Display {
                display: self.display,
                reason: "only TrueColor visuals are supported".into(),
            });
        }

        let bytes_per_pixel = match image.depth {
            0..=8 => 1,
            9..=16 => 2,
            _ => 4,
        };
        let width = geometry.width as usize;
        let height = geometry.height as usize;
        let stride = (width * bytes_per_pixel).div_ceil(4) * 4;
        let expected = stride * height;
        if image.data.len() < expected {
            return Err(Error::Display {
                display: self.display,
                reason: format!(
                    "short image buffer ({} bytes, expected {expected})",
                    image.data.len()
                ),
            });
        }

        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            let row = &image.data[y * stride..(y + 1) * stride];
            for x in 0..width {
                let offset = x * bytes_per_pixel;
                let pixel = match bytes_per_pixel {
                    1 => row[offset] as u32,
                    2 => u16::from_le_bytes([row[offset], row[offset + 1]]) as u32,
                    _ => u32::from_le_bytes([
                        row[offset],
                        row[offset + 1],
                        row[offset + 2],
                        row[offset + 3],
                    ]),
                };
                rgba.push(scale_channel(pixel, visual.red_mask));
                rgba.push(scale_channel(pixel, visual.green_mask));
                rgba.push(scale_channel(pixel, visual.blue_mask));
                rgba.push(255);
            }
        }

        Ok(RawImage {
            width: u32::from(geometry.width),
            height: u32::from(geometry.height),
            rgba,
        })
    }

    pub fn screenshot(&self) -> Result<Screenshot> {
        let raw = self.capture_rgba()?;
        let png = encode_png(&raw)?;
        Ok(Screenshot {
            width: raw.width,
            height: raw.height,
            png,
        })
    }

    pub fn fingerprint(&self) -> Result<u64> {
        let raw = self.capture_rgba()?;
        let mut hasher = DefaultHasher::new();
        raw.width.hash(&mut hasher);
        raw.height.hash(&mut hasher);
        raw.rgba.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

/// Give a display a quiet dark background. Uses a solid 1x1 pixmap, like
/// xsetroot, because the root may already have a background pixmap.
pub fn set_root_background(display: u16, red: u8, green: u8, blue: u8) -> Result<()> {
    use x11rb::protocol::xproto::{
        ChangeWindowAttributesAux, ConnectionExt as _, CreateGCAux, Rectangle,
    };
    use x11rb::wrapper::ConnectionExt as _;

    let display_name = format!(":{display}");
    let (conn, screen_num) =
        x11rb::connect(Some(&display_name)).map_err(|error| Error::Display {
            display,
            reason: error.to_string(),
        })?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let pixel = (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);

    let pixmap = conn.generate_id().map_err(|e| x11_error(display, e))?;
    conn.create_pixmap(screen.root_depth, pixmap, root, 1, 1)
        .map_err(|e| x11_error(display, e))?;
    let gc = conn.generate_id().map_err(|e| x11_error(display, e))?;
    conn.create_gc(gc, pixmap, &CreateGCAux::new().foreground(pixel))
        .map_err(|e| x11_error(display, e))?;
    conn.poly_fill_rectangle(
        pixmap,
        gc,
        &[Rectangle {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }],
    )
    .map_err(|e| x11_error(display, e))?;

    let attributes = ChangeWindowAttributesAux::new().background_pixmap(pixmap);
    conn.change_window_attributes(root, &attributes)
        .map_err(|e| x11_error(display, e))?;
    conn.clear_area(false, root, 0, 0, 0, 0)
        .map_err(|e| x11_error(display, e))?;
    conn.free_gc(gc).map_err(|e| x11_error(display, e))?;
    conn.free_pixmap(pixmap)
        .map_err(|e| x11_error(display, e))?;
    conn.sync().map_err(|e| x11_error(display, e))?;
    Ok(())
}

#[derive(Debug)]
struct Geometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl Geometry {
    fn parse(shell: &str) -> Result<Self> {
        let map = parse_shell(shell);
        let get = |key: &str| {
            map.get(key)
                .ok_or_else(|| Error::invalid(format!("xdotool geometry output is missing {key}")))
        };
        Ok(Self {
            x: get("X")?.parse().unwrap_or(0),
            y: get("Y")?.parse().unwrap_or(0),
            width: get("WIDTH")?.parse().unwrap_or(0),
            height: get("HEIGHT")?.parse().unwrap_or(0),
        })
    }
}

fn parse_shell(output: &str) -> BTreeMap<String, String> {
    output
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect()
}

fn button_code(button: &str) -> Result<u32> {
    match button.to_ascii_lowercase().as_str() {
        "left" | "1" => Ok(1),
        "middle" | "2" => Ok(2),
        "right" | "3" => Ok(3),
        other => Err(Error::invalid(format!(
            "unknown mouse button '{other}' (use left, middle or right)"
        ))),
    }
}

fn validate_point(x: i32, y: i32) -> Result<()> {
    if !(-100_000..=100_000).contains(&x) || !(-100_000..=100_000).contains(&y) {
        return Err(Error::invalid(
            "coordinates must be between -100000 and 100000",
        ));
    }
    Ok(())
}

pub fn validate_resolution(width: u16, height: u16) -> Result<()> {
    if !(320..=7680).contains(&width) || !(240..=4320).contains(&height) {
        return Err(Error::invalid(
            "resolution must be between 320x240 and 7680x4320",
        ));
    }
    Ok(())
}

fn validate_key_combo(combo: &str) -> Result<()> {
    if combo.is_empty() || combo.len() > 128 {
        return Err(Error::invalid("key combination must be 1-128 characters"));
    }
    if !combo
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '_' | '-'))
    {
        return Err(Error::invalid(format!(
            "invalid key combination '{combo}' (examples: ctrl+s, alt+F4, super+Return)"
        )));
    }
    Ok(())
}

fn escape_regex(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len() * 2);
    for ch in input.chars() {
        if matches!(
            ch,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn scale_channel(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = mask >> shift;
    let value = (pixel & mask) >> shift;
    ((value as u64 * 255 + max as u64 / 2) / max as u64) as u8
}

fn encode_png(image: &RawImage) -> Result<Vec<u8>> {
    crate::core::image::encode_png(image)
}

fn x11_error(display: u16, error: impl std::fmt::Display) -> Error {
    Error::Display {
        display,
        reason: error.to_string(),
    }
}

/// True once an EWMH-compliant window manager has announced itself on the
/// display. Used to avoid returning from `create` before the WM is ready to
/// manage client windows.
pub fn window_manager_ready(display: u16) -> bool {
    let Ok((conn, screen_num)) = x11rb::connect(Some(&format!(":{display}"))) else {
        return false;
    };
    let root = conn.setup().roots[screen_num].root;
    let Ok(atom) = conn.intern_atom(false, b"_NET_SUPPORTING_WM_CHECK") else {
        return false;
    };
    let Ok(atom) = atom.reply() else {
        return false;
    };
    let Ok(property) = conn.get_property(
        false,
        root,
        atom.atom,
        x11rb::protocol::xproto::AtomEnum::WINDOW,
        0,
        1,
    ) else {
        return false;
    };
    let Ok(property) = property.reply() else {
        return false;
    };
    property
        .value32()
        .is_some_and(|mut values| values.next().is_some())
}

/// Focus a top-level window on the *host* display. Used by `runspace attach`.
pub fn focus_host_window(host_display: &str, title: &str) -> Result<()> {
    process::require_tool("xdotool", process::X11_INSTALL)?;
    let pattern = format!("^{}$", escape_regex(title));
    let output = Command::new("xdotool")
        .args(["search", "--name", &pattern])
        .env("DISPLAY", host_display)
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .map_err(|source| Error::Spawn {
            program: "xdotool".into(),
            source,
        })?;
    if !output.status.success() {
        return Err(Error::invalid(format!(
            "could not find the Runspace window on {host_display}"
        )));
    }
    let id = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<u64>().ok())
        .ok_or_else(|| {
            Error::invalid(format!(
                "could not find the Runspace window on {host_display}"
            ))
        })?;

    let window_id = id.to_string();
    let output = Command::new("xdotool")
        .args(["windowactivate", "--sync", &window_id])
        .env("DISPLAY", host_display)
        .output()
        .map_err(|source| Error::Spawn {
            program: "xdotool".into(),
            source,
        })?;
    if !output.status.success() {
        let fallback = Command::new("xdotool")
            .args(["windowfocus", &window_id])
            .env("DISPLAY", host_display)
            .output()
            .map_err(|source| Error::Spawn {
                program: "xdotool".into(),
                source,
            })?;
        if !fallback.status.success() {
            return Err(Error::command_failed("xdotool", &fallback));
        }
    }
    let _ = Command::new("xdotool")
        .args(["windowraise", &window_id])
        .env("DISPLAY", host_display)
        .status();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resolution_limits() {
        assert!(validate_resolution(1280, 800).is_ok());
        assert!(validate_resolution(319, 800).is_err());
        assert!(validate_resolution(1280, 239).is_err());
        assert!(validate_resolution(7681, 800).is_err());
    }

    #[test]
    fn escapes_regex_metacharacters() {
        assert_eq!(escape_regex("a.b(c)"), "a\\.b\\(c\\)");
        assert_eq!(escape_regex("plain title"), "plain title");
    }

    #[test]
    fn parses_xdotool_shell_output() {
        let map = parse_shell("WINDOW=123\nX=4\nY=5\nWIDTH=800\nHEIGHT=600\n");
        assert_eq!(map.get("WINDOW").unwrap(), "123");
        assert_eq!(map.get("WIDTH").unwrap(), "800");
    }

    #[test]
    fn rejects_bad_key_combos() {
        assert!(validate_key_combo("ctrl+s").is_ok());
        assert!(validate_key_combo("alt+F4").is_ok());
        assert!(validate_key_combo("").is_err());
        assert!(validate_key_combo("ctrl+s;rm -rf /").is_err());
    }

    #[test]
    fn scales_colour_channels() {
        assert_eq!(scale_channel(0x00ff_0000, 0x00ff_0000), 255);
        assert_eq!(scale_channel(0, 0x00ff_0000), 0);
        assert_eq!(scale_channel(0x1f, 0x3f), 125);
    }
}
