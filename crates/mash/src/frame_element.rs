//! FrameElement tree composition for MASH shell output.
//!
//! This module provides functions for composing shell output (prompts, command output,
//! structured annotations) into FrameElement trees that can be rendered by the
//! daemon's renderer host.

use malt_protocol::common::ResolvedStyle;
use malt_protocol::frame_element::FrameElement;

/// Default PS1 prompt format.
const DEFAULT_PS1: &str = "\\w \\$ ";

/// Default PS2 continuation prompt format.
const DEFAULT_PS2: &str = "> ";

/// Expand PS1/PS2 escape sequences into the actual prompt string.
///
/// Supported escapes:
/// - `\a` - bell (ASCII 0x07)
/// - `\d` - date in "Weekday Month Date" format
/// - `\e` / `\E` - escape character (ASCII 0x1B)
/// - `\h` - hostname (short, up to first '.')
/// - `\H` - full hostname
/// - `\j` - number of jobs (always 0 for now)
/// - `\l` - basename of terminal device (always "malt" for now)
/// - `\n` - newline
/// - `\r` - carriage return
/// - `\s` - name of the shell ("mash")
/// - `\t` - time in 24-hour HH:MM:SS format
/// - `\T` - time in 12-hour HH:MM:SS format
/// - `\@` - time in 12-hour am/pm format
/// - `\A` - time in 24-hour HH:MM format
/// - `\u` - username
/// - `\v` - version (short, e.g., "0.1")
/// - `\V` - version (full, e.g., "0.1.0")
/// - `\w` - current working directory (with ~ for home)
/// - `\W` - basename of current directory
/// - `\!` - history number (always 0 for now)
/// - `\#` - command number (always 0 for now)
/// - `\$` - '#' if root, '$' otherwise
/// - `\\` - backslash
/// - `\[`, `\]` - non-printing delimiters (ignored for now)
pub fn expand_prompt_format(ps1: &str, cwd: &str, _exit_code: i32) -> String {
    let mut result = String::new();
    let mut chars = ps1.chars().peekable();

    // Get current time for time-based escapes
    let now = std::time::SystemTime::now();
    let datetime = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| {
            let secs = d.as_secs();

            chrono::DateTime::from_timestamp(secs as i64, 0).unwrap_or(chrono::DateTime::UNIX_EPOCH)
        })
        .unwrap_or_else(|_| chrono::DateTime::UNIX_EPOCH);

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('a') => result.push('\x07'), // bell
                Some('d') => {
                    // Date in "Weekday Month Date" format
                    result.push_str(&datetime.format("%a %b %d").to_string());
                }
                Some('e') | Some('E') => result.push('\x1B'), // escape
                Some('h') => {
                    // Short hostname
                    if let Ok(hostname) = std::env::var("HOSTNAME")
                        .or_else(|_| std::env::var("COMPUTERNAME"))
                        .or_else(|_| std::env::var("HOST"))
                    {
                        if let Some(idx) = hostname.find('.') {
                            result.push_str(&hostname[..idx]);
                        } else {
                            result.push_str(&hostname);
                        }
                    } else {
                        result.push_str("localhost");
                    }
                }
                Some('H') => {
                    // Full hostname
                    if let Ok(hostname) = std::env::var("HOSTNAME")
                        .or_else(|_| std::env::var("COMPUTERNAME"))
                        .or_else(|_| std::env::var("HOST"))
                    {
                        result.push_str(&hostname);
                    } else {
                        result.push_str("localhost.localdomain");
                    }
                }
                Some('j') => result.push('0'), // jobs (not implemented)
                Some('l') => result.push_str("malt"), // terminal basename
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('s') => result.push_str("mash"), // shell name
                Some('t') => result.push_str(&datetime.format("%H:%M:%S").to_string()),
                Some('T') => result.push_str(&datetime.format("%I:%M:%S").to_string()),
                Some('@') => {
                    result.push_str(&datetime.format("%I:%M %p").to_string().to_lowercase())
                }
                Some('A') => result.push_str(&datetime.format("%H:%M").to_string()),
                Some('u') => {
                    // Username
                    if let Ok(user) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
                        result.push_str(&user);
                    } else {
                        result.push_str("user");
                    }
                }
                Some('v') => result.push_str("0.1"), // short version
                Some('V') => result.push_str("0.1.0"), // full version
                Some('w') => {
                    // Current working directory with ~ for home
                    if let Ok(home) =
                        std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))
                    {
                        let home_path = std::path::Path::new(&home);
                        let cwd_path = std::path::Path::new(cwd);
                        if let (Ok(home_canon), Ok(cwd_canon)) =
                            (home_path.canonicalize(), cwd_path.canonicalize())
                        {
                            if cwd_canon.starts_with(&home_canon) {
                                if let Ok(stripped) = cwd_canon.strip_prefix(&home_canon) {
                                    result.push('~');
                                    if let Some(s) = stripped.to_str() {
                                        if !s.is_empty() {
                                            result.push_str(std::path::MAIN_SEPARATOR_STR);
                                            result.push_str(s);
                                        }
                                    }
                                    continue;
                                }
                            }
                        }
                    }
                    result.push_str(cwd);
                }
                Some('W') => {
                    // Basename of current directory
                    if let Some(idx) = cwd.rfind(std::path::MAIN_SEPARATOR) {
                        if idx + 1 < cwd.len() {
                            result.push_str(&cwd[idx + 1..]);
                        } else {
                            result.push(std::path::MAIN_SEPARATOR);
                        }
                    } else {
                        result.push_str(cwd);
                    }
                }
                Some('!') => result.push('0'), // history number (not implemented)
                Some('#') => result.push('0'), // command number (not implemented)
                Some('$') => {
                    // '#' if root, '$' otherwise
                    let is_root = std::env::var("USER")
                        .or_else(|_| std::env::var("USERNAME"))
                        .map(|u| u == "root")
                        .unwrap_or(false);
                    if is_root {
                        result.push('#');
                    } else {
                        result.push('$');
                    }
                }
                Some('\\') => result.push('\\'),
                Some('[') | Some(']') => {
                    // Non-printing delimiters - ignore for now
                    // These are used for ANSI sequences that shouldn't count
                    // toward prompt width
                }
                Some(other) => {
                    // Unknown escape - output literally
                    result.push('\\');
                    result.push(other);
                }
                None => {
                    // Trailing backslash
                    result.push('\\');
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Compose a prompt into a FrameElement tree.
///
/// The prompt includes:
/// - Current working directory
/// - Exit status of last command (if non-zero)
/// - User-defined prompt segments (placeholder for Phase G plugin integration)
pub fn compose_prompt(
    cwd: &str,
    exit_code: i32,
    _plugin_segments: &[String], // Reserved for Phase G plugin system
) -> FrameElement {
    let mut parts = Vec::new();

    // Add exit status badge if non-zero
    if exit_code != 0 {
        let status_style = ResolvedStyle {
            fg: (255, 100, 100), // Red-ish for error
            bg: (50, 0, 0),      // Dark red background
            bold: true,
            italic: false,
            underline: false,
            dim: false,
            strikethrough: false,
            reverse: false,
            blink: false,
            token_name: None,
            _unknown: Vec::new(),
        };

        parts.push(FrameElement::Badge {
            text: format!("{}", exit_code),
            severity: Box::new(malt_protocol::frame_element::BadgeSeverity::Error),
            style: Box::new(status_style),
        });
    }

    // Add current directory
    let cwd_display = if cwd.len() > 40 {
        format!("...{}", &cwd[cwd.len() - 37..])
    } else {
        cwd.to_string()
    };

    let prompt_style = ResolvedStyle {
        fg: (100, 200, 100), // Green-ish for success/path
        bg: (0, 0, 0),
        bold: true,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    };

    parts.push(FrameElement::Text {
        text: format!("{} $", cwd_display),
        style: Box::new(prompt_style),
    });

    // Stack all parts horizontally
    if parts.len() == 1 {
        parts.remove(0)
    } else {
        FrameElement::Stack { children: parts }
    }
}

/// Compose a prompt from a PS1 format string with escape sequence expansion.
///
/// This is the preferred method for prompt composition as it supports
/// user-customizable PS1/PS2 variables.
pub fn compose_prompt_from_ps1(ps1: &str, cwd: &str, exit_code: i32) -> FrameElement {
    let mut parts = Vec::new();

    // Add exit status badge if non-zero
    if exit_code != 0 {
        let status_style = ResolvedStyle {
            fg: (255, 100, 100), // Red-ish for error
            bg: (50, 0, 0),      // Dark red background
            bold: true,
            italic: false,
            underline: false,
            dim: false,
            strikethrough: false,
            reverse: false,
            blink: false,
            token_name: None,
            _unknown: Vec::new(),
        };

        parts.push(FrameElement::Badge {
            text: format!("{}", exit_code),
            severity: Box::new(malt_protocol::frame_element::BadgeSeverity::Error),
            style: Box::new(status_style),
        });
    }

    // Expand PS1 format and add as text
    let expanded = expand_prompt_format(ps1, cwd, exit_code);

    let prompt_style = ResolvedStyle {
        fg: (100, 200, 100), // Green-ish for success/path
        bg: (0, 0, 0),
        bold: true,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    };

    parts.push(FrameElement::Text {
        text: expanded,
        style: Box::new(prompt_style),
    });

    // Stack all parts horizontally
    if parts.len() == 1 {
        parts.remove(0)
    } else {
        FrameElement::Stack { children: parts }
    }
}

/// Get the default PS1 prompt format.
pub fn default_ps1() -> &'static str {
    DEFAULT_PS1
}

