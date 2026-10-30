//! `filepick setup`, `filepick remove` and `filepick doctor`.
//!
//! setup: adds one marked line to the PowerShell profile of each installed
//! PowerShell (and to ~/.bashrc for Git Bash), registers the Clink script, and
//! with --allow-scripts lets PowerShell run the hook. Running it again changes
//! nothing. It writes down what it did in `%APPDATA%\filepick\setup-state.txt`
//! so `remove` can undo exactly that.
//!
//! The hook scripts (filepick.ps1, .lua, .bash) are expected next to
//! filepick.exe, as the installer puts them; --hooks DIR points elsewhere.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use crate::{config, profile};

const SHELLS: [(&str, &str); 2] = [("Windows PowerShell", "powershell.exe"), ("PowerShell 7", "pwsh.exe")];
const STATE_FILE: &str = "setup-state.txt";

pub const HELP: &str = "\
filepick setup / remove / doctor

USAGE:
    filepick setup [OPTIONS]     Turn the hotkey on in PowerShell, CMD (Clink) and Git Bash
    filepick remove [OPTIONS]    Undo everything setup did
    filepick doctor              Check the setup and explain any problem

OPTIONS:
    --allow-scripts     Let PowerShell load the hotkey script: sets the
                        CurrentUser execution policy to RemoteSigned if needed
    --hooks <DIR>       Folder with filepick.ps1/.lua/.bash (default: next to filepick.exe)
    --profile <FILE>    Use this PowerShell profile instead of the real ones (repeatable; for testing)
    --bashrc <FILE>     Use this file instead of ~/.bashrc
    --no-clink          Leave Clink alone
    --no-bash           Leave Git Bash alone
";

#[derive(Default)]
struct Options {
    allow_scripts: bool,
    hooks: Option<PathBuf>,
    profiles: Vec<PathBuf>,
    bashrc: Option<PathBuf>,
    no_clink: bool,
    no_bash: bool,
}

/// Runs `setup`, `remove` or `doctor` with the arguments after it.
pub fn main(command: &str, args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let mut opts = Options::default();
    let mut args = args;
    while let Some(arg) = args.next() {
        let arg = arg.into_string().map_err(|_| "arguments must be valid text".to_string())?;
        let mut value = |flag: &str| -> Result<PathBuf, String> {
            args.next().map(PathBuf::from).ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "--allow-scripts" => opts.allow_scripts = true,
            "--hooks" => opts.hooks = Some(value("--hooks")?),
            "--profile" => opts.profiles.push(value("--profile")?),
            "--bashrc" => opts.bashrc = Some(value("--bashrc")?),
            "--no-clink" => opts.no_clink = true,
            "--no-bash" => opts.no_bash = true,
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(ExitCode::SUCCESS);
            }
            other => return Err(format!("unknown option '{other}' (try filepick {command} --help)")),
        }
    }
    match command {
        "setup" => setup(&opts),
        "remove" => remove(&opts),
        _ => doctor(&opts),
    }
}

// ---------------------------------------------------------------------------
// setup
// ---------------------------------------------------------------------------

