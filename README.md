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

Works in Windows Terminal, the VS Code terminal and the classic PowerShell
window (Windows PowerShell 5.1 and PowerShell 7).

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

## Command-line options

```
filepick [--dir PATH] [--out FILE] [--quote none|ps|cmd|sh]
         [--ext js,ts] [--no-dirs] [--all]
```

Exit codes: `0` picked, `1` cancelled, `2` error.
