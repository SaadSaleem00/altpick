# Builds Setup.exe:  powershell -ExecutionPolicy Bypass -File installer\build.ps1
# Output: target\installer\altpick-<version>-Setup.exe
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 not found. Install it: winget install JRSoftware.InnoSetup --scope user' }

$version = (Select-String -LiteralPath (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value

Push-Location $root
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    & $iscc "/DAppVersion=$version" "/DSourceDir=$root" (Join-Path $PSScriptRoot 'altpick.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }
}
finally { Pop-Location }
Get-Item (Join-Path $root "target\installer\altpick-$version-Setup.exe")
