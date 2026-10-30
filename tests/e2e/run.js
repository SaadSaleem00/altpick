// End-to-end test: drives real PowerShell in a hidden console (ConPTY),
// presses Alt+F, works the palette, and checks what the command line did.
//
//   cd tests\e2e
//   npm install
//   node run.js            (build the release exe first: cargo build --release)
//
// Exit code 0 = all passed.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');
const pty = require('@lydell/node-pty');
const { Terminal } = require('@xterm/headless');
const pses = require('./pses');

const ROOT = path.resolve(__dirname, '..', '..');
const EXE = path.join(ROOT, 'target', 'release', 'filepick.exe');
// FILEPICK_HOOK lets you test another hook file (e.g. an old version).
const HOOK = process.env.FILEPICK_HOOK || path.join(ROOT, 'shell', 'filepick.ps1');
const COLS = 140;
const ROWS = 40;

const ALT_F = '\x1bf';
const ENTER = '\r';
const ESC = '\x1b';
const END = '\x1b[F';

// Test files, including the awkward cases.
const FILES = ['alpha.js', "it's.txt", 'my file.txt', 'اردو.txt'];
const DIRS = ['sub dir'];

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

/** A shell running in a hidden console, with a virtual screen we can read. */
class Shell {
  constructor(file, args, extraEnv = {}, ptyOptions = {}) {
    this.term = new Terminal({ cols: COLS, rows: ROWS, allowProposedApi: true, scrollback: 2000 });
    this.raw = '';
    this.proc = pty.spawn(file, args, {
      name: 'xterm-256color',
      cols: COLS,
      rows: ROWS,
      cwd: os.tmpdir(),
      env: { ...process.env, ...extraEnv },
      ...ptyOptions,
    });
    this.proc.onData((d) => {
      this.raw += d;
      this.term.write(d);
    });
    // Answers the console's questions, e.g. the cursor-position query ESC[6n.
    this.term.onData((d) => this.proc.write(d));
    this.exited = false;
    this.proc.onExit(() => (this.exited = true));
  }

  /** Every line in the buffer (scrollback included), right-trimmed. */
  lines() {
    const b = this.term.buffer.active;
    const out = [];
    for (let i = 0; i < b.length; i++) out.push(b.getLine(i).translateToString(true).trimEnd());
    return out;
  }

  text() {
    return this.lines().join('\n');
  }

  /** Waits until `test(screenText)` is true. Throws with the screen on timeout. */
  async waitFor(what, test, timeout = 15000) {
    const until = Date.now() + timeout;
    for (;;) {
      // Let xterm finish parsing what has arrived.
      await new Promise((r) => this.term.write('', r));
      if (test(this.text())) return;
      if (this.exited) throw new Error(`shell exited while waiting for ${what}\n${this.text()}`);
      if (Date.now() > until) throw new Error(`timed out waiting for ${what}. Screen:\n${this.screen()}`);
      await sleep(50);
    }
  }

  /** Just the visible rows, for error messages. */
  screen() {
    const b = this.term.buffer.active;
    const out = [];
    for (let i = 0; i < ROWS; i++) out.push(b.getLine(b.viewportY + i)?.translateToString(true) ?? '');
    return out.join('\n');
  }

  /** Line the cursor is on. */
  cursorLine() {
    const b = this.term.buffer.active;
    return (b.getLine(b.baseY + b.cursorY)?.translateToString(true) ?? '').trimEnd();
  }

  async type(s) {
    // Send one character at a time, like a person typing; ConPTY can drop
    // or reorder big chunks while PSReadLine is busy.
    for (const ch of s) {
      this.proc.write(ch);
      await sleep(5);
    }
  }

  async key(seq, pause = 150) {
    this.proc.write(seq);
    await sleep(pause);
  }

  /** Runs one command and waits for the next prompt. */
  async run(cmd, timeout) {
    const marker = `__done_${Math.random().toString(36).slice(2)}__`;
    await this.type(`${cmd}; '${marker}'.ToUpper()`);
    await this.key(ENTER, 0);
    await this.waitFor(`"${cmd}" to finish`, (t) => t.includes(marker.toUpperCase()), timeout);
    await this.waitForPrompt();
  }

  async waitForPrompt() {
    await this.waitFor('a prompt', () => /^PS .*> ?$/.test(this.cursorLine()));
  }

