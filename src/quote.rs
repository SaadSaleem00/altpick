//! Quoting a picked name so the shell reads it as one argument.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStyle {
    None,
    PowerShell,
    Cmd,
    Sh,
}

impl QuoteStyle {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "ps" | "powershell" | "pwsh" => Some(Self::PowerShell),
            "cmd" => Some(Self::Cmd),
            "sh" | "bash" => Some(Self::Sh),
            _ => None,
        }
    }

    pub fn apply(self, name: &str) -> String {
        match self {
            Self::None => name.to_string(),
            Self::PowerShell => powershell(name),
            Self::Cmd => cmd(name),
            Self::Sh => sh(name),
        }
    }
}

/// Characters that never need quoting in any of the shells we support.
fn is_plain(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '~' | '/' | '\\' | ':')
}

fn needs_quotes(name: &str) -> bool {
    name.is_empty() || !name.chars().all(is_plain)
}

/// PowerShell single quotes are literal. Any single-quote character inside
/// (including the curly ones PowerShell also treats as quotes) is doubled.
fn powershell(name: &str) -> String {
    if !needs_quotes(name) {
        return name.to_string();
    }
    let mut out = String::with_capacity(name.len() + 2);
    out.push('\'');
    for c in name.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// CMD: wrap in double quotes. Windows file names can't contain `"`.
fn cmd(name: &str) -> String {
    if needs_quotes(name) {
        format!("\"{name}\"")
    } else {
        name.to_string()
    }
}

/// POSIX shells (Git Bash): single quotes, with ' written as '\''.
fn sh(name: &str) -> String {
    if needs_quotes(name) {
        format!("'{}'", name.replace('\'', r"'\''"))
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_stay_plain() {
        for style in [QuoteStyle::PowerShell, QuoteStyle::Cmd, QuoteStyle::Sh] {
            assert_eq!(style.apply("app.js"), "app.js");
            assert_eq!(style.apply("my-app_v2.test.js"), "my-app_v2.test.js");
            assert_eq!(style.apply("رپورٹ.txt"), "رپورٹ.txt");
        }
    }

    #[test]
    fn powershell_quoting() {
        let ps = QuoteStyle::PowerShell;
        assert_eq!(ps.apply("my file.js"), "'my file.js'");
        assert_eq!(ps.apply("it's.js"), "'it''s.js'");
        assert_eq!(ps.apply("a\u{2019}b.js"), "'a\u{2019}\u{2019}b.js'");
        assert_eq!(ps.apply("$env.js"), "'$env.js'");
        assert_eq!(ps.apply("a&b.js"), "'a&b.js'");
        assert_eq!(ps.apply("(1).js"), "'(1).js'");
    }

    #[test]
    fn cmd_and_sh_quoting() {
        assert_eq!(QuoteStyle::Cmd.apply("my file.js"), "\"my file.js\"");
        assert_eq!(QuoteStyle::Cmd.apply("a&b.js"), "\"a&b.js\"");
        assert_eq!(QuoteStyle::Sh.apply("it's.js"), r"'it'\''s.js'");
    }

    #[test]
    fn parse_names() {
        assert_eq!(QuoteStyle::parse("PS"), Some(QuoteStyle::PowerShell));
        assert_eq!(QuoteStyle::parse("bash"), Some(QuoteStyle::Sh));
        assert_eq!(QuoteStyle::parse("zsh"), None);
    }
}