fn setup(opts: &Options) -> Result<ExitCode, String> {
    let hooks = hooks_dir(opts)?;
    let ps_hook = hooks.join("filepick.ps1");
    if !ps_hook.is_file() {
        return Err(format!("can't find {} (use --hooks to say where the scripts are)", ps_hook.display()));
    }
    let mut state = State::load();
    let line = ps_profile_line(&ps_hook);
    let mut warnings = 0;

    // PowerShell profiles.
    for target in powershell_targets(opts) {
        let name = target.name;
        match add_to_file(&target.profile, &line, "\r\n", true) {
            Ok(change) => record(&mut state, "profile", &target.profile, &change, name),
            Err(e) => {
                warnings += 1;
                println!("{name}: couldn't change {}: {e}", target.profile.display());
                continue;
            }
        }
        if let Some(info) = &target.info {
            warnings += usize::from(!check_policy(name, target.exe, info, opts.allow_scripts, &mut state));
            if let Some(v) = info.get("psreadline").filter(|v| !v.is_empty()) {
                if version_less(v, "2.0") {
                    warnings += 1;
                    println!("{name}: PSReadLine {v} is too old for the hotkey; 2.0 or newer is needed.");
                }
            }
        }
    }

    // CMD through Clink.
    if !opts.no_clink {
        match find_clink() {
            Some(clink) if hooks.join("filepick.lua").is_file() => {
                if clink_has_scripts(&clink, &hooks) || clink_install_scripts(&clink, &hooks) {
                    state.add("clink", &hooks.to_string_lossy());
                    println!("CMD: Clink will load the hotkey from {}", hooks.display());
                    if !clink_autorun(&clink) {
                        println!(
                            "CMD: Clink isn't set to start with CMD, so only windows where you start \
                             Clink get the hotkey. To start it always, run: clink autorun install"
                        );
                    }
                } else {
                    warnings += 1;
                    println!("CMD: Clink didn't accept the script folder {}", hooks.display());
                }
            }
            Some(_) => println!("CMD: filepick.lua is missing from {}; skipped", hooks.display()),
            None => println!(
                "CMD: Clink isn't installed, so CMD gets no hotkey (CMD can't have hotkeys without it). \
                 Get it from https://chrisant996.github.io/clink/ and run filepick setup again."
            ),
        }
    }

    // Git Bash.
    if let Some(bashrc) = bashrc_target(opts) {
        let bash_hook = hooks.join("filepick.bash");
        if bash_hook.is_file() {
            match add_to_file(&bashrc, &bash_rc_line(&bash_hook), "\n", false) {
                Ok(change) => record(&mut state, "bashrc", &bashrc, &change, "Git Bash"),
                Err(e) => {
                    warnings += 1;
                    println!("Git Bash: couldn't change {}: {e}", bashrc.display());
                }
            }
        }
    }

    state.save()?;
    if warnings == 0 {
        println!("\nDone. Open a new terminal and press {} while typing a command.", hotkey());
    } else {
        println!("\nSetup finished, but see the notes above. `filepick doctor` explains more.");
    }
    Ok(ExitCode::SUCCESS)
}

/// Checks the execution policy and, if allowed, fixes it. Returns false if
/// PowerShell still won't load the hook.
fn check_policy(
    name: &str,
    exe: &str,
    info: &BTreeMap<String, String>,
    allow: bool,
    state: &mut State,
) -> bool {
    let policy = Policy::from(info);
    if policy.allows_scripts() {
        return true;
    }
    if let Some(scope) = policy.group_policy() {
        println!(
            "{name}: your organization's Group Policy ({scope}) sets the execution policy to {}, which \
             blocks the hotkey script. Only your IT admin can change that.",
            policy.effective
        );
        return false;
    }
    if !allow {
        println!(
            "{name}: the execution policy is {}, so PowerShell won't load the hotkey. To allow it, run \
             `filepick setup --allow-scripts` (sets your own policy to RemoteSigned, the usual setting).",
            policy.effective
        );
        return false;
    }
    let before = policy.current_user.clone();
    let ok =
        powershell(exe, "Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned -Force; 'ok'")
            .is_some_and(|out| out.trim() == "ok");
    let now = powershell_info(exe).map(|i| Policy::from(&i));
    if ok && now.as_ref().is_some_and(Policy::allows_scripts) {
        // Only the first change is remembered, so remove restores the original.
        if state.get(&format!("policy:{exe}")).is_none() {
            state.add(&format!("policy:{exe}"), &before);
        }
        println!("{name}: set your execution policy to RemoteSigned so the hotkey can load.");
        true
    } else {
        println!("{name}: couldn't change the execution policy. `filepick doctor` explains more.");
        false
    }
}

