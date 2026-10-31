# filepick

Stop typing file names. Press **Alt+F** while typing a command, pick a file
with the arrow keys, press **Enter**, and its name appears at your cursor.

```
PS C:\projects\api> node ▌        ← press Alt+F
┌──────────────────────────────────────────────────────────────┐
│ app.js       server.js     utils.js     package.json   src\  │
└──────────────────────────────────────────────────────────────┘
PS C:\projects\api> node server.js▌
```

Works in Windows Terminal, the VS Code terminal and the classic console
window, in PowerShell (5.1 and 7), CMD (with [Clink](https://chrisant996.github.io/clink/))
and Git Bash.

## Keys

| Key | Does |
| --- | --- |
| Arrows, Tab | Move |
| Type | Filter the list |
| Backspace / Ctrl+U | Delete a character / clear the filter |
| Home / End, PgUp / PgDn | Jump |
| Enter | Pick |
| Esc / Ctrl+C | Cancel |

Names with spaces or special characters are quoted for you.

## Build and try it

```powershell
cargo build --release
. .\shell\filepick-hook.ps1 -Exe .\target\release\filepick.exe
```

Then type `node ` and press **Alt+F**.

CMD, with Clink installed:

```bat
set FILEPICK_EXE=%CD%\target\release\filepick.exe
clink inject --scripts %CD%\shell
```

Git Bash:

```bash
export FILEPICK_EXE="$PWD/target/release/filepick.exe"
source shell/filepick.bash
```

Plain CMD without Clink has no way to add hotkeys.

## Turn it on for good

With `filepick.exe` and the three scripts in one folder (as the installer
puts them):

```
filepick setup       adds one line to your PowerShell profiles and ~/.bashrc,
                     and registers the Clink script; running it twice is harmless
filepick setup --allow-scripts
                     also lets PowerShell load the hotkey script, if your
                     execution policy blocks it (sets yours to RemoteSigned)
filepick remove      undoes exactly what setup did
filepick doctor      checks everything and says how to fix what's wrong
```

## Settings

`%APPDATA%\filepick\config.toml`:

```toml
hotkey = "Alt+f"            # also e.g. "Ctrl+g", "F2"; avoid Ctrl+Alt (that's AltGr)
extensions = ["js", "ts"]   # only show these file types (default: everything)
```

## Tests

```powershell
cargo test
cd tests\e2e; npm install; npm test
```

`setup.js` checks `setup` / `remove` on throwaway files. `run.js` drives each
installed shell in a hidden console and presses the keys for real (about 15
minutes). See the top of each file.

## Command-line options

```
filepick [--dir PATH] [--out FILE] [--quote none|ps|cmd|sh]
         [--ext js,ts] [--no-dirs] [--all]
filepick setup | remove | doctor
```

Exit codes: `0` picked, `1` cancelled, `2` error.
