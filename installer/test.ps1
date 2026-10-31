# Installs, uninstalls and reinstalls Setup.exe on THIS PC, silently (no
# windows), and checks each step. It changes the real user PATH, PowerShell
# profiles, ~/.bashrc and Clink registration, and leaves filepick installed.
#
#   powershell -ExecutionPolicy Bypass -File installer\test.ps1
#
# Run it from PowerShell, not Git Bash: Git Bash rewrites /VERYSILENT into a
# path, and the installer then opens its normal windows.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$setup = Get-ChildItem (Join-Path $root 'target\installer\filepick-*-Setup.exe') | Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $setup) { throw 'Build it first: installer\build.ps1' }
$app = Join-Path $env:LOCALAPPDATA 'Programs\filepick'
$silent = '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'
$failed = 0

function Check($title, $ok) {
    if ($ok) { "   ok    $title" } else { "   FAIL  $title"; $script:failed++ }
}
function OnPath { [bool]([Environment]::GetEnvironmentVariable('Path', 'User') -split ';' | Where-Object { $_.TrimEnd('\') -eq $app }) }
function Marked($file) { (Test-Path -LiteralPath $file) -and [bool](Select-String -LiteralPath $file -SimpleMatch '# filepick-setup') }
$profiles = foreach ($ps in 'powershell.exe', 'pwsh.exe') {
    if (Get-Command $ps -ErrorAction SilentlyContinue) {
        $env:PSModulePath = $null  # each PowerShell builds its own
        & $ps -NoLogo -NoProfile -Command '$PROFILE.CurrentUserAllHosts'
    }
}

"== Setup.exe on this PC: $($setup.Name) =="
Start-Process $setup.FullName -ArgumentList ($silent + '/TASKS=allowscripts') -Wait
Check 'install: exe and hook scripts in place' ((Test-Path "$app\filepick.exe") -and (Test-Path "$app\filepick-hook.ps1") -and -not (Test-Path "$app\filepick.ps1"))
Check 'install: folder on the user PATH' (OnPath)
foreach ($p in $profiles) { Check "install: hotkey line in $p" (Marked $p) }
$doctor = & "$app\filepick.exe" doctor | Out-String
Check 'install: doctor sees the hotkey lines' ($doctor -notmatch "hotkey line isn't in")

Start-Process "$app\unins000.exe" -ArgumentList $silent -Wait
Start-Sleep -Seconds 3  # the uninstaller finishes in a copy of itself
Check 'uninstall: files gone' (-not (Test-Path "$app\filepick.exe"))
Check 'uninstall: PATH entry gone' (-not (OnPath))
foreach ($p in $profiles) { Check "uninstall: no hotkey line in $p" (-not (Marked $p)) }

Start-Process $setup.FullName -ArgumentList ($silent + '/TASKS=allowscripts') -Wait
Check 'reinstall: back in place' ((Test-Path "$app\filepick.exe") -and (OnPath))
foreach ($p in $profiles) { Check "reinstall: hotkey line in $p" (Marked $p) }

if ($failed) { "`n$failed FAILED"; exit 1 } else { "`nall passed" }