// ---------------------------------------------------------------------------
// remove
// ---------------------------------------------------------------------------

fn remove(opts: &Options) -> Result<ExitCode, String> {
    let state = State::load();
    let mut problems = 0;

    let mut profiles: Vec<PathBuf> = state.all("profile").map(PathBuf::from).collect();
    profiles.extend(powershell_targets(opts).into_iter().map(|t| t.profile));
    if let Some(b) = bashrc_target(opts) {
        profiles.push(b);
    }
    profiles.extend(state.all("bashrc").map(PathBuf::from));
    profiles.sort();
    profiles.dedup();
    for file in &profiles {
        let created = state.all("created").any(|c| Path::new(c) == file);
        match remove_from_file(file, created) {
            Ok(true) => println!("Removed the hotkey line from {}", file.display()),
            Ok(false) => {}
            Err(e) => {
                problems += 1;
                println!("Couldn't change {}: {e}", file.display());
            }
        }
    }

    if !opts.no_clink {
        if let Some(clink) = find_clink() {
            let mut dirs: Vec<PathBuf> = state.all("clink").map(PathBuf::from).collect();
            if let Ok(h) = hooks_dir(opts) {
                dirs.push(h);
            }
            dirs.sort();
            dirs.dedup();
            for dir in dirs {
                if clink_has_scripts(&clink, &dir) {
                    if clink_uninstall_scripts(&clink, &dir) {
                        println!("Clink no longer loads scripts from {}", dir.display());
                    } else {
                        problems += 1;
                        println!("Couldn't remove {} from Clink", dir.display());
                    }
                }
            }
        }
    }

    for (name, exe) in SHELLS {
        let Some(before) = state.get(&format!("policy:{exe}")) else { continue };
        let script = format!("Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy {before} -Force; 'ok'");
        if powershell(exe, &script).is_some_and(|o| o.trim() == "ok") {
            println!("{name}: put your execution policy back to {before}");
        } else {
            problems += 1;
            println!("{name}: couldn't put your execution policy back to {before}");
        }
    }

    if problems == 0 {
        State::delete();
        println!("filepick is removed from your shells.");
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(2))
    }
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