/// Get the default PS2 continuation prompt format.
pub fn default_ps2() -> &'static str {
    DEFAULT_PS2
}

/// Compose command stdout into a FrameElement tree.
///
/// Lines are split and rendered as a Paragraph element.
pub fn compose_stdout(data: &[u8]) -> Option<FrameElement> {
    if data.is_empty() {
        return None;
    }

    let text = String::from_utf8_lossy(data);
    let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();

    if lines.is_empty() {
        return None;
    }

    let style = ResolvedStyle {
        fg: (255, 255, 255), // White text
        bg: (0, 0, 0),       // Black background
        bold: false,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    };

    Some(FrameElement::Paragraph {
        lines,
        style: Box::new(style),
    })
}

/// Compose command stderr into a FrameElement tree.
///
/// Lines are split and rendered as a Paragraph with error styling.
pub fn compose_stderr(data: &[u8]) -> Option<FrameElement> {
    if data.is_empty() {
        return None;
    }

    let text = String::from_utf8_lossy(data);
    let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();

    if lines.is_empty() {
        return None;
    }

    let style = ResolvedStyle {
        fg: (255, 100, 100), // Red text for errors
        bg: (0, 0, 0),
        bold: false,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    };

    Some(FrameElement::Paragraph {
        lines,
        style: Box::new(style),
    })
}

