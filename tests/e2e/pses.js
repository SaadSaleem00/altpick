// Starting the VS Code PowerShell extension's terminal ("PowerShell Extension"
// / PowerShell Editor Services) without VS Code.
//
// That terminal only shows its prompt after an editor connects to it over a
// named pipe and says hello (the Language Server Protocol "initialize"
// handshake). `connect` plays the editor's part with the bare minimum.

const fs = require('fs');
const net = require('net');
const os = require('os');
const path = require('path');

/** The newest installed PowerShell extension folder, or null. */
function findExtension() {
  const root = path.join(os.homedir(), '.vscode', 'extensions');
  if (!fs.existsSync(root)) return null;
  const found = fs
    .readdirSync(root)
    .filter((d) => d.toLowerCase().startsWith('ms-vscode.powershell-'))
    .sort()
    .pop();
  return found ? path.join(root, found) : null;
}

function psQuote(s) {
  return `'${s.replace(/'/g, "''")}'`;
}

/** Arguments for powershell.exe, the same as the extension uses. */
function launch(extDir) {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'filepick-pses-'));
  const sessionFile = path.join(work, 'session.json');
  const modules = path.join(extDir, 'modules');
  const command =
    `Import-Module ${psQuote(path.join(modules, 'PowerShellEditorServices', 'PowerShellEditorServices.psd1'))}; ` +
    `Start-EditorServices -HostName 'Visual Studio Code Host' -HostProfileId 'Microsoft.VSCode' ` +
    `-HostVersion '2025.4.0' -BundledModulesPath ${psQuote(modules)} -EnableConsoleRepl ` +
    `-LogPath ${psQuote(path.join(work, 'logs'))} -SessionDetailsPath ${psQuote(sessionFile)} -FeatureFlags @()`;
  return {
    work,
    sessionFile,
    args: ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', command],
    env: { VSCODE_INJECTION: '1', VSCODE_STABLE: '1', TERM_PROGRAM: 'vscode' },
  };
}

/** Waits for the session file, connects to the language server and initializes it. */
async function connect(sessionFile, timeout = 60000) {
  const until = Date.now() + timeout;
  let details;
  while (!details) {
    try {
      details = JSON.parse(fs.readFileSync(sessionFile, 'utf8'));
    } catch {
      if (Date.now() > until) throw new Error('PowerShell Editor Services did not start');
      await new Promise((r) => setTimeout(r, 200));
    }
  }
  if (details.status !== 'started') throw new Error(`PSES failed: ${JSON.stringify(details)}`);

  const pipe = details.languageServicePipeName;
  const sock = net.connect(pipe);
  await new Promise((resolve, reject) => {
    sock.once('connect', resolve);
    sock.once('error', reject);
  });

  const send = (msg) => {
    const body = Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...msg }), 'utf8');
    sock.write(`Content-Length: ${body.length}\r\n\r\n`);
    sock.write(body);
  };

  // Read messages; answer the server's own requests with "nothing" so it never waits on us.
  let buf = Buffer.alloc(0);
  const pending = new Map();
  sock.on('data', (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    for (;;) {
      const headerEnd = buf.indexOf('\r\n\r\n');
      if (headerEnd < 0) return;
      const m = /Content-Length: (\d+)/i.exec(buf.slice(0, headerEnd).toString());
      const len = Number(m[1]);
      if (buf.length < headerEnd + 4 + len) return;
      const msg = JSON.parse(buf.slice(headerEnd + 4, headerEnd + 4 + len).toString('utf8'));
      buf = buf.slice(headerEnd + 4 + len);
      if (msg.method && msg.id !== undefined) send({ id: msg.id, result: null });
      else if (msg.id !== undefined && pending.has(msg.id)) pending.get(msg.id)(msg);
    }
  });
  sock.on('error', () => {});

  const reply = new Promise((r) => pending.set(1, r));
  send({
    id: 1,
    method: 'initialize',
    params: { processId: process.pid, rootUri: null, capabilities: {}, initializationOptions: {} },
  });
  await reply;
  send({ method: 'initialized', params: {} });
  return sock;
}

module.exports = { findExtension, launch, connect };