fn doctor(opts: &Options) -> Result<ExitCode, String> {
    let mut problems = 0;
    let ok = |text: String| println!("  ok   {text}");
    let mut bad = |text: String, fix: String| {
        problems += 1;
        println!("  !!   {text}\n       Fix: {fix}");
    };

    println!("filepick {}", env!("CARGO_PKG_VERSION"));
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent).map(Path::to_path_buf);
    match &exe_dir {
        Some(dir) if on_path(dir) => ok(format!("{} is on PATH", dir.display())),
        Some(dir) => bad(
            format!("{} isn't on PATH, so `filepick` can't be run by name", dir.display()),
            "reinstall filepick, or open a new terminal if you just installed it".into(),
        ),
        None => {}
    }

    let hooks = hooks_dir(opts).ok();
    let ps_hook = hooks.as_ref().map(|h| h.join("filepick.ps1"));
    match &ps_hook {
        Some(h) if h.is_file() => ok(format!("hook scripts are in {}", h.parent().unwrap_or(h).display())),
        _ => bad("the hook scripts aren't next to filepick.exe".into(), "reinstall filepick".into()),
    }

    let mut any_powershell = false;
    for target in powershell_targets(opts) {
        any_powershell = true;
        let name = target.name;
        let profile_ok = std::fs::read(&target.profile).map(|b| profile::has_line(&b)).unwrap_or(false);
        if profile_ok {
            ok(format!("{name}: hotkey line is in {}", target.profile.display()));
        } else {
            bad(
                format!("{name}: the hotkey line isn't in {}", target.profile.display()),
                "run `filepick setup`".into(),
            );
        }
        let Some(info) = &target.info else { continue };
        let policy = Policy::from(info);
        if policy.allows_scripts() {
            ok(format!("{name}: execution policy {} lets the hotkey load", policy.effective));
        } else if let Some(scope) = policy.group_policy() {
            bad(
                format!("{name}: Group Policy ({scope}) sets the execution policy to {}", policy.effective),
                "only your IT admin can change this".into(),
            );
        } else {
            bad(
                format!("{name}: execution policy {} blocks the hotkey script", policy.effective),
                "run `filepick setup --allow-scripts`".into(),
            );
        }
        match info.get("psreadline").filter(|v| !v.is_empty()) {
            Some(v) if !version_less(v, "2.0") => ok(format!("{name}: PSReadLine {v}")),
            Some(v) => bad(
                format!("{name}: PSReadLine {v} is too old"),
                "run `Install-Module PSReadLine -Scope CurrentUser -Force` in that PowerShell".into(),
            ),
            None => bad(format!("{name}: PSReadLine isn't installed"), "reinstall PowerShell".into()),
        }
    }
    if !any_powershell {
        println!("  --   no PowerShell found");
    }

    if !opts.no_clink {
        match (find_clink(), &hooks) {
            (Some(clink), Some(h)) => {
                if clink_has_scripts(&clink, h) {
                    ok("CMD: Clink loads the hotkey script".into());
                } else {
                    bad("CMD: Clink doesn't load the hotkey script".into(), "run `filepick setup`".into());
                }
                if clink_autorun(&clink) {
                    ok("CMD: Clink starts with every CMD window".into());
                } else {
                    bad(
                        "CMD: Clink doesn't start with CMD, so CMD windows have no hotkey".into(),
                        "run `clink autorun install`".into(),
                    );
                }
            }
            _ => println!(
                "  --   CMD: Clink isn't installed, so CMD can't have the hotkey. \
                 It's free: https://chrisant996.github.io/clink/"
            ),
        }
    }

    if let Some(bashrc) = bashrc_target(opts) {
        if std::fs::read(&bashrc).map(|b| profile::has_line(&b)).unwrap_or(false) {
            ok(format!("Git Bash: hotkey line is in {}", bashrc.display()));
        } else {
            bad(
                format!("Git Bash: the hotkey line isn't in {}", bashrc.display()),
                "run `filepick setup`".into(),
            );
        }
    }

    match config::path() {
        Some(p) if p.is_file() => ok(format!("settings: {} (hotkey {})", p.display(), hotkey())),
        Some(p) => println!("  --   no settings file ({}); using hotkey {}", p.display(), hotkey()),
        None => {}
    }

    println!("\nNames in other languages look like ???? on the PowerShell command line? That's only");
    println!("how PowerShell draws them; the command still gets the right name. To see them, add");
    println!("this line to your PowerShell profile: [Console]::OutputEncoding = [Text.Encoding]::UTF8");

    if problems == 0 {
        println!("\nEverything looks fine.");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("\n{problems} problem(s) found.");
        Ok(ExitCode::from(1))
    }
}

// ---------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------

fn hotkey() -> String {
    config::load().hotkey.unwrap_or_else(|| config::DEFAULT_HOTKEY.to_string())
}

fn hooks_dir(opts: &Options) -> Result<PathBuf, String> {
    if let Some(h) = &opts.hooks {
        return std::path::absolute(h).map_err(|e| format!("bad --hooks folder: {e}"));
    }
    let exe = std::env::current_exe().map_err(|e| format!("can't find filepick.exe itself: {e}"))?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| "can't find filepick.exe's folder".into())
}

struct PowerShellTarget {
    name: &'static str,
    exe: &'static str,
    profile: PathBuf,
    /// What the shell said about itself; None when testing with --profile.
    info: Option<BTreeMap<String, String>>,
}

