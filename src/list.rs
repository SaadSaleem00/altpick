//! Reading and sorting the folder's entries.

use std::cmp::Ordering;
use std::io;
use std::path::Path;

use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// Lower-cased name, used for filtering.
    pub lower: String,
    /// Display width in terminal columns (includes the trailing `\` for folders).
    pub width: usize,
}

impl Entry {
    pub fn new(name: String, is_dir: bool) -> Self {
        let lower = name.to_lowercase();
        let width = name.width() + usize::from(is_dir);
        Entry { name, is_dir, lower, width }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub include_dirs: bool,
    pub include_hidden: bool,
    /// Lower-case extensions without the dot. Empty = show every file.
    pub extensions: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options { include_dirs: true, include_hidden: false, extensions: Vec::new() }
    }
}

pub fn read(dir: &Path, opts: &Options) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in std::fs::read_dir(dir)? {
        // A single unreadable entry shouldn't hide the whole folder.
        let Ok(item) = item else { continue };
        // Names that aren't valid Unicode can't be typed back safely; skip them.
        let Ok(name) = item.file_name().into_string() else { continue };

        let Ok(file_type) = item.file_type() else { continue };
        let is_dir = if file_type.is_symlink() { item.path().is_dir() } else { file_type.is_dir() };

        if !opts.include_hidden && is_hidden(&item, &name) {
            continue;
        }
        if is_dir {
            if !opts.include_dirs || !opts.extensions.is_empty() {
                continue;
            }
        } else if !opts.extensions.is_empty() && !has_extension(&name, &opts.extensions) {
            continue;
        }
        entries.push(Entry::new(name, is_dir));
    }
    sort(&mut entries);
    Ok(entries)
}

/// Files first, then folders; each group in natural order (file2 before file10).
pub fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| a.is_dir.cmp(&b.is_dir).then_with(|| natural_cmp(&a.name, &b.name)));
}

fn has_extension(name: &str, exts: &[String]) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => exts.iter().any(|e| e.eq_ignore_ascii_case(ext)),
        _ => false,
    }
}

#[cfg(windows)]
fn is_hidden(item: &std::fs::DirEntry, _name: &str) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    item.metadata()
        .map(|m| m.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0)
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_hidden(_item: &std::fs::DirEntry, name: &str) -> bool {
    name.starts_with('.')
}

/// Case-insensitive comparison that treats runs of digits as numbers.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let na = take_digits(&mut ai);
                let nb = take_digits(&mut bi);
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(ca), Some(cb)) => {
                let ord = ca.to_lowercase().cmp(cb.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

fn take_digits(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(&c) = it.peek() {
        if !c.is_ascii_digit() {
            break;
        }
        s.push(c);
        it.next();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn natural_order() {
        let mut v: Vec<Entry> = ["file10.js", "File2.js", "file1.js", "b.js", "A.js"]
            .iter()
            .map(|n| Entry::new(n.to_string(), false))
            .collect();
        sort(&mut v);
        assert_eq!(names(&v), ["A.js", "b.js", "file1.js", "File2.js", "file10.js"]);
    }

    #[test]
    fn files_before_folders() {
        let mut v = vec![
            Entry::new("src".into(), true),
            Entry::new("zeta.js".into(), false),
            Entry::new("app.js".into(), false),
        ];
        sort(&mut v);
        assert_eq!(names(&v), ["app.js", "zeta.js", "src"]);
    }

    #[test]
    fn extension_filter() {
        let exts = vec!["js".to_string(), "ts".to_string()];
        assert!(has_extension("app.JS", &exts));
        assert!(has_extension("a.b.ts", &exts));
        assert!(!has_extension("readme.md", &exts));
        assert!(!has_extension(".js", &exts));
        assert!(!has_extension("js", &exts));
    }

    #[test]
    fn reads_a_real_folder() {
        let dir = std::env::temp_dir().join(format!("altpick-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["b.js", "a.js", "notes.md", "my file.js"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let all = read(&dir, &Options::default()).unwrap();
        assert_eq!(names(&all), ["a.js", "b.js", "my file.js", "notes.md", "sub"]);
        assert!(all[4].is_dir);

        let js = read(&dir, &Options { extensions: vec!["js".into()], ..Options::default() }).unwrap();
        assert_eq!(names(&js), ["a.js", "b.js", "my file.js"]);

        let no_dirs = read(&dir, &Options { include_dirs: false, ..Options::default() }).unwrap();
        assert_eq!(no_dirs.len(), 4);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
