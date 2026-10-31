// End-to-end tests: drive real shells in a hidden console (ConPTY), press
// Alt+F, work the palette, and check what the command line did.
//
//   cd tests\e2e
//   npm install
//   node run.js            (build the release exe first: cargo build --release)
//
// Shells: Windows PowerShell 5.1, PowerShell 7 (if installed), both also the
// way the VS Code terminal and the VS Code PowerShell extension start them,
// CMD with Clink (if Clink is found) and Git Bash (if installed).
//
// Settings (environment variables):
//   FILEPICK_SUITE=<text>   only run suites whose name (lower case, no
//                           spaces) contains <text>, e.g. "cmd" or "bash"
//   FILEPICK_HOOK=<file>    test another PowerShell hook file
//   FILEPICK_PROFILE=1      also load the user's own PowerShell profile
//
// The tests never write to the user's shell history.
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
const SHELL_DIR = path.join(ROOT, 'shell');
const PS_HOOK = process.env.FILEPICK_HOOK || path.join(SHELL_DIR, 'filepick-hook.ps1');
const BASH_HOOK = path.join(SHELL_DIR, 'filepick.bash');
const COLS = 140;
const ROWS = 40;

const ALT_F = '\x1bf';
const ENTER = '\r';
const ESC = '\x1b';
const END = '\x1b[F';

// Test files, including the awkward cases.
const FILES = ['alpha.js', "it's.txt", 'my file.txt', 'اردو.txt'];
const DIRS = ['sub dir'];