  /** Ends the shell and waits until node-pty has let go of it. */
  async kill() {
    // End the whole tree, including anything the shell started.
    try {
      execFileSync('taskkill.exe', ['/PID', String(this.proc.pid), '/T', '/F'], { stdio: 'ignore' });
    } catch {}
    try {
      this.proc.kill();
    } catch {}
    // Exiting node while node-pty is still closing the console can hang node.
    const until = Date.now() + 5000;
    while (!this.exited && Date.now() < until) await sleep(100);
    await sleep(500);
  }
}

function makeTestFolder() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'filepick e2e '));
  for (const f of FILES) fs.writeFileSync(path.join(dir, f), 'x');
  for (const d of DIRS) fs.mkdirSync(path.join(dir, d));
  return dir;
}

function psQuote(s) {
  return `'${s.replace(/'/g, "''")}'`;
}

/** The errors PowerShell recorded, in full, for diagnosing failures. */
async function dumpErrors(sh, dir) {
  const file = path.join(dir, '..', `filepick-e2e-errors-${process.pid}.txt`);
  await sh.run(
    `$error | ForEach-Object { '----'; $_ | Format-List * -Force | Out-String -Width 300; ` +
      `if ($_.Exception.InnerException) { 'INNER: ' + $_.Exception.InnerException.ToString() } } | ` +
      `Out-File -Width 300 -Encoding utf8 ${psQuote(file)}`,
  );
  try {
    return fs.readFileSync(file, 'utf8');
  } finally {
    fs.rmSync(file, { force: true });
  }
}

const HANDLER_CRASH = /exception occurred in custom key handler/i;

/**
 * One test case. `drive` works the palette after Alt+F. `expect` is the exact
 * line the command must print.
 *
 * `palette: false` is for cases where Alt+F must do nothing visible.
 * `setup` / `teardown` are PowerShell commands run around the case.
 * `errorsOk` skips the errors=0 check (for cases that cause an error on purpose).
 */
function pickCase(name, { cwd, prefix = 'Write-Output ', drive, expect, insertedCheck, ...more }) {
  return { name, cwd, prefix, drive, expect, insertedCheck, palette: true, ...more };
}

async function runCase(sh, c, dir) {
  const cwd = c.cwd ?? dir;
  const start = `=== case ${Math.random().toString(36).slice(2)} ===`;
  const setup = c.setup ? `${c.setup}; ` : '';
  await sh.run(`Set-Location -LiteralPath ${psQuote(cwd)}; ${setup}$error.Clear(); Clear-Host; '${start}'`);

  await sh.type(c.prefix);
  await sh.key(ALT_F, 0);
  if (c.palette) {
    await sh.waitFor('the palette', (t) => t.includes(' filepick ') && t.includes('Esc cancel'));
    await c.drive(sh);
    await sh.waitFor('the palette to close', (t) => !t.includes('Esc cancel'));
  } else {
    // Give the handler time to run (and to fail, if it's going to).
    await sleep(1500);
  }

  // Back on the prompt with the line updated.
  await sh.waitFor('the line to be updated', () => c.insertedCheck(sh.cursorLine()));
  await sleep(200);

  await sh.key(ENTER, 0);
  await sh.waitForPrompt();
  await sh.run(`"errors=" + $error.Count`);
  if (c.teardown) await sh.run(c.teardown);

  const all = sh.lines();
  const out = all.slice(all.lastIndexOf(start));
  const problems = [];
  if (!out.some((l) => l === c.expect)) {
    const near = out.filter((l) => l.includes(c.expect)).map((l) => JSON.stringify(l));
    problems.push(`expected an output line "${c.expect}"; similar lines: ${near.join(', ')}`);
  }
  if (!c.errorsOk && !out.some((l) => l === 'errors=0')) problems.push('expected errors=0');
  if (out.some((l) => HANDLER_CRASH.test(l))) problems.push('PSReadLine reported a key handler exception');
  return problems;
}

