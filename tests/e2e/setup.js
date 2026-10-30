// Tests `filepick setup` and `filepick remove` against throwaway files only:
// a fake install folder, fake PowerShell profiles, a fake .bashrc and a fake
// settings folder, all in %TEMP%. Never touches the real profile, Clink or the
// execution policy.
//
//   node setup.js          (build the release exe first: cargo build --release)

const assert = require('assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');

const ROOT = path.resolve(__dirname, '..', '..');
const EXE = path.join(ROOT, 'target', 'release', 'filepick.exe');

let failed = 0;
function check(title, fn) {
  try {
    fn();
    console.log(`   ok    ${title}`);
  } catch (e) {
    failed++;
    console.log(`   FAIL  ${title}\n         ${String(e.message).split('\n').join('\n         ')}`);
  }
}

function run(exe, args, env) {
  const r = spawnSync(exe, args, { encoding: 'utf8', env: { ...process.env, ...env }, windowsHide: true });
  if (r.error) throw r.error;
  return { code: r.status, out: (r.stdout || '') + (r.stderr || '') };
}

function utf16(text) {
  return Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(text, 'utf16le')]);
}

function which(exe) {
  const r = spawnSync('where.exe', [exe], { encoding: 'utf8' });
  return r.status === 0 ? r.stdout.split(/\r?\n/)[0].trim() : null;
}

