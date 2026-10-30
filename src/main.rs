//! filepick — a hotkey file palette for the terminal.
//!
//! Shows the files in a folder as a grid, lets you move with the arrow keys
//! and filter by typing, and hands back the name you pick. A small shell hook
//! (see `shell/filepick.ps1`) binds it to a hotkey and pastes the name at the
//! cursor.
//!
//! Exit codes: 0 = picked, 1 = cancelled, 2 = error.

mod config;
mod filter;
mod list;
mod profile;
mod quote;
mod setup;
mod term;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

use quote::QuoteStyle;

const HELP: &str = "\
filepick - pick a file from the current folder with the arrow keys

USAGE:
    filepick [OPTIONS]
    filepick setup | remove | doctor    Turn the hotkey on / off / check it (see filepick setup --help)

OPTIONS:
    --dir <PATH>        Folder to list (default: current folder)
    --out <FILE>        Write the picked name to FILE instead of printing it
    --quote <STYLE>     Quote the name for a shell: none, ps, cmd, sh (default: none)
    --ext <LIST>        Only show these file types, e.g. --ext js,mjs,ts
    --no-dirs           Hide folders
    --all               Also show hidden and system files
    -h, --help          Show this help
    -V, --version       Show the version

KEYS:
    Arrows / Tab        Move
    Home / End          First / last item
    PgUp / PgDn         Move a page
    Type                Filter the list
    Backspace / Ctrl+U  Delete a character / clear the filter
    Enter               Pick
    Esc / Ctrl+C        Cancel

EXIT CODES:
    0 picked, 1 cancelled, 2 error
";

struct Args {
    dir: PathBuf,
    out: Option<PathBuf>,
    quote: QuoteStyle,
    list: list::Options,
}

enum Parsed {
    Run(Args),
    Exit(ExitCode),
}

fn parse_args() -> Result<Parsed, String> {
    let mut dir: Option<PathBuf> = None;
    let mut out = None;
    let mut quote = QuoteStyle::None;
    let mut opts = list::Options::default();
    let mut ext_given = false;

    let mut it = std::env::args_os().skip(1).peekable();
    match it.peek().and_then(|a| a.to_str()) {
        Some(cmd @ ("setup" | "remove" | "doctor")) => {
            let cmd = cmd.to_string();
            it.next();
            return setup::main(&cmd, it).map(Parsed::Exit);
        }
        // For the CMD and Git Bash hooks: the hotkey from the settings file
        // in readline notation, e.g. \ef for Alt+F.
        Some("key") => {
            let hotkey = config::load().hotkey.unwrap_or_else(|| config::DEFAULT_HOTKEY.to_string());
            let key = config::readline_key(&hotkey)
                .or_else(|| config::readline_key(config::DEFAULT_HOTKEY))
                .unwrap_or_default();
            println!("{key}");
            return Ok(Parsed::Exit(ExitCode::SUCCESS));
        }
        _ => {}
    }
    while let Some(arg) = it.next() {
        let arg = arg.into_string().map_err(|_| "arguments must be valid text".to_string())?;
        // Accept both "--opt value" and "--opt=value".
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |flag: &str| -> Result<std::ffi::OsString, String> {
            match &inline {
                Some(v) => Ok(v.into()),
                None => it.next().ok_or_else(|| format!("{flag} needs a value")),
            }
        };
        match name.as_str() {
            "--dir" => dir = Some(PathBuf::from(value("--dir")?)),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--quote" => {
                let v = value("--quote")?.to_string_lossy().to_string();
                quote = QuoteStyle::parse(&v)
                    .ok_or_else(|| format!("unknown --quote style '{v}' (use none, ps, cmd or sh)"))?;
            }
            "--ext" => {
                ext_given = true;
                let v = value("--ext")?.to_string_lossy().to_string();
                opts.extensions = v
                    .split(',')
                    .map(|e| e.trim().trim_start_matches('.').to_lowercase())
                    .filter(|e| !e.is_empty())
                    .collect();
            }
            "--no-dirs" => opts.include_dirs = false,
            "--all" => opts.include_hidden = true,
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(Parsed::Exit(ExitCode::SUCCESS));
            }
            "-V" | "--version" => {
                println!("filepick {}", env!("CARGO_PKG_VERSION"));
                return Ok(Parsed::Exit(ExitCode::SUCCESS));
            }
            other => return Err(format!("unknown option '{other}' (try --help)")),
        }
    }

    if !ext_given {
        opts.extensions = config::load().extensions;
    }
    let dir = match dir {
        Some(d) => d,
        None => std::env::current_dir().map_err(|e| format!("can't read the current folder: {e}"))?,
    };
    Ok(Parsed::Run(Args { dir, out, quote, list: opts }))
}

fn run() -> Result<ExitCode, String> {
    let args = match parse_args()? {
        Parsed::Run(a) => a,
        Parsed::Exit(code) => return Ok(code),
    };

    if !args.dir.is_dir() {
        return Err(format!("'{}' is not a folder", args.dir.display()));
    }
    let entries =
        list::read(&args.dir, &args.list).map_err(|e| format!("can't list '{}': {e}", args.dir.display()))?;

    let picked = ui::run(&args.dir, entries).map_err(|e| format!("terminal error: {e}"))?;

    let Some(entry) = picked else {
        return Ok(ExitCode::from(1));
    };
    let text = args.quote.apply(&entry.name);

    match &args.out {
        Some(path) => std::fs::write(path, text.as_bytes())
            .map_err(|e| format!("can't write '{}': {e}", path.display()))?,
        None => println!("{text}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    term::install_panic_hook();
    match run() {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("filepick: {msg}");
            ExitCode::from(2)
        }
    }
}
