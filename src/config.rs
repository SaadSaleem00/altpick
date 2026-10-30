//! The settings file, `%APPDATA%\filepick\config.toml`.
//!
//! ```toml
//! hotkey = "Alt+f"            # PSReadLine notation; read by the shell hooks
//! extensions = ["js", "ts"]   # default for --ext
//! ```
//!
//! Only these two keys exist, so this reads the small part of TOML they need
//! (strings, string arrays, comments) instead of pulling in a TOML library.
//! A broken file never stops the palette: bad lines are ignored.

use std::path::PathBuf;

pub const DEFAULT_HOTKEY: &str = "Alt+f";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub hotkey: Option<String>,
    pub extensions: Vec<String>,
}

/// `%APPDATA%\filepick`, where the settings file and setup's notes live.
/// `FILEPICK_CONFIG_DIR` overrides it (the tests use that).
pub fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("FILEPICK_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("XDG_CONFIG_HOME"))?;
    Some(PathBuf::from(base).join("filepick"))
}

/// `%APPDATA%\filepick\config.toml`, or None if APPDATA isn't set.
pub fn path() -> Option<PathBuf> {
    Some(dir()?.join("config.toml"))
}

/// The hotkey in readline notation, for Clink and Git Bash: "Alt+f" becomes
/// `\ef`, "Ctrl+Alt+p" `\e\C-p`, "F2" `\eOQ`. None if it can't be expressed.
pub fn readline_key(hotkey: &str) -> Option<String> {
    let parts: Vec<&str> = hotkey.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last()?;
    let mut alt = false;
    let mut ctrl = false;
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "alt" | "meta" => alt = true,
            "ctrl" | "control" => ctrl = true,
            _ => return None,
        }
    }
    let lower = key.to_ascii_lowercase();
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<usize>().ok()) {
        const F: [&str; 12] =
            ["OP", "OQ", "OR", "OS", "[15~", "[17~", "[18~", "[19~", "[20~", "[21~", "[23~", "[24~"];
        if alt || ctrl || !(1..=12).contains(&n) {
            return None;
        }
        return Some(format!("\\e{}", F[n - 1]));
    }
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return None };
    if !c.is_ascii_graphic() || c == '"' || c == '\\' {
        return None;
    }
    let c = if ctrl { c.to_ascii_lowercase() } else { c };
    Some(match (alt, ctrl) {
        (true, true) => format!("\\e\\C-{c}"),
        (true, false) => format!("\\e{c}"),
        (false, true) => format!("\\C-{c}"),
        // A bare letter would stop you typing that letter.
        (false, false) => return None,
    })
}

/// Reads the settings file. A missing or unreadable file gives the defaults.
pub fn load() -> Config {
    path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default()
}

pub fn parse(text: &str) -> Config {
    let mut config = Config::default();
    for line in text.lines() {
        let line = strip_comment(line).trim();
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "hotkey" => config.hotkey = string(value.trim()).filter(|s| !s.is_empty()),
            "extensions" => {
                if let Some(list) = string_array(value.trim()) {
                    config.extensions = list
                        .into_iter()
                        .map(|e| e.trim().trim_start_matches('.').to_lowercase())
                        .filter(|e| !e.is_empty())
                        .collect();
                }
            }
            _ => {}
        }
    }
    config
}

/// Drops a `#` comment, unless the `#` is inside a quoted string.
fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '#') => return &line[..i],
            _ => {}
        }
    }
    line
}

/// `"text"` (with \" and \\ escapes) or `'text'` (literal).
fn string(value: &str) -> Option<String> {
    let (s, rest) = take_string(value)?;
    rest.trim().is_empty().then_some(s)
}

/// `["a", 'b']`
fn string_array(value: &str) -> Option<Vec<String>> {
    let mut rest = value.strip_prefix('[')?.trim_start();
    let mut items = Vec::new();
    loop {
        if let Some(after) = rest.strip_prefix(']') {
            return after.trim().is_empty().then_some(items);
        }
        let (item, after) = take_string(rest)?;
        items.push(item);
        rest = after.trim_start();
        if let Some(after) = rest.strip_prefix(',') {
            rest = after.trim_start();
        }
    }
}

/// Reads one quoted string from the start of `s`; returns it and what follows.
fn take_string(s: &str) -> Option<(String, &str)> {
    let mut chars = s.char_indices();
    let (_, quote) = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for (i, c) in chars {
        if quote == '"' && escaped {
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if quote == '"' && c == '\\' {
            escaped = true;
        } else if c == quote {
            return Some((out, &s[i + c.len_utf8()..]));
        } else {
            out.push(c);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_keys() {
        let c = parse(
            "# my settings\nhotkey = \"Alt+p\"   # comment\nextensions = [\"JS\", '.ts', \"\"]\nother = 1\n",
        );
        assert_eq!(c.hotkey.as_deref(), Some("Alt+p"));
        assert_eq!(c.extensions, ["js", "ts"]);
    }

    #[test]
    fn hash_inside_quotes_is_not_a_comment() {
        assert_eq!(parse("hotkey = \"Ctrl+#\"").hotkey.as_deref(), Some("Ctrl+#"));
    }

    #[test]
    fn broken_lines_are_ignored() {
        let c = parse("hotkey = Alt+p\nextensions = [\"js\"\nhotkey\n= \"x\"");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn hotkeys_in_readline_notation() {
        assert_eq!(readline_key("Alt+f").as_deref(), Some(r"\ef"));
        assert_eq!(readline_key("alt+F").as_deref(), Some(r"\eF"));
        assert_eq!(readline_key("Ctrl+Alt+P").as_deref(), Some(r"\e\C-p"));
        assert_eq!(readline_key("Ctrl+g").as_deref(), Some(r"\C-g"));
        assert_eq!(readline_key("F2").as_deref(), Some(r"\eOQ"));
        assert_eq!(readline_key("F12").as_deref(), Some(r"\e[24~"));
        for bad in ["f", "Shift+f", "F13", "Alt+F2", "Alt+", "Alt+\\", "Alt+ab", ""] {
            assert_eq!(readline_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn empty_file_gives_defaults() {
        assert_eq!(parse(""), Config::default());
    }
}
