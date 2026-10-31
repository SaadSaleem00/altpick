// Git Bash in its own window (mintty). Opens a real mintty window for a few
// seconds and types into it, so it takes keyboard focus: don't type while it
// runs. Not part of `npm test`; run it with `npm run test:mintty`.
//
// mintty is a terminal, not a Windows console. Windows programs started from
// it run behind a hidden console that keeps the palette's alternate-screen
// switch to itself and blanks mintty's real screen. altpick.bash works
// around that; this test checks the fix through mintty's own output log:
// mintty must be switched to its alternate screen around the palette, and
// the picked name must be typed afterwards.
//
//   ALTPICK_HOOK_BASH=<file>   test another bash hook (e.g. an old version)

const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawn, spawnSync } = require('child_process');

const ROOT = path.resolve(__dirname, '..', '..');
const EXE = path.join(ROOT, 'target', 'release', 'altpick.exe');
const HOOK = process.env.ALTPICK_HOOK_BASH || path.join(ROOT, 'shell', 'altpick.bash');
const GIT = path.join(process.env.ProgramFiles || 'C:\\Program Files', 'Git');
const MINTTY = path.join(GIT, 'usr', 'bin', 'mintty.exe');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const fwd = (p) => p.replace(/\\/g, '/');

// Focuses the newest mintty window and types "cd ", Esc f (= Alt+F), then
// later Ctrl+U and "exit". Alt itself doesn't reach mintty through SendKeys.
const DRIVE = `
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class W { [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); }
'@
$p = Get-Process mintty | Sort-Object StartTime -Descending | Select-Object -First 1
[W]::SetForegroundWindow($p.MainWindowHandle) | Out-Null; Start-Sleep -Milliseconds 500
$ws = New-Object -ComObject WScript.Shell
$ws.SendKeys('cd '); Start-Sleep -Milliseconds 400
$ws.SendKeys('{ESC}'); Start-Sleep -Milliseconds 50; $ws.SendKeys('f'); Start-Sleep -Seconds 3
[W]::SetForegroundWindow($p.MainWindowHandle) | Out-Null; Start-Sleep -Milliseconds 300
$ws.SendKeys('^u'); $ws.SendKeys('exit{ENTER}')
`;

async function main() {
  if (!fs.existsSync(MINTTY)) {
    console.log('(Git for Windows mintty not found: skipping)');
    return 0;
  }
  if (!fs.existsSync(EXE)) {
    console.error(`Build first: cargo build --release (missing ${EXE})`);
    return 2;
  }
  console.log('\n== Git Bash in mintty (real window) ==');

  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'altpick-mintty-'));
  const files = path.join(tmp, 'files');
  fs.mkdirSync(files);
  fs.writeFileSync(path.join(files, 'alpha.js'), 'x');
  const log = path.join(tmp, 'mintty.log');
  const rc = path.join(tmp, 'rc');
  // Git's own bashrc gives the usual two-line prompt (user@host MSYSTEM path / $).
  fs.writeFileSync(
    rc,
    [
      '. /etc/bash.bashrc',
      'HISTFILE=/dev/null',
      `cd '${fwd(files)}'`,
      'echo OLD-OUTPUT-LINE',
      `export ALTPICK_EXE='${fwd(EXE)}' ALTPICK_TEST_KEYS=alp,enter`,
      `. '${fwd(HOOK)}'`,
      '',
    ].join('\n'),
  );
  const drive = path.join(tmp, 'drive.ps1');
  fs.writeFileSync(drive, DRIVE);

  let failed = 0;
  try {
    const mintty = spawn(MINTTY, ['-s', '90,20', '-l', fwd(log), '-e', '/usr/bin/bash', '--noprofile', '--rcfile', fwd(rc), '-i'], {
      stdio: 'ignore',
    });
    await sleep(3000);
    spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', drive], { stdio: 'ignore' });
    await Promise.race([new Promise((r) => mintty.on('exit', r)), sleep(10000)]);
    try {
      mintty.kill();
    } catch {}

    const out = fs.existsSync(log) ? fs.readFileSync(log, 'utf8') : '';
    const enter = out.indexOf('\x1b[?1049h');
    const palette = out.indexOf(' altpick ');
    const leave = out.lastIndexOf('\x1b[?1049l');
    const picked = out.lastIndexOf('alpha.js');
    const checks = [
      ['the palette opened', palette >= 0],
      ['mintty switched to its alternate screen before the palette', enter >= 0 && enter < palette],
      ['and back after it, so the old screen returns', leave > palette],
      ['the picked name was typed after that', picked > leave],
    ];
    for (const [title, ok] of checks) {
      if (!ok) failed++;
      console.log(`   ${ok ? 'ok  ' : 'FAIL'}  ${title}`);
    }
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  console.log(failed ? `\n${failed} FAILED` : '\nall passed');
  return failed ? 1 : 0;
}

main().then((code) => process.exit(code));