async function suite(label, file, args, extraEnv, ptyOptions, start) {
  console.log(`\n== ${label} ==`);
  const dir = makeTestFolder();
  const sh = new Shell(file, args, extraEnv, ptyOptions);
  let failed = 0;
  let cleanup = () => {};
  try {
    if (start) cleanup = await start(sh);
    await sh.waitForPrompt();
    await sh.run(`$PSVersionTable.PSVersion.ToString() + ' / PSReadLine ' + (Get-Module PSReadLine).Version`);
    const ver = sh.lines().find((l) => /PSReadLine \d/.test(l) && !l.startsWith('PS '));
    console.log(`   ${ver}`);
    await sh.run(`. ${psQuote(HOOK)} -Exe ${psQuote(EXE)}`);

    const cases = [
      pickCase('filter and pick a plain name', {
        drive: async (s) => {
          await s.type('alp');
          await s.key(ENTER);
        },
        insertedCheck: (l) => l.endsWith('Write-Output alpha.js'),
        expect: 'alpha.js',
      }),
      pickCase('name with a space is quoted', {
        drive: async (s) => {
          await s.type('my f');
          await s.key(ENTER);
        },
        insertedCheck: (l) => l.endsWith("Write-Output 'my file.txt'"),
        expect: 'my file.txt',
      }),
      pickCase('name with an apostrophe is quoted', {
        drive: async (s) => {
          await s.type('it');
          await s.key(ENTER);
        },
        insertedCheck: (l) => l.endsWith("Write-Output 'it''s.txt'"),
        expect: "it's.txt",
      }),
      pickCase('Urdu name', {
        drive: async (s) => {
          await s.type('اردو');
          await s.key(ENTER);
        },
        insertedCheck: (l) => l.includes('Write-Output') && l.includes('.txt'),
        expect: 'اردو.txt',
      }),
      pickCase('End key picks the folder (last item)', {
        drive: async (s) => {
          await s.key(END);
          await s.key(ENTER);
        },
        insertedCheck: (l) => l.endsWith("Write-Output 'sub dir'"),
        expect: 'sub dir',
      }),
      pickCase('Esc leaves the line alone', {
        prefix: 'Write-Output keep',
        drive: async (s) => {
          await s.key(ESC, 600);
        },
        insertedCheck: (l) => l.endsWith('Write-Output keep'),
        expect: 'keep',
      }),
      pickCase('works in C:\\', {
        cwd: 'C:\\',
        drive: async (s) => {
          await s.type('windows');
          await s.key(ENTER);
        },
        insertedCheck: (l) => l.endsWith('Write-Output Windows'),
        expect: 'Windows',
      }),
      pickCase('does nothing outside the file system (HKCU:\\)', {
        cwd: 'HKCU:\\',
        prefix: 'Write-Output reg',
        palette: false,
        insertedCheck: (l) => l.endsWith('Write-Output reg'),
        expect: 'reg',
      }),
      // The hook's error path: the palette can't start. This goes through a
      // `catch` block, which must not crash the extension terminal either.
      pickCase('a palette that fails to start leaves the line alone', {
        prefix: 'Write-Output safe',
        setup: `$saved = $global:FilePickExe; $global:FilePickExe = 'C:\\no\\such\\filepick.exe'`,
        teardown: '$global:FilePickExe = $saved',
        palette: false,
        errorsOk: true,
        insertedCheck: (l) => l.endsWith('Write-Output safe'),
        expect: 'safe',
      }),
    ];

    for (const c of cases) {
      let problems;
      try {
        problems = await runCase(sh, c, dir);
      } catch (e) {
        problems = [e.message];
        // Get back to a clean prompt for the next case.
        await sh.key(ESC, 300);
        await sh.key('\x03', 500);
      }
      if (problems.length === 0) {
        console.log(`   ok    ${c.name}`);
      } else {
        failed++;
        console.log(`   FAIL  ${c.name}\n         ${problems.join('\n         ')}`);
        try {
          const errs = await dumpErrors(sh, dir);
          if (errs.trim()) console.log(`--- $error ---\n${errs}`);
        } catch {}
        console.log(`--- screen ---\n${sh.screen()}\n--------------`);
      }
    }
  } finally {
    await sh.kill();
    try {
      cleanup();
      fs.rmSync(dir, { recursive: true, force: true });
    } catch {
      // A file can stay locked for a moment after the shell exits; it's in %TEMP% anyway.
    }
  }
  return failed;
}

