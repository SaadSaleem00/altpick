//! The palette: layout, drawing and keys.

use std::io::{self, Write};
use std::path::{Path, MAIN_SEPARATOR};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::queue;
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::filter;
use crate::list::Entry;
use crate::term::Session;

/// Widest a column gets before long names are shortened with "…".
const MAX_CELL: usize = 30;
/// Columns are never narrowed below this just because most names are short.
const MIN_CELL: usize = 12;
/// One space either side of each name.
const PAD: usize = 2;
/// Rows used by the header, filter line, divider and help line.
const CHROME_ROWS: usize = 4;
const GRID_TOP: usize = 3;
const HELP: &str = "↑↓←→ move · Enter pick · Esc cancel · type to filter";
const FONT_HINT: &str = "Some names can't be shown in this window — Windows Terminal can show them.";

/// Shows the palette and returns the picked entry, or `None` if cancelled.
pub fn run(dir: &Path, entries: Vec<Entry>) -> io::Result<Option<Entry>> {
    let dir_text = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf()).display().to_string();
    let mut session = Session::enter()?;
    let mut state = State::new(entries);
    state.font_hint =
        crate::term::classic_console() && state.entries.iter().any(|e| !console_can_show(&e.name));
    let mut test_keys = test_keys();

    loop {
        let (w, h) = terminal::size()?;
        state.fit(w as usize, h as usize);
        draw(&mut session.out, &state, &dir_text, w as usize, h as usize)?;

        let event = match test_keys.next() {
            Some(key) => {
                std::thread::sleep(std::time::Duration::from_millis(300));
                Event::Key(key)
            }
            None => event::read()?,
        };
        // Resize and other events just fall through to a redraw.
        if let Event::Key(key) = event {
            // Windows reports key releases too; only act on presses.
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match state.on_key(key) {
                Action::Pick => return Ok(state.current().cloned()),
                Action::Cancel => return Ok(None),
                Action::Redraw => {}
            }
        }
    }
}