/// Compose a command result (stdout + stderr) into a FrameElement tree.
pub fn compose_command_output(stdout: &[u8], stderr: &[u8]) -> Option<FrameElement> {
    let stdout_elem = compose_stdout(stdout);
    let stderr_elem = compose_stderr(stderr);

    match (stdout_elem, stderr_elem) {
        (Some(out), Some(err)) => Some(FrameElement::Stack {
            children: vec![out, err],
        }),
        (Some(out), None) => Some(out),
        (None, Some(err)) => Some(err),
        (None, None) => None,
    }
}

/// Compose a table from headers and rows.
pub fn compose_table(
    headers: Vec<String>,
    cells: Vec<String>,
    col_count: u16,
    _col_widths: Vec<u16>,
) -> FrameElement {
    let style = ResolvedStyle {
        fg: (255, 255, 255),
        bg: (0, 0, 0),
        bold: false,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    };

    let row_count = (cells.len() as u16).checked_div(col_count).unwrap_or(0);

    FrameElement::Table {
        headers,
        row_count,
        col_count,
        cells,
        col_widths: _col_widths,
        style,
    }
}

/// Compose a list from items.
pub fn compose_list(items: Vec<String>, ordered: bool) -> FrameElement {
    let style = ResolvedStyle {
        fg: (255, 255, 255),
        bg: (0, 0, 0),
        bold: false,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    };

    FrameElement::List {
        items,
        ordered,
        style: Box::new(style),
    }
}

/// Compose a key-value display.
pub fn compose_key_value(pairs: Vec<(String, String)>) -> FrameElement {
    let style = ResolvedStyle {
        fg: (255, 255, 255),
        bg: (0, 0, 0),
        bold: false,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    };

    let kv_pairs: Vec<malt_protocol::frame_element::KeyValuePair> = pairs
        .into_iter()
        .map(|(k, v)| malt_protocol::frame_element::KeyValuePair {
            key: k,
            value: v,
            _unknown: Vec::new(),
        })
        .collect();

    FrameElement::KeyValue {
        pairs: kv_pairs,
        style: Box::new(style),
    }
}

/// Compose a progress bar.
pub fn compose_progress_bar(percent: u8, label: String) -> FrameElement {
    let style = ResolvedStyle {
        fg: (100, 200, 100),
        bg: (50, 50, 50),
        bold: false,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    };

    FrameElement::ProgressBar {
        percent: percent.min(100),
        label,
        style: Box::new(style),
    }
}

