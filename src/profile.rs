//! Adding and removing filepick's one marked line in a shell startup file
//! (PowerShell profile, .bashrc), without disturbing anything else in it.
//!
//! The file's encoding and line endings are kept exactly. Windows PowerShell
//! 5.1 reads a profile without a byte-order mark in the old ANSI code page, so
//! files that aren't valid UTF-8 are edited byte for byte.

/// Every line filepick adds ends with this, so it can be found again.
pub const MARKER: &str = "# filepick-setup";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    /// UTF-8 with a byte-order mark.
    Utf8Bom,
    /// UTF-16 little-endian with a byte-order mark ("Unicode" in Notepad).
    Utf16Le,
    /// No mark, valid UTF-8 (plain ASCII files land here too).
    Utf8,
    /// No mark and not UTF-8: some ANSI code page. Kept byte for byte.
    Bytes,
}

struct Text {
    text: String,
    encoding: Encoding,
}

fn decode(bytes: &[u8]) -> Result<Text, String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let text =
            String::from_utf8(rest.to_vec()).map_err(|_| "the file says UTF-8 but isn't".to_string())?;
        return Ok(Text { text, encoding: Encoding::Utf8Bom });
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        if rest.len() % 2 != 0 {
            return Err("the file says UTF-16 but has an odd length".into());
        }
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
        let text = String::from_utf16(&units).map_err(|_| "the file says UTF-16 but isn't".to_string())?;
        return Ok(Text { text, encoding: Encoding::Utf16Le });
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Err("the file is UTF-16 big-endian, which filepick doesn't edit".into());
    }
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => Ok(Text { text, encoding: Encoding::Utf8 }),
        // Each byte becomes the char with the same number, so writing it back
        // gives the same bytes.
        Err(_) => Ok(Text { text: bytes.iter().map(|&b| b as char).collect(), encoding: Encoding::Bytes }),
    }
}

fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        Encoding::Utf16Le => {
            let mut out = vec![0xFF, 0xFE];
            out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            out
        }
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Bytes => text.chars().map(|c| c as u8).collect(),
    }
}

/// What the file should use between lines: whatever it already uses.
fn newline_of(text: &str, default: &'static str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\n') {
        "\n"
    } else {
        default
    }
}

/// The new file contents with `line` present exactly once, or `None` if the
/// file already has it. `line` must contain [`MARKER`]. An existing marked
/// line that differs (say, an older install path) is replaced in place.
///
/// `existing` is `None` for a file that doesn't exist yet. New files are
/// UTF-8 with a byte-order mark (which every PowerShell reads correctly)
/// when `bom_for_new` is set, plain UTF-8 otherwise (for .bashrc).
pub fn add_line(
    existing: Option<&[u8]>,
    line: &str,
    newline_for_new: &'static str,
    bom_for_new: bool,
) -> Result<Option<Vec<u8>>, String> {
    debug_assert!(line.contains(MARKER));
    let Some(bytes) = existing else {
        let encoding = if bom_for_new { Encoding::Utf8Bom } else { Encoding::Utf8 };
        return Ok(Some(encode(&format!("{line}{newline_for_new}"), encoding)));
    };
    let Text { text, mut encoding } = decode(bytes)?;
    let nl = newline_of(&text, newline_for_new);

    if !line.is_ascii() {
        match encoding {
            Encoding::Utf8Bom | Encoding::Utf16Le => {}
            // ASCII looks the same in UTF-8, so adding the mark changes nothing else.
            Encoding::Utf8 if text.is_ascii() && bom_for_new => encoding = Encoding::Utf8Bom,
            Encoding::Utf8 if !bom_for_new => {}
            _ => {
                return Err("the file has no text-encoding mark, so a line with non-English \
                            letters can't be added safely"
                    .into())
            }
        }
    }

    let mut out = String::with_capacity(text.len() + line.len() + 2);
    let mut found = false;
    let mut changed = false;
    for piece in text.split_inclusive('\n') {
        let (content, ending) = split_ending(piece);
        if content.contains(MARKER) {
            if found {
                changed = true; // a duplicate: drop it
                continue;
            }
            found = true;
            if content != line {
                changed = true;
                out.push_str(line);
                out.push_str(ending);
                continue;
            }
        }
        out.push_str(piece);
    }
    if !found {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(nl);
        }
        out.push_str(line);
        out.push_str(nl);
        changed = true;
    }
    Ok(changed.then(|| encode(&out, encoding)))
}

/// The file contents without any marked line, or `None` if there was none.
pub fn remove_lines(bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let Text { text, encoding } = decode(bytes)?;
    let mut out = String::with_capacity(text.len());
    let mut removed = false;
    for piece in text.split_inclusive('\n') {
        if split_ending(piece).0.contains(MARKER) {
            removed = true;
        } else {
            out.push_str(piece);
        }
    }
    Ok(removed.then(|| encode(&out, encoding)))
}