/// For tests in terminals that can't be sent keys (like mintty):
/// ALTPICK_TEST_KEYS="text,enter" types "text" and then presses Enter.
/// Words: enter, esc, end, home, down, right; anything else is typed.
fn test_keys() -> std::vec::IntoIter<KeyEvent> {
    let spec = std::env::var("ALTPICK_TEST_KEYS").unwrap_or_default();
    let mut keys = Vec::new();
    for word in spec.split(',').filter(|w| !w.is_empty()) {
        let code = match word {
            "enter" => KeyCode::Enter,
            "esc" => KeyCode::Esc,
            "end" => KeyCode::End,
            "home" => KeyCode::Home,
            "down" => KeyCode::Down,
            "right" => KeyCode::Right,
            text => {
                keys.extend(text.chars().map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
                continue;
            }
        };
        keys.push(KeyEvent::new(code, KeyModifiers::NONE));
    }
    keys.into_iter()
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Pick,
    Cancel,
    Redraw,
}

struct State {
    entries: Vec<Entry>,
    query: String,
    /// Indexes into `entries` that match the query, in display order.
    shown: Vec<usize>,
    /// Position in `shown`.
    sel: usize,
    /// First grid row on screen.
    scroll: usize,
    cols: usize,
    visible_rows: usize,
    cell: usize,
    widest: usize,
    /// Show FONT_HINT above the help line.
    font_hint: bool,
}

impl State {
    fn new(entries: Vec<Entry>) -> State {
        let widest = column_width(&entries);
        let shown = (0..entries.len()).collect();
        State {
            entries,
            query: String::new(),
            shown,
            sel: 0,
            scroll: 0,
            cols: 1,
            visible_rows: 1,
            cell: widest,
            widest,
            font_hint: false,
        }
    }

    fn current(&self) -> Option<&Entry> {
        self.shown.get(self.sel).map(|&i| &self.entries[i])
    }

    fn total_rows(&self) -> usize {
        self.shown.len().div_ceil(self.cols)
    }

    /// Works out columns and rows for a `w` x `h` window and keeps the
    /// selection on screen.
    fn fit(&mut self, w: usize, h: usize) {
        // Leave the last column empty: writing into it makes some consoles wrap.
        let usable = w.saturating_sub(1);
        self.cell = self.widest.min(usable.saturating_sub(PAD)).max(1);
        self.cols = (usable / (self.cell + PAD)).max(1);
        self.visible_rows = h.saturating_sub(self.chrome_rows()).max(1);
        self.keep_visible();
    }

    fn chrome_rows(&self) -> usize {
        CHROME_ROWS + usize::from(self.font_hint)
    }

    fn keep_visible(&mut self) {
        if self.shown.is_empty() {
            self.sel = 0;
            self.scroll = 0;
            return;
        }
        self.sel = self.sel.min(self.shown.len() - 1);
        let row = self.sel / self.cols;
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + self.visible_rows {
            self.scroll = row + 1 - self.visible_rows;
        }
        let max_scroll = self.total_rows().saturating_sub(self.visible_rows);
        self.scroll = self.scroll.min(max_scroll);
    }

    fn refilter(&mut self) {
        self.shown = filter::apply(&self.entries, &self.query);
        self.sel = 0;
        self.scroll = 0;
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let n = self.shown.len();
        let cols = self.cols;

        match key.code {
            KeyCode::Esc => return Action::Cancel,
            KeyCode::Char('c' | 'g') if ctrl && !alt => return Action::Cancel,
            KeyCode::Enter => {
                if n > 0 {
                    return Action::Pick;
                }
            }
            KeyCode::Char('u') if ctrl && !alt => {
                self.query.clear();
                self.refilter();
            }
            KeyCode::Backspace => {
                if self.query.pop().is_some() {
                    self.refilter();
                }
            }
            KeyCode::Up => {
                if self.sel >= cols {
                    self.sel -= cols;
                }
            }
            KeyCode::Down => {
                if self.sel + cols < n {
                    self.sel += cols;
                } else if n > 0 && self.sel / cols < (n - 1) / cols {
                    // Row below is shorter: land on its last item.
                    self.sel = n - 1;
                }
            }
            KeyCode::Left | KeyCode::BackTab => self.sel = self.sel.saturating_sub(1),
            KeyCode::Right | KeyCode::Tab => {
                if self.sel + 1 < n {
                    self.sel += 1;
                }
            }
            KeyCode::Home => self.sel = 0,
            KeyCode::End => self.sel = n.saturating_sub(1),
            KeyCode::PageUp => self.sel = self.sel.saturating_sub(cols * self.visible_rows),
            KeyCode::PageDown => self.sel = (self.sel + cols * self.visible_rows).min(n.saturating_sub(1)),
            // Plain typing. Ctrl+Alt together is AltGr on many keyboards (@, €, \ ...).
            KeyCode::Char(c) if (!ctrl && !alt) || (ctrl && alt) => {
                self.query.push(c);
                self.refilter();
            }
            _ => {}
        }
        self.keep_visible();
        Action::Redraw
    }
}

/// Column width that fits 90% of names in full, so one very long name
/// doesn't squeeze the whole grid into a single column.
fn column_width(entries: &[Entry]) -> usize {
    let mut widths: Vec<usize> = entries.iter().map(|e| e.width).collect();
    if widths.is_empty() {
        return 1;
    }
    widths.sort_unstable();
    let widest = widths[widths.len() - 1];
    let p90 = widths[(widths.len() * 9).div_ceil(10) - 1];
    p90.clamp(widest.min(MIN_CELL), MAX_CELL).max(1)
}

/// Whether the classic console's fonts (Consolas, Lucida Console, raster)
/// can draw every letter of `name`. They cover Latin, Greek and Cyrillic,
/// plus punctuation and symbols; not Arabic, Indic or East Asian scripts,
/// and nothing outside the Basic Multilingual Plane (emoji).
fn console_can_show(name: &str) -> bool {
    name.chars().all(|c| matches!(c as u32, 0..=0x058F | 0x1D00..=0x1FFF | 0x2000..=0x2BFF | 0xFB00..=0xFB06))
}

fn label(e: &Entry) -> String {
    if e.is_dir {
        format!("{}{MAIN_SEPARATOR}", e.name)
    } else {
        e.name.clone()
    }
}

fn draw(out: &mut impl Write, s: &State, dir_text: &str, w: usize, h: usize) -> io::Result<()> {
    let usable = w.saturating_sub(1);
    queue!(out, Hide)?;

    if h < s.chrome_rows() + 1 || usable < 16 {
        queue!(out, MoveTo(0, 0), Clear(ClearType::All), Print(fit("Window too small", usable)))?;
        return out.flush();
    }

    // Header: name + folder.
    let title = " AltPick ";
    queue!(
        out,
        MoveTo(0, 0),
        SetAttribute(Attribute::Reverse),
        Print(title),
        SetAttribute(Attribute::Reset),
        Print(" "),
        SetForegroundColor(Color::DarkGrey),
        Print(fit_left(dir_text, usable.saturating_sub(title.len() + 1))),
        ResetColor,
        Clear(ClearType::UntilNewLine),
    )?;

    // Filter line.
    let query_text = fit_left(&s.query, usable.saturating_sub(2));
    queue!(
        out,
        MoveTo(0, 1),
        SetForegroundColor(Color::Cyan),
        Print("> "),
        ResetColor,
        Print(&query_text),
        Clear(ClearType::UntilNewLine),
        MoveTo(0, 2),
        SetForegroundColor(Color::DarkGrey),
        Print("─".repeat(usable)),
        ResetColor,
        Clear(ClearType::UntilNewLine),
    )?;

    // Grid.
    for r in 0..s.visible_rows {
        queue!(out, MoveTo(0, (GRID_TOP + r) as u16))?;
        if s.shown.is_empty() && r == 0 {
            let msg = if s.entries.is_empty() { "  Nothing to show in this folder" } else { "  No matches" };
            queue!(out, SetForegroundColor(Color::DarkGrey), Print(fit(msg, usable)), ResetColor)?;
        }
        let row = s.scroll + r;
        for c in 0..s.cols {
            let pos = row * s.cols + c;
            let Some(&idx) = s.shown.get(pos) else { break };
            draw_cell(out, &s.entries[idx], s.cell, pos == s.sel)?;
        }
        queue!(out, Clear(ClearType::UntilNewLine))?;
    }

    if s.font_hint {
        queue!(
            out,
            MoveTo(0, (h - 2) as u16),
            SetForegroundColor(Color::Yellow),
            Print(fit(&format!(" {FONT_HINT}"), usable)),
            ResetColor,
            Clear(ClearType::UntilNewLine),
        )?;
    }

    // Bottom line: the full name if the selected one is cut short, otherwise
    // the key help; match count on the right.
    let count = format!("{}/{}", s.shown.len(), s.entries.len());
    let left = match s.current() {
        Some(e) if e.width > s.cell => label(e),
        _ => HELP.to_string(),
    };
    let room = usable.saturating_sub(count.len() + 3);
    let help = if room >= 10 {
        let left = fit(&left, room);
        let gap = usable - left.width() - count.len() - 1;
        format!(" {left}{}{count}", " ".repeat(gap))
    } else {
        fit(&format!(" {count}  Esc cancel"), usable)
    };
    queue!(
        out,
        MoveTo(0, (h - 1) as u16),
        SetForegroundColor(Color::DarkGrey),
        Print(help),
        ResetColor,
        Clear(ClearType::UntilNewLine),
    )?;

    // Blinking cursor at the end of the filter text.
    let cursor_x = (2 + query_text.width()).min(usable);
    queue!(out, MoveTo(cursor_x as u16, 1), Show)?;
    out.flush()
}

fn draw_cell(out: &mut impl Write, e: &Entry, cell: usize, selected: bool) -> io::Result<()> {
    let text = fit(&label(e), cell);
    let padding = " ".repeat(cell.saturating_sub(text.width()));
    if selected {
        queue!(out, SetAttribute(Attribute::Reverse), SetAttribute(Attribute::Bold))?;
    }
    if e.is_dir {
        queue!(out, SetForegroundColor(Color::Blue))?;
    }
    queue!(
        out,
        Print(" "),
        Print(text),
        Print(padding),
        Print(" "),
        SetAttribute(Attribute::Reset),
        ResetColor
    )
}

/// Shortens `s` to `max` columns, ending with "…" if cut.
fn fit(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw > max - 1 {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out
}

/// Like `fit`, but keeps the end of the text (useful for paths and long filters).
fn fit_left(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut tail = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        let cw = c.width().unwrap_or(0);
        if used + cw > max - 1 {
            break;
        }
        tail.push(c);
        used += cw;
    }
    let mut out = String::from("…");
    out.extend(tail.into_iter().rev());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(n: usize, w: usize, h: usize) -> State {
        let entries = (0..n).map(|i| Entry::new(format!("f{i:02}.js"), false)).collect();
        let mut s = State::new(entries);
        s.fit(w, h);
        s
    }

    fn press(s: &mut State, code: KeyCode) -> Action {
        s.on_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn grid_layout() {
        // "f00.js" is 6 wide, + 2 padding = 8 per column; 41 wide window -> 40 usable -> 5 columns.
        let s = state(12, 41, 10);
        assert_eq!(s.cell, 6);
        assert_eq!(s.cols, 5);
        assert_eq!(s.visible_rows, 6);
    }

    #[test]
    fn arrows_move_in_the_grid() {
        let mut s = state(12, 41, 10); // rows: 0-4, 5-9, 10-11
        press(&mut s, KeyCode::Right);
        assert_eq!(s.sel, 1);
        press(&mut s, KeyCode::Down);
        assert_eq!(s.sel, 6);
        press(&mut s, KeyCode::Down); // row below has only 2 items -> last item
        assert_eq!(s.sel, 11);
        press(&mut s, KeyCode::Down); // already on last row
        assert_eq!(s.sel, 11);
        press(&mut s, KeyCode::Up);
        assert_eq!(s.sel, 6);
        press(&mut s, KeyCode::Left);
        press(&mut s, KeyCode::Left);
        assert_eq!(s.sel, 4); // wraps to the end of the previous row
        press(&mut s, KeyCode::End);
        assert_eq!(s.sel, 11);
        press(&mut s, KeyCode::Right);
        assert_eq!(s.sel, 11);
        press(&mut s, KeyCode::Home);
        assert_eq!(s.sel, 0);
        press(&mut s, KeyCode::Left);
        assert_eq!(s.sel, 0);
    }

    #[test]
    fn scrolls_to_keep_selection_visible() {
        let mut s = state(100, 41, 7); // 5 columns, 3 visible rows, 20 rows total
        press(&mut s, KeyCode::End);
        assert_eq!(s.sel, 99);
        assert_eq!(s.scroll, 17);
        press(&mut s, KeyCode::Home);
        assert_eq!(s.scroll, 0);
        press(&mut s, KeyCode::PageDown);
        assert_eq!(s.sel, 15);
        assert_eq!(s.scroll, 1);
    }

    #[test]
    fn typing_filters_and_enter_picks() {
        let entries =
            ["server.js", "app.js", "notes.md"].iter().map(|n| Entry::new(n.to_string(), false)).collect();
        let mut s = State::new(entries);
        s.fit(80, 20);
        press(&mut s, KeyCode::Char('m'));
        press(&mut s, KeyCode::Char('d'));
        assert_eq!(s.current().unwrap().name, "notes.md");
        assert_eq!(press(&mut s, KeyCode::Enter), Action::Pick);
        press(&mut s, KeyCode::Char('z'));
        assert!(s.shown.is_empty());
        assert_eq!(press(&mut s, KeyCode::Enter), Action::Redraw); // nothing to pick
        press(&mut s, KeyCode::Backspace);
        assert_eq!(s.shown.len(), 1);
        s.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(s.shown.len(), 3);
    }

    #[test]
    fn altgr_characters_are_typed_not_ignored() {
        let mut s = state(3, 80, 20);
        s.on_key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::CONTROL | KeyModifiers::ALT));
        assert_eq!(s.query, "@");
    }

    #[test]
    fn cancel_keys() {
        let mut s = state(3, 80, 20);
        assert_eq!(press(&mut s, KeyCode::Esc), Action::Cancel);
        assert_eq!(s.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), Action::Cancel);
    }

    #[test]
    fn empty_folder_is_safe() {
        let mut s = state(0, 80, 20);
        for k in [KeyCode::Down, KeyCode::Right, KeyCode::End, KeyCode::PageDown, KeyCode::Enter] {
            assert_ne!(press(&mut s, k), Action::Pick);
        }
        assert!(s.current().is_none());
    }

    #[test]
    fn tiny_window_does_not_panic() {
        let mut s = state(20, 3, 2);
        press(&mut s, KeyCode::Down);
        press(&mut s, KeyCode::End);
        let mut sink = Vec::new();
        draw(&mut sink, &s, "C:\\x", 3, 2).unwrap();
        draw(&mut sink, &s, "C:\\x", 0, 0).unwrap();
    }

    #[test]
    fn one_long_name_does_not_force_a_single_column() {
        let mut names: Vec<String> = (0..15).map(|i| format!("file{i}.js")).collect();
        names.push("a-very-long-file-name-that-goes-on-and-on-and-on.config.js".into());
        let entries = names.into_iter().map(|n| Entry::new(n, false)).collect();
        let mut s = State::new(entries);
        s.fit(80, 20);
        assert_eq!(s.cell, 12);
        assert_eq!(s.cols, 5);
        // All names long -> capped width.
        let entries = (0..5).map(|i| Entry::new(format!("{i}{}", "x".repeat(50)), false)).collect();
        let s = State::new(entries);
        assert_eq!(s.cell, MAX_CELL);
    }

    #[test]
    fn console_fonts_lack_some_scripts() {
        assert!(console_can_show("Café Ünïcode Ελληνικά Кириллица – notes.txt"));
        assert!(!console_can_show("ساره.txt"));
        assert!(!console_can_show("日本.txt"));
        assert!(!console_can_show("🎉.txt"));
    }

    #[test]
    fn font_hint_takes_a_row() {
        let mut s = state(100, 80, 20);
        let rows = s.visible_rows;
        s.font_hint = true;
        s.fit(80, 20);
        assert_eq!(s.visible_rows, rows - 1);
    }

    #[test]
    fn shortening_text() {
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("abc", 4), "abc");
        assert_eq!(fit_left("C:\\Users\\me\\project", 10), "…e\\project");
        assert_eq!(fit("漢字漢字", 5), "漢字…");
    }
}