function main() {
  if (!fs.existsSync(EXE)) {
    console.error(`Build first: cargo build --release (missing ${EXE})`);
    process.exit(2);
  }
  console.log('\n== filepick setup / remove (sandboxed) ==');

  // %TEMP% is inside %LOCALAPPDATA%, so this also checks the
  // "$env:LOCALAPPDATA\..." form of the profile line.
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'filepick-setup-'));
  const install = path.join(tmp, 'Programs', 'filepick');
  fs.mkdirSync(install, { recursive: true });
  fs.copyFileSync(EXE, path.join(install, 'filepick.exe'));
  for (const f of ['filepick.ps1', 'filepick.lua', 'filepick.bash']) {
    fs.copyFileSync(path.join(ROOT, 'shell', f), path.join(install, f));
  }
  const exe = path.join(install, 'filepick.exe');

  const newProfile = path.join(tmp, 'Documents', 'WindowsPowerShell', 'profile.ps1'); // doesn't exist yet
  const oldProfile = path.join(tmp, 'old-profile.ps1');
  const bashrc = path.join(tmp, '.bashrc');
  const originals = {
    [oldProfile]: utf16('# my settings\r\nSet-Alias ll Get-ChildItem\r\n'),
    [bashrc]: Buffer.from("alias ll='ls -l'\n"),
  };
  for (const [f, b] of Object.entries(originals)) fs.writeFileSync(f, b);

  const configDir = path.join(tmp, 'config');
  const env = { FILEPICK_CONFIG_DIR: configDir };
  const args = ['--profile', newProfile, '--profile', oldProfile, '--bashrc', bashrc, '--no-clink'];

  try {
    let first;
    check('setup succeeds', () => {
      const r = run(exe, ['setup', ...args], env);
      assert.strictEqual(r.code, 0, r.out);
      first = Object.fromEntries([newProfile, oldProfile, bashrc].map((f) => [f, fs.readFileSync(f)]));
    });

    check('the new profile is UTF-8 with a BOM and holds just the line', () => {
      const b = first[newProfile];
      assert.deepStrictEqual([...b.slice(0, 3)], [0xef, 0xbb, 0xbf]);
      const text = b.slice(3).toString('utf8');
      assert.match(text, /^if \(Test-Path -LiteralPath "\$env:LOCALAPPDATA\\.*\\filepick\.ps1"\) \{ \. ".*" -Quiet \}  # filepick-setup\r\n$/);
    });

    check('the UTF-16 profile stays UTF-16, with its own lines kept', () => {
      const b = first[oldProfile];
      assert.deepStrictEqual([...b.slice(0, 2)], [0xff, 0xfe]);
      const text = b.slice(2).toString('utf16le');
      assert.ok(text.startsWith('# my settings\r\nSet-Alias ll Get-ChildItem\r\nif (Test-Path'), text);
      assert.ok(text.endsWith('# filepick-setup\r\n'), text);
    });

    check('.bashrc gets one LF line', () => {
      const text = first[bashrc].toString('utf8');
      assert.match(text, /^alias ll='ls -l'\n\[ -f "\$LOCALAPPDATA\/.*\/filepick\.bash" \] && \. ".*"  # filepick-setup\n$/);
    });

    check('running setup again changes nothing', () => {
      const r = run(exe, ['setup', ...args], env);
      assert.strictEqual(r.code, 0, r.out);
      for (const [f, b] of Object.entries(first)) assert.ok(fs.readFileSync(f).equals(b), `${f} changed`);
    });

    // The settings file changes the hotkey everywhere.
    fs.mkdirSync(configDir, { recursive: true });
    fs.writeFileSync(path.join(configDir, 'config.toml'), 'hotkey = "Alt+p"\n');

    check('`filepick key` gives the hotkey in readline notation', () => {
      assert.strictEqual(run(exe, ['key'], env).out.trim(), '\\ep');
    });

    const shells = [['Windows PowerShell', 'powershell.exe']];
    if (which('pwsh.exe')) shells.push(['PowerShell 7', 'pwsh.exe']);
    for (const [name, ps] of shells) {
      for (const profile of [newProfile, oldProfile]) {
        check(`${name}: the line in ${path.basename(profile)} binds the hotkey from the settings`, () => {
          const script =
            `. '${profile.replace(/'/g, "''")}'; ` +
            `Get-PSReadLineKeyHandler -Bound | Where-Object { $_.Function -eq 'filepick' } | ForEach-Object { $_.Key }`;
          const r = run(ps, ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command', script], env);
          assert.strictEqual(r.out.trim(), 'Alt+p', r.out);
        });
      }
      check(`${name}: the line does nothing (and no error) if filepick was deleted`, () => {
        const gone = path.join(tmp, 'gone.ps1');
        fs.writeFileSync(gone, fs.readFileSync(newProfile).toString('utf8').replace('filepick.ps1', 'not-there.ps1'));
        const r = run(ps, ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command', `. '${gone}'; 'fine'`], env);
        assert.strictEqual(r.out.trim(), 'fine', r.out);
      });
    }

    const bash = [path.join(process.env.ProgramFiles || 'C:\\Program Files', 'Git', 'bin', 'bash.exe')].find((p) => fs.existsSync(p));
    if (bash) {
      check('Git Bash: the .bashrc line binds the hotkey from the settings', () => {
        const r = run(bash, ['--noprofile', '--rcfile', bashrc.replace(/\\/g, '/'), '-i', '-c', 'bind -X'], { ...env, HISTFILE: '/dev/null' });
        assert.match(r.out, /"\\ep":? "__filepick_pick"/, r.out);
      });
    }

    check('remove puts every file back exactly and deletes the profile it created', () => {
      const r = run(exe, ['remove', ...args], env);
      assert.strictEqual(r.code, 0, r.out);
      assert.ok(!fs.existsSync(newProfile), 'the profile setup created is still there');
      for (const [f, b] of Object.entries(originals)) assert.ok(fs.readFileSync(f).equals(b), `${f} differs from the original`);
      assert.ok(!fs.existsSync(path.join(configDir, 'setup-state.txt')), 'setup-state.txt is still there');
    });

    check('remove twice is harmless', () => {
      const r = run(exe, ['remove', ...args], env);
      assert.strictEqual(r.code, 0, r.out);
    });
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }

  console.log(failed ? `\n${failed} FAILED` : '\nall passed');
  process.exit(failed ? 1 : 0);
}

main();
