// ============================================================
// @clawlabz/clawnetwork — Unit Tests
// Tests key pure-function logic extracted from index.ts
// Run: node --test tests/unit.test.mjs
// ============================================================

import { test, describe } from 'node:test'
import assert from 'node:assert/strict'
import path from 'node:path'
import os from 'node:os'
import { mkdtemp, rm, mkdir, writeFile, readFile } from 'node:fs/promises'

// ── Extracted pure functions (mirrored from index.ts) ──

function getBaseDir() {
  const envDir = process.env.OPENCLAW_DIR
  if (envDir) return envDir
  return path.join(os.homedir(), '.openclaw')
}

function isVersionOlder(current, required) {
  const c = current.split('.').map(Number)
  const r = required.split('.').map(Number)
  for (let i = 0; i < 3; i++) {
    if ((c[i] || 0) < (r[i] || 0)) return true
    if ((c[i] || 0) > (r[i] || 0)) return false
  }
  return false
}

function formatClaw(raw) {
  const DECIMALS = 9
  const ONE_CLAW = BigInt(10 ** DECIMALS)
  const value = typeof raw === 'string' ? BigInt(raw) : raw
  const whole = value / ONE_CLAW
  const frac = value % ONE_CLAW
  if (frac === 0n) return `${whole} CLAW`
  const fracStr = frac.toString().padStart(DECIMALS, '0').replace(/0+$/, '')
  return `${whole}.${fracStr} CLAW`
}

function detectPlatformTarget() {
  const platform = process.platform === 'darwin' ? 'macos' : process.platform === 'win32' ? 'windows' : 'linux'
  const arch = process.arch === 'arm64' ? 'aarch64' : 'x86_64'
  return `${platform}-${arch}`
}

// ── Tests ──

describe('getBaseDir', () => {
  const originalEnv = process.env.OPENCLAW_DIR

  test('returns OPENCLAW_DIR when set', () => {
    process.env.OPENCLAW_DIR = '/tmp/custom-profile'
    assert.equal(getBaseDir(), '/tmp/custom-profile')
  })

  test('falls back to ~/.openclaw when OPENCLAW_DIR not set', () => {
    delete process.env.OPENCLAW_DIR
    assert.equal(getBaseDir(), path.join(os.homedir(), '.openclaw'))
  })

  test('derived paths use base dir correctly', () => {
    process.env.OPENCLAW_DIR = '/tmp/test-profile'
    const base = getBaseDir()
    const workspace = path.join(base, 'workspace', 'clawnetwork')
    const binDir = path.join(base, 'bin')
    const portFile = path.join(base, 'clawnetwork-ui-port')

    assert.equal(workspace, '/tmp/test-profile/workspace/clawnetwork')
    assert.equal(binDir, '/tmp/test-profile/bin')
    assert.equal(portFile, '/tmp/test-profile/clawnetwork-ui-port')
  })

  // Restore env
  test.after(() => {
    if (originalEnv !== undefined) process.env.OPENCLAW_DIR = originalEnv
    else delete process.env.OPENCLAW_DIR
  })
})

describe('isVersionOlder', () => {
  test('detects older version', () => {
    assert.equal(isVersionOlder('0.4.19', '0.4.21'), true)
    assert.equal(isVersionOlder('0.3.0', '0.4.0'), true)
    assert.equal(isVersionOlder('0.4.21', '1.0.0'), true)
  })

  test('detects same version', () => {
    assert.equal(isVersionOlder('0.4.21', '0.4.21'), false)
    assert.equal(isVersionOlder('1.0.0', '1.0.0'), false)
  })

  test('detects newer version', () => {
    assert.equal(isVersionOlder('0.4.22', '0.4.21'), false)
    assert.equal(isVersionOlder('1.0.0', '0.4.21'), false)
  })

  test('handles missing patch version', () => {
    assert.equal(isVersionOlder('0.4', '0.4.1'), true)
    assert.equal(isVersionOlder('0.4.1', '0.4'), false)
  })
})

describe('formatClaw', () => {
  test('formats whole amounts', () => {
    assert.equal(formatClaw(1000000000n), '1 CLAW')
    assert.equal(formatClaw(5000000000n), '5 CLAW')
    assert.equal(formatClaw(0n), '0 CLAW')
  })

  test('formats fractional amounts', () => {
    assert.equal(formatClaw(1500000000n), '1.5 CLAW')
    assert.equal(formatClaw(100000000n), '0.1 CLAW')
    assert.equal(formatClaw(123456789n), '0.123456789 CLAW')
  })

  test('strips trailing zeros', () => {
    assert.equal(formatClaw(1100000000n), '1.1 CLAW')
    assert.equal(formatClaw(1010000000n), '1.01 CLAW')
  })

  test('accepts string input', () => {
    assert.equal(formatClaw('1000000000'), '1 CLAW')
    assert.equal(formatClaw('500000000'), '0.5 CLAW')
  })
})

describe('detectPlatformTarget', () => {
  test('returns valid target string', () => {
    const target = detectPlatformTarget()
    assert.match(target, /^(macos|linux|windows)-(x86_64|aarch64)$/)
  })
})

