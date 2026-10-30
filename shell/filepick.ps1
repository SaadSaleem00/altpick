<#
.SYNOPSIS
    filepick hotkey for PowerShell (Windows PowerShell 5.1 and PowerShell 7).

.DESCRIPTION
    While typing a command, press the hotkey (Alt+F by default). The filepick
    palette opens with the files in the current folder. Pick one with the
    arrow keys and press Enter: its name is typed at your cursor. Esc cancels.

    Works in Windows Terminal, the VS Code terminal and the classic
    PowerShell window.

.EXAMPLE
    . .\shell\filepick.ps1 -Exe .\target\release\filepick.exe
    Turn the hotkey on in this window (note the leading dot and space).

.EXAMPLE
    . .\shell\filepick.ps1 -Exe .\target\release\filepick.exe -Chord 'Ctrl+Alt+p'
    Use a different hotkey.
#>
[CmdletBinding()]
param(
    # Path to filepick.exe. If left out, looks next to this script, then on PATH.
    [string]$Exe,
    # PSReadLine key chord, e.g. 'Alt+f', 'Ctrl+Alt+p', 'F2'.
    [string]$Chord = 'Alt+f',
    # Don't print the "press Alt+f" line (used when loaded from a profile).
    [switch]$Quiet
)

if (-not (Get-Command Set-PSReadLineKeyHandler -ErrorAction SilentlyContinue)) {
    Write-Warning "filepick: PSReadLine isn't available in this window, so the hotkey can't be set."
    return
}

if (-not $Exe) {
    $nextToScript = Join-Path $PSScriptRoot 'filepick.exe'
    if (Test-Path -LiteralPath $nextToScript -PathType Leaf) {
        $Exe = $nextToScript
    }
    else {
        $found = Get-Command 'filepick.exe' -CommandType Application -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($found) { $Exe = $found.Path }
    }
}
if (-not $Exe -or -not (Test-Path -LiteralPath $Exe -PathType Leaf)) {
    Write-Warning "filepick: can't find filepick.exe. Pass its path with -Exe."
    return
}
$global:FilePickExe = (Resolve-Path -LiteralPath $Exe).ProviderPath

Set-PSReadLineKeyHandler -Chord $Chord `
    -BriefDescription 'filepick' `
    -Description 'Pick a file from the current folder and type its name at the cursor' `
    -ScriptBlock {
        param($key, $arg)

        # No `finally` and no `return` inside `try` here. In the VS Code
        # PowerShell extension's terminal on Windows PowerShell 5.1, entering
        # a `finally` block from a key handler throws a NullReferenceException
        # inside PowerShell itself (ExceptionHandlingOps.SuspendStoppingPipeline),
        # which no try/catch can stop. `catch` blocks are safe there.
        #
        # Plain .NET calls on purpose: System.Diagnostics.Process avoids
        # PowerShell's native-command machinery and leaves the user's
        # $LASTEXITCODE untouched.
        $tmp = $null
        $pick = $null
        try {
            # Only real folders can be listed (not HKLM:\, Env:\ ...).
            $location = $ExecutionContext.SessionState.Path.CurrentLocation
            if ($location.Provider.Name -eq 'FileSystem') {
                # The pick comes back through a temp file (UTF-8), so names in
                # any language survive. The palette draws on the console directly.
                $tmp = [System.IO.Path]::GetTempFileName()

                $start = New-Object System.Diagnostics.ProcessStartInfo
                $start.FileName = $global:FilePickExe
                # The folder is passed as the working directory rather than an
                # argument, so paths like C:\ need no quoting tricks.
                $start.WorkingDirectory = $location.ProviderPath
                $start.Arguments = '--out "' + $tmp + '" --quote ps'
                $start.UseShellExecute = $false

                $process = [System.Diagnostics.Process]::Start($start)
                $process.WaitForExit()
                $exitCode = $process.ExitCode
                $process.Dispose()

                if ($exitCode -eq 0) {
                    $pick = [System.IO.File]::ReadAllText($tmp, [System.Text.Encoding]::UTF8)
                }
            }
        }
        catch {
            # Never break the user's command line because of the palette.
        }

        if ($tmp) {
            try { [System.IO.File]::Delete($tmp) } catch { }
        }
        if ($pick) {
            try { [Microsoft.PowerShell.PSConsoleReadLine]::Insert($pick) } catch { }
        }
    }

if (-not $Quiet) {
    Write-Host "filepick: press $Chord while typing to pick a file." -ForegroundColor DarkGray
}