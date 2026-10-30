//! Talking to the console directly instead of through stdout.
//!
//! When a shell hook runs filepick, the shell usually captures the program's
//! output. Drawing on the console itself (CONOUT$ on Windows, /dev/tty
//! elsewhere) keeps the palette visible no matter what happens to stdout.
//!
//! `Session` switches the console into palette mode and puts everything back
//! exactly as it was when it is dropped, including after a panic.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crossterm::{cursor, execute, terminal};

/// Buffered writer to the console. Text is sent in one go on `flush`.
pub struct Term {
    out: imp::Out,
    buf: Vec<u8>,
}

impl Term {
    pub fn open() -> io::Result<Term> {
        Ok(Term { out: imp::Out::open()?, buf: Vec::with_capacity(32 * 1024) })
    }
}

impl Write for Term {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let result = self.out.write_text(&self.buf);
        self.buf.clear();
        result
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static SAVED_MODES: Mutex<Option<imp::Modes>> = Mutex::new(None);

/// The console in palette mode: raw keys, alternate screen, cursor hidden.
pub struct Session {
    pub out: Term,
}

impl Session {
    pub fn enter() -> io::Result<Session> {
        let out = Term::open().map_err(|e| {
            io::Error::new(e.kind(), format!("no console to draw on ({e}); run filepick from a terminal"))
        })?;
        if let Ok(mut saved) = SAVED_MODES.lock() {
            *saved = Some(imp::Modes::save());
        }
        ACTIVE.store(true, Ordering::SeqCst);

        // From here on, dropping `session` (including on an early `?`) restores the console.
        let mut session = Session { out };
        terminal::enable_raw_mode()?;
        execute!(session.out, terminal::EnterAlternateScreen, cursor::Hide)?;
        // Re-open after switching screens: on old consoles without escape-code
        // support the alternate screen is a separate buffer.
        session.out = Term::open()?;
        Ok(session)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.out.flush();
        restore();
    }
}

/// Puts the console back the way it was. Safe to call more than once.
pub fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    if let Ok(mut t) = Term::open() {
        let _ = execute!(t, cursor::Show, terminal::LeaveAlternateScreen);
    }
    let _ = terminal::disable_raw_mode();
    // crossterm's disable_raw_mode switches a fixed set of flags back on; the
    // shell may have had a different mode (PSReadLine does), so restore the
    // exact original.
    if let Some(modes) = SAVED_MODES.lock().ok().and_then(|mut m| m.take()) {
        modes.restore();
    }
}

/// Make sure a crash never leaves the user's terminal stuck in palette mode.
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        default_hook(info);
    }));
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    extern "system" {
        fn WriteConsoleW(
            console: *mut c_void,
            buffer: *const u16,
            chars: u32,
            written: *mut u32,
            reserved: *const c_void,
        ) -> i32;
        fn GetConsoleMode(console: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(console: *mut c_void, mode: u32) -> i32;
    }

    fn open_console(name: &str) -> io::Result<File> {
        OpenOptions::new().read(true).write(true).open(name)
    }

    fn raw(f: &File) -> *mut c_void {
        f.as_raw_handle()
    }

    pub struct Out(File);

    impl Out {
        pub fn open() -> io::Result<Out> {
            open_console("CONOUT$").map(Out)
        }

        /// Writes as UTF-16 so names in any language show correctly,
        /// whatever code page the console is set to.
        pub fn write_text(&mut self, bytes: &[u8]) -> io::Result<()> {
            let text = String::from_utf8_lossy(bytes);
            let wide: Vec<u16> = text.encode_utf16().collect();
            let mut rest = &wide[..];
            while !rest.is_empty() {
                let mut n = rest.len().min(8192);
                // Never split a surrogate pair between two calls.
                if n < rest.len() && (0xD800..0xDC00).contains(&rest[n - 1]) {
                    n -= 1;
                }
                let mut written = 0u32;
                // SAFETY: the handle is a live console handle owned by `self.0`,
                // and `rest[..n]` is a valid buffer of `n` UTF-16 units.
                let ok = unsafe {
                    WriteConsoleW(raw(&self.0), rest.as_ptr(), n as u32, &mut written, std::ptr::null())
                };
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                if written == 0 {
                    return Err(io::Error::new(io::ErrorKind::WriteZero, "console accepted no text"));
                }
                rest = &rest[(written as usize).min(rest.len())..];
            }
            Ok(())
        }
    }

    /// The console's input and output modes before filepick touched them.
    pub struct Modes {
        input: Option<(File, u32)>,
        output: Option<(File, u32)>,
    }

    impl Modes {
        pub fn save() -> Modes {
            Modes { input: save_one("CONIN$"), output: save_one("CONOUT$") }
        }

        pub fn restore(&self) {
            for (file, mode) in [&self.input, &self.output].into_iter().flatten() {
                // SAFETY: `file` is an open console handle.
                unsafe {
                    SetConsoleMode(raw(file), *mode);
                }
            }
        }
    }

    fn save_one(name: &str) -> Option<(File, u32)> {
        let file = open_console(name).ok()?;
        let mut mode = 0u32;
        // SAFETY: `file` is an open console handle and `mode` is a valid out-pointer.
        let ok = unsafe { GetConsoleMode(raw(&file), &mut mode) };
        (ok != 0).then_some((file, mode))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io::{self, Write};

    pub struct Out(File);

    impl Out {
        pub fn open() -> io::Result<Out> {
            OpenOptions::new().write(true).open("/dev/tty").map(Out)
        }

        pub fn write_text(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.0.write_all(bytes)?;
            self.0.flush()
        }
    }

    /// crossterm already restores the exact terminal settings on Unix.
    pub struct Modes;

    impl Modes {
        pub fn save() -> Modes {
            Modes
        }
        pub fn restore(&self) {}
    }
}