describe('workspace file operations', () => {
  let tmpDir

  test.before(async () => {
    tmpDir = await mkdtemp(path.join(os.tmpdir(), 'clawnetwork-test-'))
  })

  test.after(async () => {
    if (tmpDir) await rm(tmpDir, { recursive: true, force: true })
  })

  test('wallet file round-trip', async () => {
    const walletDir = path.join(tmpDir, 'workspace', 'clawnetwork')
    await mkdir(walletDir, { recursive: true })

    const wallet = {
      address: 'b68807c359882a4cb4a2eba5af5ea2a084d21548edcdab5fbc94a0a751d713ec',
      secret_key: 'test-secret-key-do-not-use',
    }
    const walletPath = path.join(walletDir, 'wallet.json')
    await writeFile(walletPath, JSON.stringify(wallet), { mode: 0o600 })

    const loaded = JSON.parse(await readFile(walletPath, 'utf8'))
    assert.equal(loaded.address, wallet.address)
    assert.equal(loaded.secret_key, wallet.secret_key)
  })

  test('PID file write and read', async () => {
    const pidFile = path.join(tmpDir, 'node.pid')
    await writeFile(pidFile, '12345')
    const pid = parseInt(await readFile(pidFile, 'utf8'), 10)
    assert.equal(pid, 12345)
  })

  test('port file format', async () => {
    const portFile = path.join(tmpDir, 'clawnetwork-ui-port')
    const info = { port: 19877, pid: 99999, startedAt: new Date().toISOString() }
    await writeFile(portFile, JSON.stringify(info))

    const loaded = JSON.parse(await readFile(portFile, 'utf8'))
    assert.equal(loaded.port, 19877)
    assert.equal(loaded.pid, 99999)
    assert.ok(loaded.startedAt)
  })

  test('config file round-trip', async () => {
    const cfgDir = path.join(tmpDir, 'workspace', 'clawnetwork')
    await mkdir(cfgDir, { recursive: true })

    const cfg = { network: 'mainnet', rpcPort: 9710, p2pPort: 9711, syncMode: 'full' }
    await writeFile(path.join(cfgDir, 'config.json'), JSON.stringify(cfg))

    const loaded = JSON.parse(await readFile(path.join(cfgDir, 'config.json'), 'utf8'))
    assert.deepEqual(loaded, cfg)
  })
})

describe('UI server script generation', () => {
  // This test simulates what startUiServer() does: inject const variables
  // then prepend them to UI_SERVER_SCRIPT. It catches const/var redeclaration bugs.

  test('generated script passes Node.js syntax check', async () => {
    const { execFileSync } = await import('node:child_process')
    const indexPath = path.join(path.dirname(new URL(import.meta.url).pathname), '..', 'index.ts')
    const code = await readFile(indexPath, 'utf8')

    // Extract UI_SERVER_SCRIPT template literal content
    const marker = 'const UI_SERVER_SCRIPT = `'
    const start = code.indexOf(marker)
    assert.ok(start !== -1, 'UI_SERVER_SCRIPT not found in index.ts')

    // Find matching closing backtick (skip escaped ones)
    let depth = 0
    let i = start + marker.length
    let scriptEnd = -1
    while (i < code.length) {
      if (code[i] === '`' && depth === 0) { scriptEnd = i; break }
      if (code[i] === '$' && code[i + 1] === '{') { depth++; i += 2; continue }
      if (code[i] === '}' && depth > 0) { depth--; i++; continue }
      i++
    }
    assert.ok(scriptEnd !== -1, 'Could not find end of UI_SERVER_SCRIPT')
    const uiScript = code.slice(start + marker.length, scriptEnd)

    // Simulate startUiServer() injection — exactly what the real code does
    const injected = [
      'const OPENCLAW_BASE_DIR = "/tmp/test-profile/.openclaw";',
      'const PLUGIN_VERSION = "0.1.18";',
      'const HTML_PATH = "/tmp/test.html";',
      'const HTML = "<html></html>";',
    ].join('\n') + '\n' + uiScript

    const tmpScript = path.join(os.tmpdir(), `clawnetwork-ui-syntax-check-${Date.now()}.js`)
    await writeFile(tmpScript, injected)

    try {
      execFileSync('node', ['--check', tmpScript], { timeout: 5000 })
    } catch (e) {
      await rm(tmpScript, { force: true })
      assert.fail(`Generated UI server script has syntax errors:\n${e.stderr || e.message}`)
    }
    await rm(tmpScript, { force: true })
  })

  test('generated script has no const/var redeclaration conflicts', async () => {
    const indexPath = path.join(path.dirname(new URL(import.meta.url).pathname), '..', 'index.ts')
    const code = await readFile(indexPath, 'utf8')

    const marker = 'const UI_SERVER_SCRIPT = `'
    const start = code.indexOf(marker) + marker.length
    let depth = 0, i = start, scriptEnd = -1
    while (i < code.length) {
      if (code[i] === '`' && depth === 0) { scriptEnd = i; break }
      if (code[i] === '$' && code[i + 1] === '{') { depth++; i += 2; continue }
      if (code[i] === '}' && depth > 0) { depth--; i++; continue }
      i++
    }
    const uiScript = code.slice(start, scriptEnd)

    // These names are injected as const by startUiServer()
    const injectedNames = ['OPENCLAW_BASE_DIR', 'PLUGIN_VERSION', 'HTML_PATH', 'HTML']
    for (const name of injectedNames) {
      const varPattern = new RegExp(`\\bvar\\s+${name}\\b`)
      const constPattern = new RegExp(`\\bconst\\s+${name}\\b`)
      const letPattern = new RegExp(`\\blet\\s+${name}\\b`)
      assert.ok(!varPattern.test(uiScript), `UI script must not redeclare injected '${name}' with var`)
      assert.ok(!constPattern.test(uiScript), `UI script must not redeclare injected '${name}' with const`)
      assert.ok(!letPattern.test(uiScript), `UI script must not redeclare injected '${name}' with let`)
    }
  })
})