/// The profiles to change. With --profile, exactly those (no shell is asked).
/// Otherwise each installed PowerShell is asked for its own profile path,
/// because Documents is often moved into OneDrive.
fn powershell_targets(opts: &Options) -> Vec<PowerShellTarget> {
    if !opts.profiles.is_empty() {
        return opts
            .profiles
            .iter()
            .map(|p| PowerShellTarget { name: "PowerShell", exe: "", profile: p.clone(), info: None })
            .collect();
    }
    let mut out = Vec::new();
    for (name, exe) in SHELLS {
        let Some(info) = powershell_info(exe) else { continue };
        let Some(profile) = info.get("profile").filter(|p| !p.is_empty()) else { continue };
        let profile = PathBuf::from(profile);
        // pwsh and Windows PowerShell never share a profile, but be safe.
        if out.iter().any(|t: &PowerShellTarget| t.profile == profile) {
            continue;
        }
        out.push(PowerShellTarget { name, exe, profile, info: Some(info) });
    }
    out
}

fn powershell_info(exe: &str) -> Option<BTreeMap<String, String>> {
    let out = powershell(
        exe,
        "'profile=' + $PROFILE.CurrentUserAllHosts\n\
         'version=' + $PSVersionTable.PSVersion\n\
         'psreadline=' + (Get-Module PSReadLine -ListAvailable | Sort-Object Version -Descending | \
             Select-Object -First 1).Version\n\
         Get-ExecutionPolicy -List | ForEach-Object { 'policy.' + $_.Scope + '=' + $_.ExecutionPolicy }\n\
         'policy.Effective=' + (Get-ExecutionPolicy)",
    )?;
    Some(
        out.lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect(),
    )
}

/// Runs a PowerShell script without a window or profile; returns its output.
fn powershell(exe: &str, script: &str) -> Option<String> {
    // UTF-8 output so paths in any language come through, and
    // -EncodedCommand so no quoting can go wrong on the way.
    let full = format!(
        "$ErrorActionPreference = 'Stop'\n\
         [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false\n{script}"
    );
    let utf16: Vec<u8> = full.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut cmd = Command::new(exe);
    cmd.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", &base64(&utf16)]);
    let out = output(cmd)?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim_start_matches('\u{feff}').to_string())
}

fn output(mut cmd: Command) -> Option<std::process::Output> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output().ok()
}

struct Policy {
    effective: String,
    current_user: String,
    machine_policy: String,
    user_policy: String,
}

impl Policy {
    fn from(info: &BTreeMap<String, String>) -> Policy {
        let get = |k: &str| info.get(k).cloned().unwrap_or_else(|| "Undefined".into());
        Policy {
            effective: get("policy.Effective"),
            current_user: get("policy.CurrentUser"),
            machine_policy: get("policy.MachinePolicy"),
            user_policy: get("policy.UserPolicy"),
        }
    }

    fn allows_scripts(&self) -> bool {
        matches!(self.effective.to_ascii_lowercase().as_str(), "remotesigned" | "unrestricted" | "bypass")
    }

    /// The Group Policy scope that decides the policy, if any.
    fn group_policy(&self) -> Option<&'static str> {
        if !self.machine_policy.eq_ignore_ascii_case("undefined") {
            Some("MachinePolicy")
        } else if !self.user_policy.eq_ignore_ascii_case("undefined") {
            Some("UserPolicy")
        } else {
            None
        }
    }
}

/// The line for the PowerShell profile. It does nothing if filepick was
/// deleted without `filepick remove`, so it can never break the profile.
fn ps_profile_line(hook: &Path) -> String {
    let p = ps_path(hook);
    format!("if (Test-Path -LiteralPath {p}) {{ . {p} -Quiet }}  {}", profile::MARKER)
}

/// A PowerShell string for `path`. Under %LOCALAPPDATA% (where the installer
/// puts filepick) it's written as "$env:LOCALAPPDATA\..." so the line is
/// plain ASCII even when the user name isn't, which keeps the profile's
/// encoding out of the picture.
fn ps_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = under_localappdata(&text) {
        let rest = rest.replace('`', "``").replace('$', "`$");
        return format!("\"$env:LOCALAPPDATA{rest}\"");
    }
    format!("'{}'", text.replace('\'', "''"))
}