/** A copy of env without PSModulePath (see the Shell constructor). */
function withoutModulePath(env) {
  const out = { ...env };
  for (const k of Object.keys(out)) if (k.toLowerCase() === 'psmodulepath') delete out[k];
  return out;
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function psQuote(s) {
  return `'${s.replace(/'/g, "''")}'`;
}

function shQuote(s) {
  return `'${s.replace(/'/g, "'\\''")}'`;
}

/** Same rule as src/quote.rs: letters, digits and . _ - + ~ / \ : need no quotes. */
function needsQuotes(name) {
  return name === '' || !/^[\p{L}\p{N}._\-+~/\\:]+$/u.test(name);
}

// ---------------------------------------------------------------------------
// Shell kinds: how each shell's prompt looks and how to talk to it.
//
// `marker(m)` is a command that prints m without m appearing in the typed
// line itself, so "the output contains m" means the command really ran.
// `show(prefix, suffix)` wraps the picked name in a command that prints
// "[name]" only if that file or folder exists, which proves the quoting works.
// ---------------------------------------------------------------------------

const POWERSHELL = {
  prompt: /^PS .*> ?$/,
  sep: '; ',
  marker: (m) => `'${m.slice(0, 4)}' + '${m.slice(4)}'`,
  cd: (dir) => `Set-Location -LiteralPath ${psQuote(dir)}; $error.Clear()`,
  echo: (word) => `Write-Output ${word}`,
  say: (text) => `Write-Output ${psQuote(text)}`,
  show: { prefix: 'Get-Item -LiteralPath ', suffix: ' | ForEach-Object { "[$($_.Name)]" }' },
  quote: (n) => (needsQuotes(n) ? psQuote(n) : n),
  version: `$PSVersionTable.PSVersion.ToString() + ' / PSReadLine ' + (Get-Module PSReadLine).Version`,
  load: () => `. ${psQuote(PS_HOOK)} -Exe ${psQuote(EXE)}`,
  errorsCheck: `"errors=" + $error.Count`,
  crash: /exception occurred in custom key handler/i,
  badExe: {
    setup: `$saved = $global:FilePickExe; $global:FilePickExe = 'C:\\no\\such\\filepick.exe'`,
    teardown: '$global:FilePickExe = $saved',
  },
  hasProviders: true,
};

const CMD = {
  prompt: /^[A-Za-z]:\\.*>$/,
  sep: ' & ',
  marker: (m) => `echo ${m.slice(0, 4)}^${m.slice(4)}`,
  cd: (dir) => `cd /d "${dir}"`,
  echo: (word) => `echo ${word}`,
  say: (text) => `echo ${text}`,
  show: { prefix: 'for %f in (', suffix: ') do @if exist "%~f" echo [%~f]' },
  quote: (n) => (needsQuotes(n) ? `"${n}"` : n),
  version: 'ver',
  load: null, // Clink loads the script at startup (see cmdArgs).
  crash: /filepick\.lua:\d+/,
  badExe: {
    setup: 'set "FILEPICK_SAVED=%FILEPICK_EXE%" & set "FILEPICK_EXE=C:\\no\\such\\filepick.exe"',
    teardown: 'set "FILEPICK_EXE=%FILEPICK_SAVED%"',
  },
};

const BASH = {
  prompt: /^BASH\$ ?$/,
  sep: '; ',
  marker: (m) => `echo ${m.slice(0, 4)}''${m.slice(4)}`,
  cd: (dir) => `cd ${shQuote(dir.replace(/\\/g, '/'))}`,
  echo: (word) => `echo ${word}`,
  say: (text) => `echo ${shQuote(text)}`,
  show: { prefix: 'for f in ', suffix: `; do [ -e "$f" ] && printf '[%s]\\n' "$f"; done` },
  quote: (n) => (needsQuotes(n) ? shQuote(n) : n),
  version: 'echo "bash $BASH_VERSION"',
  load: () => `source ${shQuote(BASH_HOOK.replace(/\\/g, '/'))}`,
  crash: /__filepick|filepick\.bash/,
  badExe: {
    setup: 'FILEPICK_SAVED=$FILEPICK_EXE; FILEPICK_EXE=/c/no/such/filepick.exe',
    teardown: 'FILEPICK_EXE=$FILEPICK_SAVED',
  },
};

/** A shell running in a hidden console, with a virtual screen we can read. */
class Shell {
  constructor(kind, file, args, extraEnv = {}, ptyOptions = {}) {
    this.kind = kind;
    this.term = new Terminal({ cols: COLS, rows: ROWS, allowProposedApi: true, scrollback: 3000 });
    this.proc = pty.spawn(file, args, {
      name: 'xterm-256color',
      cols: COLS,
      rows: ROWS,
      cwd: os.tmpdir(),
      // PSModulePath from a PowerShell 7 parent would make Windows PowerShell load 7's modules.
      env: withoutModulePath({ ...process.env, ...extraEnv }),
      ...ptyOptions,
    });
    this.proc.onData((d) => this.term.write(d));
    // Answers the console's questions, e.g. the cursor-position query ESC[6n.
    this.term.onData((d) => this.send(d));
    // node-pty gives its input pipe no error handler, so one failed write
    // ("EAGAIN": pipe full) would crash node. send() retries those instead.
    this.input = this.proc._agent?.inSocket;
    this.input?.on('error', () => {});
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

  /** Writes to the shell's input, waiting until it's taken and retrying if the pipe is full. */
  async send(data) {
    if (!this.input) {
      this.proc.write(data);
      return;
    }
    for (let attempt = 0; ; attempt++) {
      const err = await new Promise((r) => this.input.write(data, r));
      if (!err) return;
      if (err.code !== 'EAGAIN' || attempt >= 50 || this.exited) throw err;
      await sleep(20);
    }
  }

  async type(s) {
    // Send one character at a time, like a person typing; ConPTY can drop
    // or reorder big chunks while the line editor is busy.
    for (const ch of s) {
      await this.send(ch);
      await sleep(5);
    }
  }

  async key(seq, pause = 150) {
    await this.send(seq);
    await sleep(pause);
  }

  /** Runs one command and waits for the next prompt. */
  async run(cmd, timeout) {
    const m = `__done_${Math.random().toString(36).slice(2)}__`;
    await this.type(`${cmd}${this.kind.sep}${this.kind.marker(m)}`);
    await this.key(ENTER, 0);
    await this.waitFor(`"${cmd}" to finish`, (t) => t.includes(m), timeout);
    await this.waitForPrompt();
  }

  async waitForPrompt(timeout) {
    await this.waitFor('a prompt', () => this.kind.prompt.test(this.cursorLine()), timeout);
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

/** The errors PowerShell recorded, in full, for diagnosing failures. */
async function dumpPowerShellErrors(sh, dir) {
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

// ---------------------------------------------------------------------------
// Test cases
// ---------------------------------------------------------------------------

/**
 * Pick `name` by typing `filter` (or pressing `keys`) in the palette. The
 * line must then end with the quoted name, and running it must print "[name]".
 */
function pickCase(kind, title, { name, filter, keys, cwd }) {
  return {
    title,
    cwd,
    prefix: kind.show.prefix,
    suffix: kind.show.suffix,
    drive: async (s) => {
      if (filter) await s.type(filter);
      if (keys) await s.key(keys);
      await s.key(ENTER);
    },
    inserted: kind.show.prefix + kind.quote(name),
    expect: `[${name}]`,
    palette: true,
  };
}

/** Alt+F must leave `line` exactly as it was. */
function unchangedCase(title, line, expect, more) {
  return { title, prefix: line, suffix: '', inserted: line, expect, palette: true, ...more };
}

function casesFor(kind) {
  const cases = [
    pickCase(kind, 'filter and pick a plain name', { name: 'alpha.js', filter: 'alp' }),
    pickCase(kind, 'name with a space is quoted', { name: 'my file.txt', filter: 'my f' }),
    pickCase(kind, 'name with an apostrophe is quoted', { name: "it's.txt", filter: 'it' }),
    pickCase(kind, 'Urdu name', { name: 'اردو.txt', filter: 'اردو' }),
    pickCase(kind, 'End key picks the folder (last item)', { name: 'sub dir', keys: END }),
    pickCase(kind, 'works in C:\\', { name: 'Windows', filter: 'windows', cwd: 'C:\\' }),
    unchangedCase('Esc leaves the line alone', kind.echo('keep'), 'keep', {
      drive: (s) => s.key(ESC, 600),
    }),
  ];
  if (kind.hasProviders) {
    cases.push(
      unchangedCase('does nothing outside the file system (HKCU:\\)', kind.echo('reg'), 'reg', {
        cwd: 'HKCU:\\',
        palette: false,
      }),
    );
  }
  // The hook's error path: the palette can't start. (Not for installed
  // layouts, where the exe next to the script is always found.)
  if (kind.badExe) {
    cases.push(
      unchangedCase('a palette that fails to start leaves the line alone', kind.echo('safe'), 'safe', {
        ...kind.badExe,
        palette: false,
        errorsOk: true,
      }),
    );
  }
  return cases;
}

/**
 * A pattern for how `text` looks at the end of the line on screen. PowerShell
 * draws its command line in the console's old code page, so letters it can't
 * show there (Urdu, for one) appear as "?". The real text is still right: each
 * case also checks what the command printed.
 */
function shownAs(text) {
  const parts = [...text].map((ch) =>
    ch.charCodeAt(0) < 128 ? ch.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') : `(?:${ch}|\\?)`,
  );
  return new RegExp(`${parts.join('')}$`, 'u');
}

async function runCase(sh, c, dir) {
  const k = sh.kind;
  const start = `=== case ${Math.random().toString(36).slice(2)} ===`;
  const setup = c.setup ? `${k.sep}${c.setup}` : '';
  await sh.run(`${k.cd(c.cwd ?? dir)}${setup}${k.sep}${k.say(start)}`);

  await sh.type(c.prefix);
  await sh.key(ALT_F, 0);
  if (c.palette) {
    await sh.waitFor('the palette', (t) => t.includes(' filepick ') && t.includes('Esc cancel'));
    await c.drive(sh);
    await sh.waitFor('the palette to close', (t) => !t.includes('Esc cancel'));
  } else {
    // Give the hook time to run (and to fail, if it's going to).
    await sleep(1500);
  }

  // Back on the prompt with the line updated.
  await sh.waitFor(`the line to end with: ${c.inserted}`, () => shownAs(c.inserted).test(sh.cursorLine()));
  await sleep(200);
  await sh.type(c.suffix);
  await sh.key(ENTER, 0);
  await sh.waitForPrompt();
  if (k.errorsCheck) await sh.run(k.errorsCheck);
  if (c.teardown) await sh.run(c.teardown);

  const all = sh.lines();
  const out = all.slice(all.lastIndexOf(start));
  const problems = [];
  if (!out.some((l) => l === c.expect)) {
    const near = out.filter((l) => l.includes(c.expect)).map((l) => JSON.stringify(l));
    problems.push(`expected an output line "${c.expect}"; similar lines: ${near.join(', ')}`);
  }
  if (k.errorsCheck && !c.errorsOk && !out.some((l) => l === 'errors=0')) problems.push('expected errors=0');
  if (out.some((l) => k.crash.test(l))) problems.push('the hook reported an error on screen');
  return problems;
}

async function suite(s) {
  console.log(`\n== ${s.label} ==`);
  const dir = makeTestFolder();
  const sh = new Shell(s.kind, s.file, s.args, s.env, s.ptyOptions);
  let failed = 0;
  let cleanup = () => {};
  try {
    if (s.start) cleanup = await s.start(sh);
    await sh.waitForPrompt(30000);
    await sh.run(s.kind.version);
    const ver = sh
      .lines()
      .filter((l) => l && !s.kind.prompt.test(l) && !l.includes('__done_') && !/^PS |^[A-Z]:\\.*>|^BASH\$/.test(l))
      .pop();
    console.log(`   ${ver}`);
    if (s.kind.load) await sh.run(s.kind.load());

    for (const c of casesFor(s.kind)) {
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
        console.log(`   ok    ${c.title}`);
      } else {
        failed++;
        console.log(`   FAIL  ${c.title}\n         ${problems.join('\n         ')}`);
        if (s.kind.errorsCheck) {
          try {
            const errs = await dumpPowerShellErrors(sh, dir);
            if (errs.trim()) console.log(`--- $error ---\n${errs}`);
          } catch {}
        }
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

// ---------------------------------------------------------------------------
// Finding the shells and starting them the way real terminals do
// ---------------------------------------------------------------------------

function which(exe) {
  try {
    const out = execFileSync('where.exe', [exe], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
    return out.split(/\r?\n/)[0].trim() || null;
  } catch {
    return null;
  }
}

function firstExisting(paths) {
  return paths.find((p) => p && fs.existsSync(p)) || null;
}

/** VS Code's PowerShell shell-integration script, if VS Code is installed. */
function findVsCodeShellIntegration() {
  const tail = ['resources', 'app', 'out', 'vs', 'workbench', 'contrib', 'terminal', 'common', 'scripts'];
  const roots = [
    path.join(process.env.LOCALAPPDATA || '', 'Programs', 'Microsoft VS Code'),
    path.join(process.env.ProgramFiles || 'C:\\Program Files', 'Microsoft VS Code'),
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

function findClink() {
  return firstExisting([
    path.join(process.env.LOCALAPPDATA || '', 'Programs', 'clink', 'clink_x64.exe'),
    path.join(process.env['ProgramFiles(x86)'] || '', 'clink', 'clink_x64.exe'),
    path.join(process.env.ProgramFiles || '', 'clink', 'clink_x64.exe'),
  ]);
}

function findGitBash() {
  return firstExisting([
    path.join(process.env.ProgramFiles || 'C:\\Program Files', 'Git', 'bin', 'bash.exe'),
    path.join(process.env.LOCALAPPDATA || '', 'Programs', 'Git', 'bin', 'bash.exe'),
  ]);
}

// FILEPICK_PROFILE=1 loads the user's own PowerShell profile too, like a real
// terminal does. Off by default so the tests don't depend on this machine.
const PROFILE_ARGS = process.env.FILEPICK_PROFILE === '1' ? [] : ['-NoProfile'];

// Keeps the test commands out of the user's real PSReadLine history.
const NO_HISTORY = 'Set-PSReadLineOption -HistorySaveStyle SaveNothing';

/** Starts PowerShell plainly, with history off. */
function plainArgs() {
  return ['-NoLogo', ...PROFILE_ARGS, '-NoExit', '-Command', NO_HISTORY];
}

/** Starts PowerShell the way the VS Code terminal does, with history off. */
function vsCodeArgs(script) {
  return ['-NoLogo', ...PROFILE_ARGS, '-NoExit', '-Command', `${NO_HISTORY}; try { . ${psQuote(script)} } catch {}`];
}

const VSCODE_ENV = { TERM_PROGRAM: 'vscode', VSCODE_NONCE: 'e2e-nonce', VSCODE_STABLE: '1' };

/** The PowerShell extension's own terminal (PowerShell Editor Services). */
function extensionSuite(ext, label, exe) {
  const l = pses.launch(ext);
  return {
    label,
    kind: POWERSHELL,
    file: exe,
    args: l.args,
    env: l.env,
    ptyOptions: { useConptyDll: true },
    start: async (sh) => {
      const sock = await pses.connect(l.sessionFile);
      // The extension runs its own startup command, so history is turned off
      // as the first typed line. PSReadLine 2.2+ never writes lines that
      // mention "token" to the history file, so this line stays out too.
      await sh.waitForPrompt(30000);
      await sh.run(`${NO_HISTORY} <# token #>`);
      return () => {
        sock.destroy();
        fs.rmSync(l.work, { recursive: true, force: true });
      };
    },
  };
}

/** CMD with Clink injected for this window only, using a throwaway Clink profile. */
function cmdSuite(clink, label, ptyOptions = {}) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'filepick-clink-'));
  const inject = `"${clink}" inject -q --profile "${profile}" --scripts "${SHELL_DIR}"`;
  return {
    label,
    kind: CMD,
    file: 'cmd.exe',
    // A raw command line: node-pty would otherwise escape the quotes. /k strips
    // the outer pair of quotes. /d skips any AutoRun commands.
    args: `/d /k "${inject}"`,
    env: { FILEPICK_EXE: EXE },
    ptyOptions,
    start: async () => () => fs.rmSync(profile, { recursive: true, force: true }),
  };
}

function bashSuite(bash, label) {
  return {
    label,
    kind: BASH,
    file: bash,
    args: ['--noprofile', '--norc', '-i'],
    // No history file; a prompt the tests can recognize.
    env: { FILEPICK_EXE: EXE, HISTFILE: '/dev/null', PS1: 'BASH$ ', CHERE_INVOKING: '1' },
  };
}

/**
 * A throwaway copy of the installed layout (exe and scripts in one folder),
 * set up with `filepick setup` into a fake profile and .bashrc. Shells that
 * load only those files show the hotkey works the way an install does: no
 * -Exe and no FILEPICK_EXE, just the line setup wrote.
 */
function makeInstall() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'filepick-install-'));
  const dir = path.join(root, 'Programs', 'filepick');
  fs.mkdirSync(dir, { recursive: true });
  fs.copyFileSync(EXE, path.join(dir, 'filepick.exe'));
  for (const f of ['filepick-hook.ps1', 'filepick.lua', 'filepick.bash']) fs.copyFileSync(path.join(SHELL_DIR, f), path.join(dir, f));
  const profile = path.join(root, 'profile.ps1');
  const bashrc = path.join(root, '.bashrc');
  // The user's own line. It sets the test prompt, because Git's
  // /etc/bash.bashrc runs first and sets its own.
  fs.writeFileSync(bashrc, "PS1='BASH$ '\n");
  const env = { FILEPICK_CONFIG_DIR: path.join(root, 'config') };
  execFileSync(path.join(dir, 'filepick.exe'), ['setup', '--profile', profile, '--bashrc', bashrc, '--no-clink'], {
    env: { ...process.env, ...env },
    stdio: 'ignore',
  });
  return { profile, bashrc, env, cleanup: () => fs.rmSync(root, { recursive: true, force: true }) };
}

async function main() {
  if (!fs.existsSync(EXE)) {
    console.error(`Build first: cargo build --release (missing ${EXE})`);
    process.exit(2);
  }
  const install = makeInstall();
  process.on('exit', install.cleanup);

  const suites = [{ label: 'Windows PowerShell 5.1', kind: POWERSHELL, file: 'powershell.exe', args: plainArgs() }];
  const vscode = findVsCodeShellIntegration();
  const pwsh = which('pwsh.exe');
  const ext = pses.findExtension();
  const clink = findClink();
  const bash = findGitBash();

  if (vscode) {
    const vs = { kind: POWERSHELL, file: 'powershell.exe', args: vsCodeArgs(vscode), env: VSCODE_ENV };
    suites.push({ ...vs, label: 'Windows PowerShell 5.1 + VS Code shell integration' });
    // Like VS Code itself: a newer bundled conpty.dll instead of the one in Windows.
    suites.push({
      ...vs,
      label: 'Windows PowerShell 5.1 + VS Code shell integration + bundled conpty',
      ptyOptions: { useConptyDll: true },
    });
  } else console.log('(VS Code not found: skipping the VS Code shell-integration runs)');

  if (pwsh) {
    suites.push({ label: 'PowerShell 7', kind: POWERSHELL, file: pwsh, args: plainArgs() });
    if (vscode) {
      suites.push({
        label: 'PowerShell 7 + VS Code shell integration',
        kind: POWERSHELL,
        file: pwsh,
        args: vsCodeArgs(vscode),
        env: VSCODE_ENV,
      });
    }
  } else console.log('(pwsh not installed: skipping PowerShell 7)');

  if (ext) {
    suites.push(extensionSuite(ext, 'Windows PowerShell 5.1 in the PowerShell extension terminal', 'powershell.exe'));
    // With pwsh installed, the extension uses it by default.
    if (pwsh) suites.push(extensionSuite(ext, 'PowerShell 7 in the PowerShell extension terminal', pwsh));
  } else console.log('(VS Code PowerShell extension not found: skipping its terminal)');

  if (clink) {
    suites.push(cmdSuite(clink, 'CMD + Clink'));
    suites.push(cmdSuite(clink, 'CMD + Clink + bundled conpty', { useConptyDll: true }));
  } else console.log('(Clink not found: skipping CMD)');

  if (bash) suites.push(bashSuite(bash, 'Git Bash'));
  else console.log('(Git Bash not found: skipping it)');

  // As installed by `filepick setup` (see makeInstall).
  suites.push({
    label: 'Windows PowerShell 5.1 after filepick setup',
    kind: { ...POWERSHELL, load: () => `. ${psQuote(install.profile)}`, badExe: null },
    file: 'powershell.exe',
    args: plainArgs(),
    env: install.env,
  });
  if (bash) {
    suites.push({
      label: 'Git Bash after filepick setup',
      kind: { ...BASH, load: null, badExe: null },
      file: bash,
      args: ['--noprofile', '--rcfile', install.bashrc.replace(/\\/g, '/'), '-i'],
      env: { ...install.env, HISTFILE: '/dev/null', PS1: 'BASH$ ', CHERE_INVOKING: '1' },
    });
  }

  const only = (process.env.FILEPICK_SUITE || '').toLowerCase();
  const key = (label) => label.toLowerCase().replace(/ /g, '');
  const chosen = suites.filter((s) => !only || key(s.label).includes(only));
  if (chosen.length === 0) {
    console.error(`FILEPICK_SUITE=${only} matches no suite. Suites:\n${suites.map((s) => `  ${key(s.label)}`).join('\n')}`);
    process.exit(2);
  }

  let failed = 0;
  for (const s of chosen) failed += await suite(s);

  console.log(failed ? `\n${failed} FAILED` : '\nall passed');
  process.exit(failed ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
