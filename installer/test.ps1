# Installs, uninstalls and reinstalls Setup.exe on THIS PC, silently (no
# windows), and checks each step. It changes the real user PATH, PowerShell
# profiles, ~/.bashrc and Clink registration, and leaves altpick installed.
#
#   powershell -ExecutionPolicy Bypass -File installer\test.ps1
#
# Run it from PowerShell, not Git Bash: Git Bash rewrites /VERYSILENT into a
# path, and the installer then opens its normal windows.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$setup = Get-ChildItem (Join-Path $root 'target\installer\altpick-*-Setup.exe') | Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $setup) { throw 'Build it first: installer\build.ps1' }
$app = Join-Path $env:LOCALAPPDATA 'Programs\altpick'
$silent = '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'
$failed = 0

function Check($title, $ok) {
    if ($ok) { "   ok    $title" } else { "   FAIL  $title"; $script:failed++ }
}
function OnPath($dir = $app) { [bool]([Environment]::GetEnvironmentVariable('Path', 'User') -split ';' | Where-Object { $_.TrimEnd('\') -eq $dir }) }
function Marked($file, $marker = '# altpick-setup') { (Test-Path -LiteralPath $file) -and [bool](Select-String -LiteralPath $file -SimpleMatch $marker) }
# AltPick used to be called filepick; installing AltPick must remove it.
$old = Join-Path $env:LOCALAPPDATA 'Programs\filepick'
$hadOld = Test-Path "$old\filepick.exe"
$profiles = foreach ($ps in 'powershell.exe', 'pwsh.exe') {
    if (Get-Command $ps -ErrorAction SilentlyContinue) {
        $env:PSModulePath = $null  # each PowerShell builds its own
        & $ps -NoLogo -NoProfile -Command '$PROFILE.CurrentUserAllHosts'
    }
}

"== Setup.exe on this PC: $($setup.Name) =="
Start-Process $setup.FullName -ArgumentList ($silent + '/TASKS=allowscripts') -Wait
Check 'install: exe and hook scripts in place' ((Test-Path "$app\altpick.exe") -and (Test-Path "$app\altpick-hook.ps1") -and -not (Test-Path "$app\altpick.ps1"))
Check 'install: folder on the user PATH' (OnPath)
foreach ($p in $profiles) { Check "install: hotkey line in $p" (Marked $p) }
$doctor = & "$app\altpick.exe" doctor | Out-String
Check 'install: doctor sees the hotkey lines' ($doctor -notmatch "hotkey line isn't in")
if ($hadOld) { '   (the old filepick was installed before this run)' }
Check 'install: old filepick files and PATH entry gone' ((-not (Test-Path "$old\filepick.exe")) -and -not (OnPath $old))
foreach ($p in $profiles) { Check "install: no old filepick line in $p" (-not (Marked $p '# filepick-setup')) }
Check 'install: no old %APPDATA%\filepick folder' (-not (Test-Path (Join-Path $env:APPDATA 'filepick')))

Start-Process "$app\unins000.exe" -ArgumentList $silent -Wait
Start-Sleep -Seconds 3  # the uninstaller finishes in a copy of itself
Check 'uninstall: files gone' (-not (Test-Path "$app\altpick.exe"))
Check 'uninstall: PATH entry gone' (-not (OnPath))
foreach ($p in $profiles) { Check "uninstall: no hotkey line in $p" (-not (Marked $p)) }

Start-Process $setup.FullName -ArgumentList ($silent + '/TASKS=allowscripts') -Wait
Check 'reinstall: back in place' ((Test-Path "$app\altpick.exe") -and (OnPath))
foreach ($p in $profiles) { Check "reinstall: hotkey line in $p" (Marked $p) }

if ($failed) { "`n$failed FAILED"; exit 1 } else { "`nall passed" }
