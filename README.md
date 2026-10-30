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
. .\shell\filepick.ps1 -Exe .\target\release\filepick.exe
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

## Tests

```powershell
cargo test
cd tests\e2e; npm install; node run.js
```

The end-to-end tests drive each installed shell in a hidden console and
press the keys for real. See the top of `tests/e2e/run.js`.

## Command-line options

```
filepick [--dir PATH] [--out FILE] [--quote none|ps|cmd|sh]
         [--ext js,ts] [--no-dirs] [--all]
```

Exit codes: `0` picked, `1` cancelled, `2` error.