/// The line for ~/.bashrc, built the same way.
fn bash_rc_line(hook: &Path) -> String {
    let text = hook.to_string_lossy();
    let p = match under_localappdata(&text) {
        Some(rest) => format!("\"$LOCALAPPDATA{}\"", rest.replace('\\', "/").replace(['$', '`', '"'], "")),
        None => sh_quote(&msys_path(&text)),
    };
    format!("[ -f {p} ] && . {p}  {}", profile::MARKER)
}

/// The part of `path` after %LOCALAPPDATA%, starting with a separator.
fn under_localappdata(path: &str) -> Option<String> {
    let base = std::env::var("LOCALAPPDATA").ok()?;
    let base = base.trim_end_matches(['\\', '/']);
    let head = path.get(..base.len())?;
    let rest = &path[base.len()..];
    (head.eq_ignore_ascii_case(base) && rest.starts_with(['\\', '/'])).then(|| rest.to_string())
}

/// C:\Users\x -> /c/Users/x
fn msys_path(path: &str) -> String {
    let p = path.replace('\\', "/");
    match p.as_bytes() {
        [d, b':', b'/', ..] if d.is_ascii_alphabetic() => {
            format!("/{}{}", (*d as char).to_ascii_lowercase(), &p[2..])
        }
        _ => p,
    }
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// ~/.bashrc if Git Bash is installed (or --bashrc), unless --no-bash.
fn bashrc_target(opts: &Options) -> Option<PathBuf> {
    if opts.no_bash {
        return None;
    }
    if let Some(b) = &opts.bashrc {
        return Some(b.clone());
    }
    if !opts.profiles.is_empty() {
        return None; // testing with fake profiles: never touch the real ~/.bashrc
    }
    find_git_bash()?;
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".bashrc"))
}

fn find_git_bash() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for var in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(var) {
            let base = PathBuf::from(base);
            candidates.push(base.join("Git").join("bin").join("bash.exe"));
            candidates.push(base.join("Programs").join("Git").join("bin").join("bash.exe"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[derive(PartialEq, Eq)]
enum Change {
    Unchanged,
    Changed,
    /// The file didn't exist before.
    Created,
}

fn add_to_file(file: &Path, line: &str, newline: &'static str, bom: bool) -> Result<Change, String> {
    let existing = match std::fs::read(file) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let Some(new) = profile::add_line(existing.as_deref(), line, newline, bom)? else {
        return Ok(Change::Unchanged);
    };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(file, new).map_err(|e| e.to_string())?;
    Ok(if existing.is_none() { Change::Created } else { Change::Changed })
}

/// Notes a file setup touched, and prints what happened to it.
fn record(state: &mut State, key: &str, file: &Path, change: &Change, name: &str) {
    let path = file.to_string_lossy();
    state.add(key, &path);
    if *change == Change::Created {
        state.add("created", &path);
    }
    let what = if *change == Change::Unchanged { "already set up in" } else { "added the hotkey to" };
    println!("{name}: {what} {}", file.display());
}

/// Takes filepick's line out. A file setup created is deleted again once
/// nothing else is in it.
fn remove_from_file(file: &Path, created_by_setup: bool) -> Result<bool, String> {
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    let Some(new) = profile::remove_lines(&bytes)? else { return Ok(false) };
    if created_by_setup && profile::is_blank(&new) {
        std::fs::remove_file(file).map_err(|e| e.to_string())?;
    } else {
        std::fs::write(file, new).map_err(|e| e.to_string())?;
    }
    Ok(true)
}

fn find_clink() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(base).join("Programs").join("clink").join("clink_x64.exe"));
    }
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(base) = std::env::var_os(var) {
            candidates.push(PathBuf::from(base).join("clink").join("clink_x64.exe"));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("clink_x64.exe")));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn clink(clink: &Path, args: &[&std::ffi::OsStr]) -> Option<(bool, String)> {
    let mut cmd = Command::new(clink);
    cmd.args(args);
    let out = output(cmd)?;
    Some((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned()))
}

