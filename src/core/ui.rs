use std::io::{self, IsTerminal};
use std::sync::OnceLock;

fn colour_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM")
                .map(|term| term != "dumb")
                .unwrap_or(true)
    })
}

fn wrap(code: &str, text: &str) -> String {
    if colour_enabled() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn bold(text: &str) -> String {
    wrap("1", text)
}

pub fn dim(text: &str) -> String {
    wrap("2", text)
}

pub fn green(text: &str) -> String {
    wrap("32", text)
}

pub fn yellow(text: &str) -> String {
    wrap("33", text)
}

pub fn red(text: &str) -> String {
    wrap("31", text)
}

pub fn cyan(text: &str) -> String {
    wrap("36", text)
}

pub fn duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else if seconds < 86_400 {
        format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
    } else {
        format!("{}d {:02}h", seconds / 86_400, (seconds % 86_400) / 3600)
    }
}

pub fn memory(bytes: u64) -> String {
    if bytes == 0 {
        return "-".into();
    }
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * MIB;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else if bytes >= 10.0 * MIB {
        format!("{:.0} MiB", bytes / MIB)
    } else {
        format!("{:.1} MiB", bytes / MIB)
    }
}

pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if index < widths.len() {
                widths[index] = widths[index].max(cell.chars().count());
            }
        }
    }

    let mut out = String::new();
    let header = headers
        .iter()
        .enumerate()
        .map(|(index, cell)| pad(cell, widths[index]))
        .collect::<Vec<_>>()
        .join("  ");
    out.push_str(&dim(header.trim_end()));
    out.push('\n');

    for row in rows {
        let line = row
            .iter()
            .enumerate()
            .map(|(index, cell)| pad(cell, widths.get(index).copied().unwrap_or(0)))
            .collect::<Vec<_>>()
            .join("  ");
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn pad(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - len))
    }
}

pub fn kv(pairs: &[(&str, String)]) -> String {
    let width = pairs.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    pairs
        .iter()
        .map(|(key, value)| format!("  {:<width$}  {value}", key, width = width))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(59), "59s");
        assert_eq!(duration(61), "1m 01s");
        assert_eq!(duration(3661), "1h 01m");
        assert_eq!(duration(90_061), "1d 01h");
    }

    #[test]
    fn formats_memory() {
        assert_eq!(memory(0), "-");
        assert_eq!(memory(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(memory(128 * 1024 * 1024), "128 MiB");
    }
}