/// Whether the file holds nothing but whitespace (and maybe a byte-order mark).
pub fn is_blank(bytes: &[u8]) -> bool {
    decode(bytes).map(|t| t.text.trim().is_empty()).unwrap_or(false)
}

/// Whether the file has a marked line.
pub fn has_line(bytes: &[u8]) -> bool {
    decode(bytes).map(|t| t.text.lines().any(|l| l.contains(MARKER))).unwrap_or(false)
}

fn split_ending(piece: &str) -> (&str, &str) {
    if let Some(c) = piece.strip_suffix("\r\n") {
        (c, "\r\n")
    } else if let Some(c) = piece.strip_suffix('\n') {
        (c, "\n")
    } else {
        (piece, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "if (Test-Path x) { . x }  # filepick-setup";

    fn add(existing: &[u8]) -> Option<Vec<u8>> {
        add_line(Some(existing), LINE, "\r\n", true).unwrap()
    }

    #[test]
    fn new_file_gets_the_line_with_a_bom() {
        let out = add_line(None, LINE, "\r\n", true).unwrap().unwrap();
        assert_eq!(out, [&[0xEF, 0xBB, 0xBF][..], format!("{LINE}\r\n").as_bytes()].concat());
        let out = add_line(None, LINE, "\n", false).unwrap().unwrap();
        assert_eq!(out, format!("{LINE}\n").as_bytes());
    }

    #[test]
    fn appends_keeping_line_endings_and_adds_nothing_twice() {
        let once = add(b"Import-Module PSReadLine\r\n").unwrap();
        assert_eq!(once, format!("Import-Module PSReadLine\r\n{LINE}\r\n").as_bytes());
        assert_eq!(add(&once), None);

        // No newline at the end, LF file.
        let lf = add(b"a\nb").unwrap();
        assert_eq!(lf, format!("a\nb\n{LINE}\n").as_bytes());
    }

    #[test]
    fn replaces_an_older_line_in_place_and_drops_duplicates() {
        let old = "a\r\nold  # filepick-setup\r\nb\r\nagain # filepick-setup\r\n";
        let out = add(old.as_bytes()).unwrap();
        assert_eq!(out, format!("a\r\n{LINE}\r\nb\r\n").as_bytes());
    }

    #[test]
    fn remove_takes_out_only_marked_lines() {
        let text = format!("a\r\n{LINE}\r\nb\r\n");
        assert_eq!(remove_lines(text.as_bytes()).unwrap().unwrap(), b"a\r\nb\r\n");
        assert_eq!(remove_lines(b"a\r\nb\r\n").unwrap(), None);
        // Added then removed: back to the exact original.
        let original = b"Import-Module PSReadLine\r\n";
        assert_eq!(remove_lines(&add(original).unwrap()).unwrap().unwrap(), original);
    }

    #[test]
    fn keeps_utf16_and_bom_files_as_they_were() {
        let utf16: Vec<u8> =
            [0xFF, 0xFE].into_iter().chain("é\r\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let out = add(&utf16).unwrap();
        assert_eq!(out, encode(&format!("é\r\n{LINE}\r\n"), Encoding::Utf16Le));
        assert_eq!(remove_lines(&out).unwrap().unwrap(), utf16);

        let bom = [&[0xEF, 0xBB, 0xBF][..], "x\n".as_bytes()].concat();
        assert!(add(&bom).unwrap().starts_with(&[0xEF, 0xBB, 0xBF]));
    }

    #[test]
    fn ansi_bytes_survive_unchanged() {
        // "café" in Windows-1252: not valid UTF-8.
        let ansi = b"# caf\xe9\r\n".to_vec();
        let out = add(&ansi).unwrap();
        assert_eq!(out, [&ansi[..], format!("{LINE}\r\n").as_bytes()].concat());
        assert_eq!(remove_lines(&out).unwrap().unwrap(), ansi);
    }

    #[test]
    fn non_english_line_needs_a_safe_encoding() {
        let line = "if (Test-Path 'C:\\Users\\ساره\\f.ps1') { }  # filepick-setup";
        // Plain ASCII file: gains a BOM, otherwise unchanged.
        let out = add_line(Some(b"a\r\n"), line, "\r\n", true).unwrap().unwrap();
        assert_eq!(out, [&[0xEF, 0xBB, 0xBF][..], format!("a\r\n{line}\r\n").as_bytes()].concat());
        // ANSI file with its own non-ASCII text: refused.
        assert!(add_line(Some(b"caf\xe9\r\n"), line, "\r\n", true).is_err());
        // .bashrc (UTF-8 without a mark is normal there): fine.
        assert!(add_line(Some("é\n".as_bytes()), line, "\n", false).is_ok());
    }

    #[test]
    fn has_line_finds_the_marker() {
        assert!(has_line(format!("a\n{LINE}\n").as_bytes()));
        assert!(!has_line(b"a\n"));
    }
}