function findPwsh() {
  try {
    return execFileSync('where.exe', ['pwsh.exe'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).split(/\r?\n/)[0].trim() || null;
  } catch {
    return null;
  }
}

/** VS Code's PowerShell shell-integration script, if VS Code is installed. */
function findVsCodeShellIntegration() {
  const tail = ['resources', 'app', 'out', 'vs', 'workbench', 'contrib', 'terminal', 'common', 'scripts'];
  const roots = [
    path.join(process.env.LOCALAPPDATA || '', 'Programs', 'Microsoft VS Code'),
    path.join(process.env.ProgramFiles || 'C:\Program Files', 'Microsoft VS Code'),
  ];
  for (const root of roots) {
    if (!fs.existsSync(root)) continue;
    // Newer installs keep each version in a hash-named subfolder.
    const bases = [root, ...fs.readdirSync(root).map((d) => path.join(root, d))];
    for (const base of bases) {
      const f = path.join(base, ...tail, 'shellIntegration.ps1');
      if (fs.existsSync(f)) return f;
    }
  }
  return null;
}

// FILEPICK_PROFILE=1 loads the user's own PowerShell profile too, like a real
// terminal does. Off by default so the tests don't depend on this machine.
const PROFILE_ARGS = process.env.FILEPICK_PROFILE === '1' ? [] : ['-NoProfile'];

// Keeps the test commands out of the user's real PSReadLine history.
const NO_HISTORY = 'Set-PSReadLineOption -HistorySaveStyle SaveNothing';

/** Starts the shell plainly (no profile), with history off. */
function plainArgs() {
  return ['-NoLogo', ...PROFILE_ARGS, '-NoExit', '-Command', NO_HISTORY];
}

/** Starts the shell the way the VS Code terminal does, with history off. */
function vsCodeArgs(script) {
  return ['-NoLogo', ...PROFILE_ARGS, '-NoExit', '-Command', `${NO_HISTORY}; try { . ${psQuote(script)} } catch {}`];
}

const VSCODE_ENV = { TERM_PROGRAM: 'vscode', VSCODE_NONCE: 'e2e-nonce', VSCODE_STABLE: '1' };

async function main() {
  if (!fs.existsSync(EXE)) {
    console.error(`Build first: cargo build --release (missing ${EXE})`);
    process.exit(2);
  }
  const suites = [['Windows PowerShell 5.1', 'powershell.exe', plainArgs()]];
  const vscode = findVsCodeShellIntegration();
  const pwsh = findPwsh();
  if (vscode) {
    suites.push([
      'Windows PowerShell 5.1 + VS Code shell integration',
      'powershell.exe',
      vsCodeArgs(vscode),
      VSCODE_ENV,
    ]);
    // Like VS Code itself: a newer bundled conpty.dll instead of the one in Windows.
    suites.push([
      'Windows PowerShell 5.1 + VS Code shell integration + bundled conpty',
      'powershell.exe',
      vsCodeArgs(vscode),
      VSCODE_ENV,
      { useConptyDll: true },
    ]);
  } else console.log('(VS Code not found: skipping the VS Code shell-integration runs)');
  if (pwsh) {
    suites.push(['PowerShell 7', pwsh, plainArgs()]);
    if (vscode) {
      suites.push([
        'PowerShell 7 + VS Code shell integration',
        pwsh,
        vsCodeArgs(vscode),
        VSCODE_ENV,
      ]);
    }
  } else console.log('(pwsh not installed: skipping PowerShell 7)');

  // The VS Code PowerShell extension's own terminal (a different host).
  const ext = pses.findExtension();
  const extensionSuite = (label, exe) => {
    const l = pses.launch(ext);
    return [
      label,
      exe,
      l.args,
      l.env,
      { useConptyDll: true },
      async (sh) => {
        const sock = await pses.connect(l.sessionFile);
        // The extension runs its own startup command, so history is turned off
        // as the first typed line. PSReadLine 2.2+ never writes lines that
        // mention "token" to the history file, so this line stays out too.
        await sh.waitForPrompt();
        await sh.run(`${NO_HISTORY} <# token #>`);
        return () => {
          sock.destroy();
          fs.rmSync(l.work, { recursive: true, force: true });
        };
      },
    ];
  };
  if (ext) {
    suites.push(extensionSuite('Windows PowerShell 5.1 in the PowerShell extension terminal', 'powershell.exe'));
    // With pwsh installed, the extension uses it by default.
    if (pwsh) suites.push(extensionSuite('PowerShell 7 in the PowerShell extension terminal', pwsh));
  } else console.log('(VS Code PowerShell extension not found: skipping its terminal)');

  // FILEPICK_SUITE=vscode runs only the suites whose name contains "vscode".
  const only = (process.env.FILEPICK_SUITE || '').toLowerCase();
  let failed = 0;
  let ran = 0;
  for (const [label, file, args, env, ptyOptions, start] of suites) {
    if (only && !label.toLowerCase().replace(/ /g, '').includes(only)) continue;
    ran++;
    failed += await suite(label, file, args, env, ptyOptions, start);
  }
  if (ran === 0) {
    const names = suites.map(([label]) => `  ${label.toLowerCase().replace(/ /g, '')}`).join('\n');
    console.error(`FILEPICK_SUITE=${only} matches no suite. Suites:\n${names}`);
    process.exit(2);
  }

  console.log(failed ? `\n${failed} FAILED` : '\nall passed');
  process.exit(failed ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