fn same_dir(a: &str, b: &Path) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase();
    norm(a) == norm(&b.to_string_lossy())
}

fn clink_has_scripts(exe: &Path, dir: &Path) -> bool {
    clink(exe, &["installscripts".as_ref(), "--list".as_ref()])
        .is_some_and(|(_, out)| out.lines().any(|l| same_dir(l, dir)))
}

fn clink_install_scripts(exe: &Path, dir: &Path) -> bool {
    clink(exe, &["installscripts".as_ref(), dir.as_os_str()]).is_some() && clink_has_scripts(exe, dir)
}

fn clink_uninstall_scripts(exe: &Path, dir: &Path) -> bool {
    clink(exe, &["uninstallscripts".as_ref(), dir.as_os_str()]).is_some() && !clink_has_scripts(exe, dir)
}

/// Whether Clink is set to start in every CMD window.
fn clink_autorun(exe: &Path) -> bool {
    clink(exe, &["autorun".as_ref(), "show".as_ref()])
        .is_some_and(|(_, out)| out.to_ascii_lowercase().contains("clink") && out.contains("inject"))
}

fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| same_dir(&d.to_string_lossy(), dir)))
}

/// "2.0.0" < "2.1"? Compares the numbers one by one.
fn version_less(a: &str, b: &str) -> bool {
    let nums = |s: &str| s.split(['.', '-']).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (mut a, mut b) = (nums(a), nums(b));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a < b
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// What setup did, so remove can undo it. One `key=value` per line.
struct State {
    entries: Vec<(String, String)>,
}

impl State {
    fn file() -> Option<PathBuf> {
        Some(config::dir()?.join(STATE_FILE))
    }

    fn load() -> State {
        let text = Self::file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let entries = text
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        State { entries }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries.iter().filter(move |(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn add(&mut self, key: &str, value: &str) {
        if !self.entries.iter().any(|(k, v)| k == key && v == value) {
            self.entries.push((key.to_string(), value.to_string()));
        }
    }

    fn save(&self) -> Result<(), String> {
        let file = Self::file().ok_or("can't find the settings folder (APPDATA isn't set)")?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
        }
        let mut text = String::from("# Written by `filepick setup` so `filepick remove` can undo it.\n");
        for (k, v) in &self.entries {
            text.push_str(&format!("{k}={v}\n"));
        }
        std::fs::write(&file, text).map_err(|e| format!("can't write {}: {e}", file.display()))
    }

    fn delete() {
        if let Some(f) = Self::file() {
            let _ = std::fs::remove_file(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(version_less("1.2", "2.0"));
        assert!(!version_less("2.0.0", "2.0"));
        assert!(!version_less("2.10", "2.9"));
    }

    #[test]
    fn profile_line_outside_localappdata_is_single_quoted() {
        let line = ps_profile_line(Path::new(r"D:\it's\filepick.ps1"));
        assert_eq!(
            line,
            r"if (Test-Path -LiteralPath 'D:\it''s\filepick.ps1') { . 'D:\it''s\filepick.ps1' -Quiet }  # filepick-setup"
        );
    }

    #[test]
    fn msys_paths() {
        assert_eq!(msys_path(r"C:\Program Files\x"), "/c/Program Files/x");
        assert_eq!(msys_path("/already/unix"), "/already/unix");
    }

    #[test]
    fn group_policy_wins() {
        let info: BTreeMap<String, String> = [
            ("policy.MachinePolicy", "AllSigned"),
            ("policy.UserPolicy", "Undefined"),
            ("policy.Effective", "AllSigned"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let p = Policy::from(&info);
        assert!(!p.allows_scripts());
        assert_eq!(p.group_policy(), Some("MachinePolicy"));
    }
}