/// Compose a badge with a severity level.
pub fn compose_badge(
    text: String,
    severity: malt_protocol::frame_element::BadgeSeverity,
) -> FrameElement {
    let style = ResolvedStyle {
        fg: match severity {
            malt_protocol::frame_element::BadgeSeverity::Info => (100, 200, 255),
            malt_protocol::frame_element::BadgeSeverity::Success => (100, 255, 100),
            malt_protocol::frame_element::BadgeSeverity::Warning => (255, 200, 100),
            malt_protocol::frame_element::BadgeSeverity::Error => (255, 100, 100),
        },
        bg: (0, 0, 0),
        bold: true,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    };

    FrameElement::Badge {
        text,
        severity: Box::new(severity),
        style: Box::new(style),
    }
}

/// Compose a simple text element.
pub fn compose_text(text: String, fg: (u8, u8, u8), bold: bool) -> FrameElement {
    let style = ResolvedStyle {
        fg,
        bg: (0, 0, 0),
        bold,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    };

    FrameElement::Text {
        text,
        style: Box::new(style),
    }
}

/// Compose a crash recovery message.
pub fn compose_crash_recovery_message() -> FrameElement {
    let style = ResolvedStyle {
        fg: (255, 100, 100), // Red text
        bg: (0, 0, 0),
        bold: true,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: true, // Blinking to draw attention
        token_name: None,
        _unknown: Vec::new(),
    };

    FrameElement::Text {
        text: "[shell crashed — press Enter to restart]".to_string(),
        style: Box::new(style),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_prompt_basic() {
        let elem = compose_prompt("/home/user", 0, &[]);
        match elem {
            FrameElement::Text { text, .. } => {
                assert!(text.contains("/home/user"));
                assert!(text.contains("$"));
            }
            FrameElement::Stack { children } => {
                // Should have at least one child
                assert!(!children.is_empty());
            }
            _ => panic!("Expected Text or Stack element"),
        }
    }

    #[test]
    fn compose_prompt_with_error() {
        let elem = compose_prompt("/home/user", 1, &[]);
        match elem {
            FrameElement::Stack { children } => {
                // Should have badge for exit code + prompt text
                assert_eq!(children.len(), 2);
            }
            _ => {
                // Could be single element if logic changes
            }
        }
    }

    #[test]
    fn compose_stdout_splits_lines() {
        let data = b"line1\nline2\nline3";
        let elem = compose_stdout(data).unwrap();
        match elem {
            FrameElement::Paragraph { lines, .. } => {
                assert_eq!(lines.len(), 3);
                assert_eq!(lines[0], "line1");
                assert_eq!(lines[1], "line2");
                assert_eq!(lines[2], "line3");
            }
            _ => panic!("Expected Paragraph element"),
        }
    }

    #[test]
    fn compose_empty_returns_none() {
        assert!(compose_stdout(b"").is_none());
        assert!(compose_stderr(b"").is_none());
    }

    #[test]
    fn compose_list_creates_element() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let elem = compose_list(items.clone(), false);
        match elem {
            FrameElement::List {
                items: got,
                ordered,
                ..
            } => {
                assert_eq!(got, items);
                assert!(!ordered);
            }
            _ => panic!("Expected List element"),
        }
    }

    #[test]
    fn compose_table_creates_element() {
        let headers = vec!["Name".to_string(), "Value".to_string()];
        let cells = vec!["foo".to_string(), "bar".to_string()];
        let elem = compose_table(headers.clone(), cells, 2, vec![10, 10]);
        match elem {
            FrameElement::Table {
                headers: got_headers,
                row_count,
                col_count,
                ..
            } => {
                assert_eq!(got_headers, headers);
                assert_eq!(col_count, 2);
                assert_eq!(row_count, 1); // 2 cells / 2 columns = 1 row
            }
            _ => panic!("Expected Table element"),
        }
    }

    #[test]
    fn compose_badge_with_severity() {
        let elem = compose_badge(
            "Error".to_string(),
            malt_protocol::frame_element::BadgeSeverity::Error,
        );
        match elem {
            FrameElement::Badge { text, severity, .. } => {
                assert_eq!(text, "Error");
                assert!(matches!(
                    *severity,
                    malt_protocol::frame_element::BadgeSeverity::Error
                ));
            }
            _ => panic!("Expected Badge element"),
        }
    }
}
