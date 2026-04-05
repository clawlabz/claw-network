/* eslint-disable @typescript-eslint/no-require-imports */
declare const process: { stdout: { write: (s: string) => void }; env: Record<string, string | undefined>; platform: string; arch: string; kill: (pid: number, sig: string) => boolean; pid: number; on: (event: string, handler: (...args: unknown[]) => void) => void }
declare function require(id: string): any
declare function setTimeout(fn: () => void, ms: number): unknown
declare function clearTimeout(id: unknown): void
declare function setInterval(fn: () => void, ms: number): unknown
declare function clearInterval(id: unknown): void
declare function fetch(url: string, init?: Record<string, unknown>): Promise<{ status: number; ok: boolean; text: () => Promise<string>; json: () => Promise<unknown> }>

const VERSION = '0.1.32'
const PLUGIN_ID = 'clawnetwork'
const GITHUB_REPO = 'clawlabz/claw-network'
const DEFAULT_RPC_PORT = 9710
const DEFAULT_P2P_PORT = 9711
const DEFAULT_NETWORK = 'mainnet'
const DEFAULT_SYNC_MODE = 'light'
const DEFAULT_HEALTH_CHECK_SECONDS = 30
const MIN_NODE_VERSION = '0.4.21'
const DEFAULT_UI_PORT = 19877
const MAX_RESTART_ATTEMPTS = 3

// Built-in bootstrap peers for each network
const BOOTSTRAP_PEERS: Record<string, string[]> = {
  mainnet: [
    '/ip4/178.156.162.162/tcp/9711',
    '/ip4/39.102.144.231/tcp/9711',
  ],
  testnet: [
    '/ip4/178.156.162.162/tcp/9721',
    '/ip4/39.102.144.231/tcp/9721',
  ],
  devnet: [], // local dev, no bootstrap
}
const RESTART_BACKOFF_BASE_MS = 5_000
const DECIMALS = 9
const ONE_CLAW = BigInt(10 ** DECIMALS)
const MAX_LOG_BYTES = 5 * 1024 * 1024 // 5 MB log rotation threshold
const HEX64_RE = /^[0-9a-f]{64}$/i
const HEX_RE = /^[0-9a-f]+$/i

// ============================================================
// OpenClaw API Types (mirrors Gateway runtime)
// ============================================================

type GatewayRespond = (ok: boolean, payload: Record<string, unknown>) => void

interface GatewayMethodContext {
  respond?: GatewayRespond
  params?: Record<string, unknown>
}

interface CliCommandChain {
  description: (text: string) => CliCommandChain
  argument: (spec: string, desc?: string) => CliCommandChain
  option: (flags: string, desc: string) => CliCommandChain
  action: (handler: (...args: unknown[]) => void) => CliCommandChain
  command: (name: string) => CliCommandChain
  allowExcessArguments: (allow: boolean) => CliCommandChain
}

interface CliProgram {
  command: (name: string) => CliCommandChain
}

interface RegisterCliContext {
  program: CliProgram
}

interface OpenClawApi {
  config?: Record<string, unknown>
  logger?: {
    info?: (message: string, payload?: Record<string, unknown>) => void
    warn?: (message: string, payload?: Record<string, unknown>) => void
    error?: (message: string, payload?: Record<string, unknown>) => void
  }
  registerGatewayMethod?: (name: string, handler: (ctx: GatewayMethodContext) => void) => void
  registerCli?: (
    handler: (ctx: RegisterCliContext) => void,
    options?: { commands?: string[] }
  ) => void
  registerService?: (service: { id: string; start?: () => void; stop?: () => void }) => void
}

// ============================================================
// Configuration
// ============================================================

interface PluginConfig {
  network: string
  autoStart: boolean
  autoDownload: boolean
  autoRegisterAgent: boolean
  rpcPort: number
  p2pPort: number
  syncMode: string
  healthCheckSeconds: number
  uiPort: number
  extraBootstrapPeers: string[]
}

function getConfig(api: OpenClawApi): PluginConfig {
  const c = (api.config && typeof api.config === 'object') ? api.config : {}
  return {
    network: typeof c.network === 'string' ? c.network : DEFAULT_NETWORK,
    autoStart: typeof c.autoStart === 'boolean' ? c.autoStart : true,
    autoDownload: typeof c.autoDownload === 'boolean' ? c.autoDownload : true,
    autoRegisterAgent: typeof c.autoRegisterAgent === 'boolean' ? c.autoRegisterAgent : true,
    rpcPort: typeof c.rpcPort === 'number' ? c.rpcPort : DEFAULT_RPC_PORT,
    p2pPort: typeof c.p2pPort === 'number' ? c.p2pPort : DEFAULT_P2P_PORT,
    syncMode: typeof c.syncMode === 'string' ? c.syncMode : DEFAULT_SYNC_MODE,
    healthCheckSeconds: typeof c.healthCheckSeconds === 'number' ? c.healthCheckSeconds : DEFAULT_HEALTH_CHECK_SECONDS,
    uiPort: typeof c.uiPort === 'number' ? c.uiPort : DEFAULT_UI_PORT,
    extraBootstrapPeers: Array.isArray(c.extraBootstrapPeers) ? c.extraBootstrapPeers.filter((p: unknown) => typeof p === 'string') : [],
  }
}

// ============================================================
// Utilities
// ============================================================

const os = require('os')
const path = require('path')
const fs = require('fs')
const { execFileSync, spawn: nodeSpawn, fork } = require('child_process')

function getBaseDir(): string {
  // Gateway sets OPENCLAW_STATE_DIR for named profiles (e.g. ~/.openclaw-ludis)
  // OPENCLAW_DIR is the user-facing alias (used by install.sh)
  const stateDir = process.env.OPENCLAW_STATE_DIR
  if (stateDir) return stateDir
  const envDir = process.env.OPENCLAW_DIR
  if (envDir) return envDir
  return path.join(os.homedir(), '.openclaw')
}

function homePath(...segments: string[]): string {
  return path.join(os.homedir(), ...segments)
}

const WORKSPACE_DIR = path.join(getBaseDir(), 'workspace', 'clawnetwork')
const BIN_DIR = path.join(getBaseDir(), 'bin')
// Plugin uses its own chain data dir under workspace to avoid locking conflicts with other nodes
const DATA_DIR = path.join(getBaseDir(), 'workspace', 'clawnetwork', 'chain-data')
const WALLET_PATH = path.join(WORKSPACE_DIR, 'wallet.json')
const LOG_PATH = path.join(WORKSPACE_DIR, 'node.log')
const UI_PORT_FILE = path.join(getBaseDir(), 'clawnetwork-ui-port')

function ensureDir(dir: string): void {
  fs.mkdirSync(dir, { recursive: true })
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}

function formatClaw(raw: bigint | string): string {
  const value = typeof raw === 'string' ? BigInt(raw) : raw
  const whole = value / ONE_CLAW
  const frac = value % ONE_CLAW
  if (frac === 0n) return `${whole} CLAW`
  const fracStr = frac.toString().padStart(DECIMALS, '0').replace(/0+$/, '')
  return `${whole}.${fracStr} CLAW`
}

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  return `${h}h ${m}m`
}

// ── Input validation ──

function isValidAddress(addr: string): boolean {
  return HEX64_RE.test(addr)
}

function isValidPrivateKey(key: string): boolean {
  return key.length === 64 && HEX_RE.test(key)
}

function isValidAmount(amount: string): boolean {
  return /^\d+(\.\d+)?$/.test(amount) && parseFloat(amount) > 0
}

function isValidNetwork(network: string): boolean {
  return ['mainnet', 'testnet', 'devnet'].includes(network)
}

function isValidSyncMode(mode: string): boolean {
  return ['full', 'fast', 'light'].includes(mode)
}

function sanitizeAgentName(name: string): string {
  return name.replace(/[^a-zA-Z0-9_-]/g, '').slice(0, 32)
}

// ── Log rotation ──

function rotateLogIfNeeded(): void {
  try {
    if (!fs.existsSync(LOG_PATH)) return
    const stat = fs.statSync(LOG_PATH)
    if (stat.size > MAX_LOG_BYTES) {
      const rotated = `${LOG_PATH}.1`
      try { fs.unlinkSync(rotated) } catch { /* ok */ }
      fs.renameSync(LOG_PATH, rotated)
    }
  } catch { /* ok */ }
}

// ============================================================
// Binary Management
// ============================================================

function findBinary(): string | null {
  // 1. Check our managed location
  const managedPath = path.join(BIN_DIR, process.platform === 'win32' ? 'claw-node.exe' : 'claw-node')
  if (fs.existsSync(managedPath)) return managedPath

  // 2. Check if in PATH
  try {
    const which = process.platform === 'win32' ? 'where' : 'which'
    const result = execFileSync(which, ['claw-node'], { encoding: 'utf8', timeout: 5000 }).trim()
    if (result) return result.split('\n')[0]
  } catch { /* not found */ }

  // 3. Check data dir bin (install.sh puts it here)
  const dataDirBin = path.join(DATA_DIR, 'bin', 'claw-node')
  if (fs.existsSync(dataDirBin)) return dataDirBin

  return null
}

function getBinaryVersion(binaryPath: string): string | null {
  try {
    const output = execFileSync(binaryPath, ['--version'], { encoding: 'utf8', timeout: 5000 }).trim()
    const match = output.match(/(\d+\.\d+\.\d+)/)
    return match ? match[1] : output
  } catch { return null }
}

function isVersionOlder(current: string, required: string): boolean {
  const c = current.split('.').map(Number)
  const r = required.split('.').map(Number)
  for (let i = 0; i < 3; i++) {
    if ((c[i] || 0) < (r[i] || 0)) return true
    if ((c[i] || 0) > (r[i] || 0)) return false
  }
  return false
}

function detectPlatformTarget(): string {
  const platform = process.platform === 'darwin' ? 'macos' : process.platform === 'win32' ? 'windows' : 'linux'
  const arch = process.arch === 'arm64' ? 'aarch64' : 'x86_64'
  return `${platform}-${arch}`
}

async function downloadBinary(api: OpenClawApi): Promise<string> {
  ensureDir(BIN_DIR)
  const target = detectPlatformTarget()
  const ext = process.platform === 'win32' ? 'zip' : 'tar.gz'
  const binaryName = process.platform === 'win32' ? 'claw-node.exe' : 'claw-node'
  const destPath = path.join(BIN_DIR, binaryName)

  api.logger?.info?.(`[clawnetwork] downloading claw-node for ${target}...`)

  // Resolve latest version from GitHub (HTTPS only)
  let version = 'latest'
  try {
    const res = await fetch(`https://api.github.com/repos/${GITHUB_REPO}/releases/latest`)
    if (res.ok) {
      const data = await res.json() as Record<string, unknown>
      if (typeof data.tag_name === 'string') version = data.tag_name
    }
  } catch { /* fallback to latest redirect */ }

  const baseUrl = version === 'latest'
    ? `https://github.com/${GITHUB_REPO}/releases/latest/download`
    : `https://github.com/${GITHUB_REPO}/releases/download/${version}`

  const downloadUrl = `${baseUrl}/claw-node-${target}.${ext}`
  const checksumUrl = `${baseUrl}/SHA256SUMS.txt`
  api.logger?.info?.(`[clawnetwork] download URL: ${downloadUrl}`)

  const tmpFile = path.join(os.tmpdir(), `claw-node-download-${Date.now()}.${ext}`)
  try {
    execFileSync('curl', ['-sSfL', '-o', tmpFile, downloadUrl], { timeout: 120_000 })
  } catch {
    try {
      execFileSync('wget', ['-qO', tmpFile, downloadUrl], { timeout: 120_000 })
    } catch (e: unknown) {
      throw new Error(`Failed to download claw-node: ${(e as Error).message}`)
    }
  }

  // Verify SHA256 checksum
  try {
    const checksumTmp = path.join(os.tmpdir(), `claw-node-sha256-${Date.now()}.txt`)
    execFileSync('curl', ['-sSfL', '-o', checksumTmp, checksumUrl], { timeout: 30_000 })
    const checksumContent = fs.readFileSync(checksumTmp, 'utf8')
    const expectedLine = checksumContent.split('\n').find((l: string) => l.includes(`claw-node-${target}`))
    if (expectedLine) {
      const expectedHash = expectedLine.split(/\s+/)[0]
      const cmd = process.platform === 'darwin' ? 'shasum' : 'sha256sum'
      const args = process.platform === 'darwin' ? ['-a', '256', tmpFile] : [tmpFile]
      const actualOutput = execFileSync(cmd, args, { encoding: 'utf8', timeout: 30_000 })
      const actualHash = actualOutput.split(/\s+/)[0]
      if (actualHash.toLowerCase() !== expectedHash.toLowerCase()) {
        fs.unlinkSync(tmpFile)
        try { fs.unlinkSync(checksumTmp) } catch { /* ok */ }
        throw new Error(`SHA256 mismatch: expected ${expectedHash}, got ${actualHash}`)
      }
      api.logger?.info?.(`[clawnetwork] SHA256 verified: ${actualHash.slice(0, 16)}...`)
    }
    try { fs.unlinkSync(checksumTmp) } catch { /* ok */ }
  } catch (e: unknown) {
    // Upgrade checksum failure to error (was warning before)
    const msg = (e as Error).message
    if (msg.includes('SHA256 mismatch')) throw e
    // Throw error on checksum download/verification failure
    throw new Error(`Failed to verify checksum: ${msg}`)
  }

  // Extract
  if (ext === 'tar.gz') {
    execFileSync('tar', ['xzf', tmpFile, '-C', BIN_DIR], { timeout: 30_000 })
  } else {
    execFileSync('powershell', ['-Command', `Expand-Archive -Path "${tmpFile}" -DestinationPath "${BIN_DIR}" -Force`], { timeout: 30_000 })
  }

  // Ensure executable
  if (process.platform !== 'win32') {
    fs.chmodSync(destPath, 0o755)
  }

  // Cleanup
  try { fs.unlinkSync(tmpFile) } catch { /* ok */ }

  if (!fs.existsSync(destPath)) {
    throw new Error(`Binary not found after extraction at ${destPath}`)
  }

  api.logger?.info?.(`[clawnetwork] claw-node installed at ${destPath} (${version})`)
  return destPath
}

// ============================================================
// Node Init
// ============================================================

function isInitialized(): boolean {
  // Both genesis config AND chain data must exist for a proper init
  const hasGenesis = fs.existsSync(path.join(DATA_DIR, 'genesis.json'))
  const hasChainDb = fs.existsSync(path.join(DATA_DIR, 'chain.redb'))
  return hasGenesis && hasChainDb
}

function initNode(binaryPath: string, network: string, api: OpenClawApi): void {
  if (!isValidNetwork(network)) throw new Error(`Invalid network: ${network}`)
  if (isInitialized()) {
    api.logger?.info?.('[clawnetwork] node already initialized, skipping init')
    return
  }
  api.logger?.info?.(`[clawnetwork] initializing node for ${network}...`)
  try {
    ensureDir(DATA_DIR)
    const output = execFileSync(binaryPath, ['init', '--network', network, '--data-dir', DATA_DIR], {
      encoding: 'utf8',
      timeout: 30_000,
      env: { HOME: os.homedir(), PATH: process.env.PATH || '' }, // minimal env
    })
    api.logger?.info?.(`[clawnetwork] init complete: ${output.trim().slice(0, 200)}`)
  } catch (e: unknown) {
    throw new Error(`Node init failed: ${(e as Error).message}`)
  }
}

// ============================================================
// Wallet Management
// ============================================================

interface WalletData {
  address: string
  secretKey: string
  createdAt: string
  network: string
}

function loadWallet(): WalletData | null {
  try {
    const raw = fs.readFileSync(WALLET_PATH, 'utf8')
    return JSON.parse(raw) as WalletData
  } catch { return null }
}

function saveWallet(data: WalletData): void {
  ensureDir(WORKSPACE_DIR)
  fs.writeFileSync(WALLET_PATH, JSON.stringify(data, null, 2) + '\n', { mode: 0o600 })
}

function ensureWallet(network: string, api?: OpenClawApi): WalletData {
  const existing = loadWallet()
  if (existing) return existing

  // Try to read from claw-node's key.json
  const nodeKeyPath = path.join(DATA_DIR, 'key.json')
  if (fs.existsSync(nodeKeyPath)) {
    try {
      const nodeKey = JSON.parse(fs.readFileSync(nodeKeyPath, 'utf8'))
      const wallet: WalletData = {
        address: String(nodeKey.address || nodeKey.public_key || ''),
        secretKey: String(nodeKey.secret_key || nodeKey.private_key || ''),
        createdAt: new Date().toISOString(),
        network,
      }
      if (wallet.address && wallet.secretKey) {
        saveWallet(wallet)
        api?.logger?.info?.(`[clawnetwork] wallet loaded from node key: ${wallet.address.slice(0, 12)}...`)
        return wallet
      }
    } catch { /* fallthrough to generate */ }
  }

  // Generate new wallet via crypto.randomBytes
  const crypto = require('crypto')
  const privKey = crypto.randomBytes(32)
  const secretKeyHex = privKey.toString('hex')

  let address = ''
  const binary = findBinary()
  if (binary) {
    try {
      ensureDir(DATA_DIR)
      execFileSync(binary, ['key', 'import', secretKeyHex, '--data-dir', DATA_DIR], {
        encoding: 'utf8',
        timeout: 10_000,
        env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
      })
      const showOut = execFileSync(binary, ['key', 'show', '--data-dir', DATA_DIR], {
        encoding: 'utf8',
        timeout: 5000,
        env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
      }).trim()
      const showMatch = showOut.match(/[0-9a-f]{64}/i)
      if (showMatch) address = showMatch[0]
    } catch { /* ok, address resolved later */ }
  }

  const wallet: WalletData = {
    address,
    secretKey: secretKeyHex,
    createdAt: new Date().toISOString(),
    network,
  }
  saveWallet(wallet)
  api?.logger?.info?.(`[clawnetwork] new wallet generated: ${address ? address.slice(0, 12) + '...' : '(pending)'}`)
  return wallet
}

// ============================================================
// Node Process Manager
// ============================================================

let nodeProcess: any = null
let healthTimer: unknown = null
let restartCount = 0
let stopping = false

interface NodeStatus {
  running: boolean
  pid: number | null
  blockHeight: number | null
  peerCount: number | null
  network: string
  syncMode: string
  rpcUrl: string
  walletAddress: string
  binaryVersion: string | null
  pluginVersion: string
  uptime: number | null
  uptimeFormatted: string | null
  restartCount: number
  dataDir: string
}

let nodeStartedAt: number | null = null
let lastHealth: { blockHeight: number | null; peerCount: number | null; syncing: boolean } = { blockHeight: null, peerCount: null, syncing: false }
let cachedBinaryVersion: string | null = null
let activeRpcPort: number | null = null  // actual port the plugin node is running on (may differ from config)
let activeP2pPort: number | null = null

function isNodeRunning(): { running: boolean; pid: number | null } {
  // 1. In-memory process reference
  if (nodeProcess && !nodeProcess.killed) return { running: true, pid: nodeProcess.pid }
  // 2. PID file — the ONLY authority for "is MY node running"
  const pidFile = path.join(WORKSPACE_DIR, 'node.pid')
  try {
    const pid = parseInt(fs.readFileSync(pidFile, 'utf8').trim(), 10)
    if (pid > 0) {
      try { execFileSync('kill', ['-0', String(pid)], { timeout: 2000 }); return { running: true, pid } } catch {
        // PID file exists but process is dead — clean up stale PID file
        try { fs.unlinkSync(pidFile) } catch { /* ok */ }
      }
    }
  } catch { /* no file */ }
  // No port probing — if we don't have a PID, we don't own any running node
  return { running: false, pid: null }
}

/** Check if a TCP port is in use via nc -z (works across users, no bind needed) */
function isPortInUse(port: number): boolean {
  try {
    execFileSync('nc', ['-z', '127.0.0.1', String(port)], { timeout: 2000, stdio: 'ignore' })
    return true // connection succeeded → something is listening
  } catch {
    return false // connection refused → port is free
  }
}

/** Find available ports starting from the configured ones, skipping occupied ports */
function findAvailablePorts(rpcPort: number, p2pPort: number, api: OpenClawApi): { rpcPort: number; p2pPort: number } {
  const MAX_TRIES = 20
  let rpc = rpcPort
  let p2p = p2pPort

  // Find available RPC port
  for (let i = 0; i < MAX_TRIES; i++) {
    if (!isPortInUse(rpc)) break
    api.logger?.info?.(`[clawnetwork] RPC port ${rpc} in use, trying ${rpc + 1}...`)
    rpc++
  }

  // Find available P2P port (must also differ from RPC port)
  for (let i = 0; i < MAX_TRIES; i++) {
    if (!isPortInUse(p2p) && p2p !== rpc) break
    api.logger?.info?.(`[clawnetwork] P2P port ${p2p} in use or conflicts with RPC, trying ${p2p + 1}...`)
    p2p++
  }

  if (rpc !== rpcPort || p2p !== p2pPort) {
    api.logger?.info?.(`[clawnetwork] resolved ports: RPC=${rpc} (config=${rpcPort}), P2P=${p2p} (config=${p2pPort})`)
  }
  return { rpcPort: rpc, p2pPort: p2p }
}

function buildStatus(cfg: PluginConfig): NodeStatus {
  const wallet = loadWallet()
  const nodeState = isNodeRunning()
  const rpcPort = activeRpcPort ?? cfg.rpcPort
  const uptime = nodeStartedAt ? Math.floor((Date.now() - nodeStartedAt) / 1000) : null
  return {
    running: nodeState.running,
    pid: nodeState.pid,
    blockHeight: lastHealth.blockHeight,
    peerCount: lastHealth.peerCount,
    network: cfg.network,
    syncMode: cfg.syncMode,
    rpcUrl: `http://localhost:${rpcPort}`,
    walletAddress: wallet?.address ?? '',
    binaryVersion: cachedBinaryVersion,
    pluginVersion: VERSION,
    uptime,
    uptimeFormatted: uptime !== null ? formatUptime(uptime) : null,
    restartCount,
    dataDir: DATA_DIR,
  }
}

async function checkHealth(rpcPort: number): Promise<{ blockHeight: number | null; peerCount: number | null; syncing: boolean }> {
  try {
    const res = await fetch(`http://localhost:${rpcPort}/health`)
    if (!res.ok) return { blockHeight: null, peerCount: null, syncing: false }
    const data = await res.json() as Record<string, unknown>
    return {
      blockHeight: typeof data.block_height === 'number' ? data.block_height : typeof data.blockHeight === 'number' ? data.blockHeight : null,
      peerCount: typeof data.peer_count === 'number' ? data.peer_count : typeof data.peers === 'number' ? data.peers : null,
      syncing: data.syncing === true,
    }
  } catch {
    return { blockHeight: null, peerCount: null, syncing: false }
  }
}

async function rpcCall(rpcPort: number, method: string, params: unknown[] = []): Promise<unknown> {
  const res = await fetch(`http://localhost:${rpcPort}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', method, params, id: Date.now() }),
  })
  const data = await res.json() as Record<string, unknown>
  if (data.error) {
    const err = data.error as Record<string, unknown>
    throw new Error(String(err.message || JSON.stringify(err)))
  }
  return data.result
}

function startNodeProcess(binaryPath: string, cfg: PluginConfig, api: OpenClawApi): void {
  // Guard: check in-memory reference and PID file only (not port)
  if (nodeProcess && !nodeProcess.killed) {
    api.logger?.warn?.('[clawnetwork] node already running (in-memory)')
    return
  }
  const existingState = isNodeRunning()
  if (existingState.running) {
    api.logger?.info?.(`[clawnetwork] node already running (pid=${existingState.pid}), skipping start`)
    return
  }

  if (!isValidNetwork(cfg.network)) { api.logger?.error?.(`[clawnetwork] invalid network: ${cfg.network}`); return }
  if (!isValidSyncMode(cfg.syncMode)) { api.logger?.error?.(`[clawnetwork] invalid sync mode: ${cfg.syncMode}`); return }

  // Find available ports (auto-increment if configured ports are occupied by other processes)
  const ports = findAvailablePorts(cfg.rpcPort, cfg.p2pPort, api)
  activeRpcPort = ports.rpcPort
  activeP2pPort = ports.p2pPort

  const args = ['start', '--network', cfg.network, '--rpc-port', String(ports.rpcPort), '--p2p-port', String(ports.p2pPort), '--sync-mode', cfg.syncMode, '--data-dir', DATA_DIR, '--allow-genesis']

  // Add bootstrap peers: built-in for the network + user-configured extra peers
  const peers = [...(BOOTSTRAP_PEERS[cfg.network] ?? []), ...cfg.extraBootstrapPeers]
  for (const peer of peers) {
    args.push('--bootstrap', peer)
  }

  api.logger?.info?.(`[clawnetwork] starting node: ${binaryPath} ${args.join(' ')}`)

  rotateLogIfNeeded()
  ensureDir(WORKSPACE_DIR)

  // Minimal env to prevent leaking secrets from parent process
  const safeEnv: Record<string, string> = {
    HOME: os.homedir(),
    PATH: process.env.PATH || '/usr/local/bin:/usr/bin:/bin',
    RUST_LOG: process.env.RUST_LOG || 'claw=info',
  }

  // Open log file as fd for direct stdio redirect (allows parent process to exit)
  const logFd = fs.openSync(LOG_PATH, 'a')

  nodeProcess = nodeSpawn(binaryPath, args, {
    stdio: ['ignore', logFd, logFd],
    detached: true,
    env: safeEnv,
  })

  // Unref so CLI process can exit while node keeps running in background
  nodeProcess.unref()

  nodeStartedAt = Date.now()
  restartCount = 0
  stopping = false

  // Cache binary version
  cachedBinaryVersion = getBinaryVersion(binaryPath)

  // Save PID for later management
  const pidFile = path.join(WORKSPACE_DIR, 'node.pid')
  fs.writeFileSync(pidFile, String(nodeProcess.pid))

  // Save actual runtime ports to workspace (UI server and health checks read from here)
  const runtimeCfg = path.join(WORKSPACE_DIR, 'runtime.json')
  fs.writeFileSync(runtimeCfg, JSON.stringify({ rpcPort: ports.rpcPort, p2pPort: ports.p2pPort, pid: nodeProcess.pid, startedAt: Date.now() }))

  nodeProcess.on('exit', (code: number | null) => {
    api.logger?.warn?.(`[clawnetwork] node exited with code ${code}`)
    fs.closeSync(logFd)
    nodeProcess = null
    nodeStartedAt = null
    lastHealth = { blockHeight: null, peerCount: null, syncing: false }
    try { fs.unlinkSync(pidFile) } catch { /* ok */ }

    // Check file-based stop signal (set by stop from different CLI process)
    const stopFile = path.join(WORKSPACE_DIR, 'stop.signal')
    const wasStopped = stopping || fs.existsSync(stopFile)
    try { fs.unlinkSync(stopFile) } catch { /* ok */ }

    if (!wasStopped && code !== 0 && restartCount < MAX_RESTART_ATTEMPTS) {
      restartCount++
      const delay = RESTART_BACKOFF_BASE_MS * Math.pow(2, restartCount - 1)
      api.logger?.info?.(`[clawnetwork] restarting in ${delay}ms (attempt ${restartCount}/${MAX_RESTART_ATTEMPTS})...`)
      setTimeout(() => startNodeProcess(binaryPath, cfg, api), delay)
    }
  })

  startHealthCheck(cfg, api)
}

function startHealthCheck(cfg: PluginConfig, api: OpenClawApi): void {
  if (healthTimer) clearTimeout(healthTimer)

  const check = async () => {
    const rpcPort = activeRpcPort ?? cfg.rpcPort
    lastHealth = await checkHealth(rpcPort)
    if (lastHealth.blockHeight !== null) {
      api.logger?.info?.(`[clawnetwork] height=${lastHealth.blockHeight} peers=${lastHealth.peerCount} syncing=${lastHealth.syncing}`)
    }
    if (!stopping) {
      healthTimer = setTimeout(check, cfg.healthCheckSeconds * 1000)
    }
  }

  healthTimer = setTimeout(check, 5000)
}

function stopNode(api: OpenClawApi): void {
  stopping = true
  if (healthTimer) {
    clearTimeout(healthTimer)
    healthTimer = null
  }

  // Find PID: in-memory process or PID file — the ONLY ways to identify our node
  let pid: number | null = nodeProcess?.pid ?? null
  const pidFile = path.join(WORKSPACE_DIR, 'node.pid')
  if (!pid) {
    try {
      const savedPid = parseInt(fs.readFileSync(pidFile, 'utf8').trim(), 10)
      if (savedPid > 0) pid = savedPid
    } catch { /* no pid file */ }
  }

  if (pid) {
    api.logger?.info?.(`[clawnetwork] stopping node pid=${pid} (SIGTERM)...`)
    try { process.kill(pid, 'SIGTERM') } catch (e: unknown) {
      api.logger?.warn?.(`[clawnetwork] failed to kill pid=${pid}: ${(e as Error).message}`)
    }
    setTimeout(() => {
      try { process.kill(pid as number, 'SIGKILL') } catch { /* ok */ }
    }, 10_000)
  } else {
    api.logger?.warn?.('[clawnetwork] no PID found — cannot stop node (may not be running)')
  }

  // Write stop signal file (tells restart loop in other CLI processes to stop)
  const stopFile = path.join(WORKSPACE_DIR, 'stop.signal')
  try { fs.writeFileSync(stopFile, String(Date.now())) } catch { /* ok */ }

  // NO pkill — we only kill our own process identified by PID file

  nodeProcess = null
  nodeStartedAt = null
  activeRpcPort = null
  activeP2pPort = null
  lastHealth = { blockHeight: null, peerCount: null, syncing: false }
  try { fs.unlinkSync(pidFile) } catch { /* ok */ }
}

// ============================================================
// Agent Registration
// ============================================================

async function autoRegisterAgent(cfg: PluginConfig, wallet: WalletData, api: OpenClawApi): Promise<void> {
  if (!cfg.autoRegisterAgent) return
  if (!wallet.address) return

  try {
    const agent = await rpcCall(cfg.rpcPort, 'claw_getAgent', [wallet.address])
    if (agent) {
      api.logger?.info?.(`[clawnetwork] agent already registered on-chain: ${wallet.address.slice(0, 12)}...`)
      return
    }
  } catch {
    return // Node not ready
  }

  if (cfg.network === 'testnet' || cfg.network === 'devnet') {
    try {
      await rpcCall(cfg.rpcPort, 'claw_faucet', [wallet.address])
      api.logger?.info?.('[clawnetwork] faucet: received testnet CLAW')
    } catch { /* ok */ }
  }

  const binary = findBinary()
  if (!binary) return

  try {
    const agentName = sanitizeAgentName(`openclaw-${wallet.address.slice(0, 8)}`)
    const output = execFileSync(binary, [
      'register-agent', '--name', agentName,
      '--rpc', `http://localhost:${activeRpcPort ?? cfg.rpcPort}`,
      '--data-dir', DATA_DIR,
    ], {
      encoding: 'utf8',
      timeout: 30_000,
      env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
    })
    api.logger?.info?.(`[clawnetwork] agent registered: ${agentName} — ${output.trim().slice(0, 200)}`)
  } catch (e: unknown) {
    api.logger?.warn?.(`[clawnetwork] agent registration skipped: ${(e as Error).message}`)
  }
}

// ============================================================
// Mining: Auto Miner Registration + Heartbeat Loop
// ============================================================

// V2 heartbeat: chain requires 100 blocks × 3s = 300s minimum between heartbeats.
// Use 310s to provide margin for block time variance.
const MINER_HEARTBEAT_INTERVAL_MS = 310 * 1000 // 310 seconds (~5.2 minutes)
let minerHeartbeatTimer: unknown = null

async function autoRegisterMiner(cfg: PluginConfig, wallet: WalletData, api: OpenClawApi): Promise<void> {
  if (!wallet.address) return

  const binary = findBinary()
  if (!binary) return

  // Register as miner
  const minerName = sanitizeAgentName(`openclaw-miner-${wallet.address.slice(0, 8)}`)
  try {
    const output = execFileSync(binary, [
      'register-miner', '--name', minerName,
      '--rpc', `http://localhost:${activeRpcPort ?? cfg.rpcPort}`,
      '--data-dir', DATA_DIR,
    ], {
      encoding: 'utf8',
      timeout: 30_000,
      env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
    })
    api.logger?.info?.(`[clawnetwork] miner registered: ${minerName} — ${output.trim().slice(0, 200)}`)
  } catch (e: unknown) {
    // "already registered" is fine
    const msg = (e as Error).message
    if (msg.includes('already') || msg.includes('exists')) {
      api.logger?.info?.(`[clawnetwork] miner already registered: ${wallet.address.slice(0, 12)}...`)
    } else {
      api.logger?.warn?.(`[clawnetwork] miner registration failed: ${msg.slice(0, 200)}`)
    }
  }

  // Send first heartbeat immediately
  await sendMinerHeartbeat(cfg, api)

  // Start periodic heartbeat loop
  startMinerHeartbeatLoop(cfg, api)
}

async function sendMinerHeartbeat(cfg: PluginConfig, api: OpenClawApi): Promise<void> {
  const binary = findBinary()
  if (!binary) return

  // Try V3 checkin first; fall back to legacy heartbeat if not active
  try {
    const output = execFileSync(binary, [
      'miner-checkin',
      '--rpc', `http://localhost:${activeRpcPort ?? cfg.rpcPort}`,
      '--data-dir', DATA_DIR,
    ], {
      encoding: 'utf8',
      timeout: 30_000,
      env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
    })
    api.logger?.info?.(`[clawnetwork] ${output.trim()}`)
    return
  } catch (e: unknown) {
    const msg = (e as Error).message || ''
    if (!msg.includes('not yet active') && !msg.includes('method not found')) {
      api.logger?.warn?.(`[clawnetwork] checkin failed: ${msg.slice(0, 200)}`)
      return
    }
    // V3 not active — fall through to legacy heartbeat
  }

  try {
    const output = execFileSync(binary, [
      'miner-heartbeat',
      '--rpc', `http://localhost:${activeRpcPort ?? cfg.rpcPort}`,
      '--data-dir', DATA_DIR,
    ], {
      encoding: 'utf8',
      timeout: 30_000,
      env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
    })
    api.logger?.info?.(`[clawnetwork] ${output.trim()}`)
  } catch (e: unknown) {
    api.logger?.warn?.(`[clawnetwork] heartbeat failed: ${(e as Error).message.slice(0, 200)}`)
  }
}

function startMinerHeartbeatLoop(cfg: PluginConfig, api: OpenClawApi): void {
  if (minerHeartbeatTimer) clearInterval(minerHeartbeatTimer)
  minerHeartbeatTimer = setInterval(() => {
    sendMinerHeartbeat(cfg, api).catch(() => {})
  }, MINER_HEARTBEAT_INTERVAL_MS)
  api.logger?.info?.(`[clawnetwork] miner heartbeat loop started (every ${Math.round(MINER_HEARTBEAT_INTERVAL_MS / 60000)}min)`)
}

function stopMinerHeartbeatLoop(): void {
  if (minerHeartbeatTimer) {
    clearInterval(minerHeartbeatTimer)
    minerHeartbeatTimer = null
  }
}

// ============================================================
// WebUI Server
// ============================================================

function buildUiHtml(cfg: PluginConfig): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>ClawNetwork Node Dashboard</title>
  <link rel="icon" href="https://explorer.clawlabz.xyz/favicon.png">
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link href="https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@400;500;600;700;800&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet">
  <style>
    :root {
      --bg: #0a0705;
      --bg-panel: #140e0a;
      --border: #2a1c14;
      --accent: #F96706;
      --accent-dim: rgba(249, 103, 6, 0.15);
      --accent-light: #FF8C3A;
      --purple: #a855f7;
      --purple-dim: rgba(168, 85, 247, 0.15);
      --green: #22c55e;
      --green-dim: rgba(34, 197, 94, 0.15);
      --text: #fffaf5;
      --text-dim: #8892a0;
      --danger: #ef4444;
      --font: 'Space Grotesk', system-ui, -apple-system, sans-serif;
      --font-mono: 'JetBrains Mono', 'SF Mono', Consolas, monospace;
      --radius: 10px;
      --shadow: 0 4px 24px rgba(0, 0, 0, 0.5);
    }
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    body { background: var(--bg); color: var(--text); font-family: var(--font); line-height: 1.6; min-height: 100vh; }
    .container { max-width: 960px; margin: 0 auto; padding: 0 20px; }
    @keyframes pulse { 0%,100%{opacity:1} 50%{opacity:0.4} }

    .header { background: var(--bg-panel); border-bottom: 1px solid var(--border); padding: 16px 0; position: sticky; top: 0; z-index: 100; }
    .header .container { display: flex; align-items: center; justify-content: space-between; }
    .logo { font-size: 22px; font-weight: 800; letter-spacing: -0.5px; }
    .logo-claw { color: #ffffff; }
    .logo-net { color: var(--accent); }
    .header-badge { font-size: 11px; background: var(--accent-dim); color: var(--accent); padding: 2px 8px; border-radius: 4px; }

    .stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 12px; }
    .stat-card { background: var(--bg-panel); border: 1px solid var(--border); border-radius: var(--radius); padding: 20px; }
    .stat-label { font-size: 12px; color: var(--text-dim); text-transform: uppercase; letter-spacing: 1px; }
    .stat-value { font-size: 28px; font-weight: 700; font-family: var(--font-mono); margin-top: 4px; }
    .stat-value.green { color: var(--green); }
    .stat-value.accent { color: var(--accent); }
    .stat-value.purple { color: var(--purple); }
    .stat-value.danger { color: var(--danger); }

    .panel { background: var(--bg-panel); border: 1px solid var(--border); border-radius: var(--radius); padding: 20px; margin: 16px 0; }
    .panel-title { font-size: 14px; font-weight: 600; color: var(--text-dim); margin-bottom: 12px; text-transform: uppercase; letter-spacing: 1px; }
    .info-row { display: flex; justify-content: space-between; padding: 8px 0; border-bottom: 1px solid var(--border); font-size: 14px; }
    .info-row:last-child { border-bottom: none; }
    .info-label { color: var(--text-dim); }
    .info-value { font-family: var(--font-mono); color: var(--text); word-break: break-all; max-width: 60%; text-align: right; }

    .status-dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; margin-right: 6px; }
    .status-dot.online { background: var(--green); box-shadow: 0 0 8px var(--green); }
    .status-dot.offline { background: var(--danger); }
    .status-dot.syncing { background: #ffaa00; animation: pulse 1.5s infinite; }

    .btn { display: inline-flex; align-items: center; gap: 6px; padding: 8px 16px; border-radius: 6px; border: 1px solid var(--border); background: var(--bg-panel); color: var(--text); font-size: 13px; cursor: pointer; transition: 0.2s; font-family: var(--font); }
    .btn:hover { border-color: var(--accent); color: var(--accent); }
    .btn.danger:hover { border-color: var(--danger); color: var(--danger); }
    .btn.primary { background: var(--accent-dim); border-color: var(--accent); color: var(--accent); }
    .node-controls { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; padding-top: 16px; margin-top: 16px; border-top: 1px solid var(--border); }
    .node-controls .spacer { flex: 1; }

    .wallet-hero { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; margin-bottom: 16px; flex-wrap: wrap; }
    .wallet-balance { font-size: 36px; font-weight: 800; font-family: var(--font-mono); color: var(--accent); letter-spacing: -1px; line-height: 1; }
    .wallet-balance-label { font-size: 11px; color: var(--text-dim); text-transform: uppercase; letter-spacing: 1px; margin-bottom: 6px; }
    .wallet-addr-wrap { flex: 1; min-width: 0; }
    .wallet-addr-label { font-size: 11px; color: var(--text-dim); text-transform: uppercase; letter-spacing: 1px; margin-bottom: 6px; }
    .wallet-addr { font-family: var(--font-mono); font-size: 12px; background: var(--bg); padding: 8px 12px; border-radius: 6px; border: 1px solid var(--border); word-break: break-all; display: flex; align-items: center; gap: 8px; }
    .copy-btn { background: none; border: none; color: var(--accent); cursor: pointer; font-size: 12px; padding: 2px 8px; border-radius: 4px; border: 1px solid var(--accent); white-space: nowrap; font-family: var(--font); transition: 0.2s; }
    .copy-btn:hover { background: var(--accent-dim); }

    .logs-box { background: #060402; border: 1px solid var(--border); border-radius: var(--radius); padding: 16px; font-family: var(--font-mono); font-size: 12px; max-height: 300px; overflow-y: auto; white-space: pre-wrap; color: var(--text-dim); line-height: 1.8; }

    .wallet-addr { font-family: var(--font-mono); font-size: 13px; background: var(--bg); padding: 8px 12px; border-radius: 6px; border: 1px solid var(--border); word-break: break-all; display: flex; align-items: center; gap: 8px; }
    .copy-btn { background: none; border: none; color: var(--accent); cursor: pointer; font-size: 14px; padding: 2px 6px; }
    .copy-btn:hover { opacity: 0.7; }

    .toast { position: fixed; bottom: 24px; right: 24px; background: var(--bg-panel); border: 1px solid var(--accent); color: var(--accent); padding: 12px 20px; border-radius: 8px; font-size: 13px; opacity: 0; transition: 0.3s; z-index: 1000; }
    .toast.show { opacity: 1; }

    .quick-actions { display: grid; grid-template-columns: repeat(2, 1fr); gap: 10px; margin: 16px 0 0; }
    .quick-action { background: var(--bg-panel); border: 1px solid var(--border); border-radius: var(--radius); padding: 14px 16px; cursor: pointer; transition: 0.2s; display: flex; align-items: center; gap: 10px; font-size: 13px; color: var(--text); }
    .quick-action:hover { border-color: var(--accent); color: var(--accent); transform: translateY(-1px); }
    .quick-action .qa-icon { font-size: 18px; width: 28px; text-align: center; }
    .quick-action .qa-label { font-weight: 500; }
    .quick-action .qa-hint { font-size: 11px; color: var(--text-dim); margin-top: 2px; }
    .quick-action.warn:hover { border-color: var(--danger); color: var(--danger); }

    .modal-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.7); display: none; align-items: center; justify-content: center; z-index: 200; }
    .modal-overlay.open { display: flex; }
    .modal { background: var(--bg-panel); border: 1px solid var(--border); border-radius: var(--radius); padding: 28px; max-width: 520px; width: 90%; box-shadow: var(--shadow); }
    .modal-title { font-size: 16px; font-weight: 700; margin-bottom: 12px; }
    .modal-warn { background: rgba(255,85,85,0.1); border: 1px solid var(--danger); border-radius: 6px; padding: 10px 14px; font-size: 12px; color: var(--danger); margin-bottom: 14px; line-height: 1.5; }
    .modal-key { font-family: var(--font-mono); font-size: 13px; background: var(--bg); padding: 12px; border-radius: 6px; border: 1px solid var(--border); word-break: break-all; line-height: 1.6; user-select: all; }
    .modal-actions { display: flex; gap: 8px; margin-top: 16px; justify-content: flex-end; }
    .modal-close { background: none; border: 1px solid var(--border); color: var(--text-dim); padding: 8px 16px; border-radius: 6px; cursor: pointer; font-size: 13px; }
    .modal-close:hover { border-color: var(--text); color: var(--text); }
    .modal-input { width: 100%; box-sizing: border-box; background: var(--bg); border: 1px solid var(--border); border-radius: 6px; padding: 10px 12px; font-size: 14px; color: var(--text); font-family: var(--font-mono); outline: none; margin-top: 4px; }
    .modal-input:focus { border-color: var(--accent); }
    .modal-hint { font-size: 12px; color: var(--text-dim); margin-top: 6px; line-height: 1.5; }

    .upgrade-banner { padding: 14px 16px; border-radius: var(--radius); margin-bottom: 16px; font-size: 13px; line-height: 1.6; display: flex; justify-content: space-between; align-items: center; gap: 12px; }
    .upgrade-banner.recommended { background: rgba(255, 170, 0, 0.1); border: 1px solid rgba(255, 170, 0, 0.3); color: #ffaa00; }
    .upgrade-banner.recommended .upgrade-text { flex: 1; }
    .upgrade-banner.recommended .upgrade-actions { display: flex; gap: 8px; }
    .upgrade-banner.required { background: rgba(255, 140, 0, 0.1); border: 1px solid rgba(255, 140, 0, 0.3); color: #ff8c3a; }
    .upgrade-banner.required .upgrade-text { flex: 1; }
    .upgrade-banner.required .upgrade-actions { display: flex; gap: 8px; }
    .upgrade-banner.critical { background: rgba(239, 68, 68, 0.15); border: 1px solid rgba(239, 68, 68, 0.4); color: var(--danger); width: 100%; margin-left: calc(-20px - 1px); margin-right: calc(-20px - 1px); padding: 16px calc(20px + 1px); border-radius: 0; font-weight: 600; }
    .upgrade-banner.critical .upgrade-text { flex: 1; }
    .upgrade-banner.critical .upgrade-actions { display: flex; gap: 8px; }
    .upgrade-btn { background: var(--accent); color: var(--bg); border: none; padding: 6px 12px; border-radius: 4px; font-size: 12px; cursor: pointer; font-weight: 600; transition: 0.2s; }
    .upgrade-btn:hover { opacity: 0.85; }
    .upgrade-dismiss { background: none; border: 1px solid currentColor; color: currentColor; padding: 4px 10px; border-radius: 4px; font-size: 12px; cursor: pointer; transition: 0.2s; }
    .upgrade-dismiss:hover { opacity: 0.7; }
  </style>
</head>
<body>
  <header class="header">
    <div class="container">
      <div style="display:flex;align-items:center;gap:14px">
        <div class="logo"><img src="https://explorer.clawlabz.xyz/favicon.png" style="width:28px;height:28px;border-radius:6px;vertical-align:middle;margin-right:8px"><span class="logo-claw">Claw</span><span class="logo-net">Network</span></div>
        <span class="header-badge">Node Dashboard</span>
      </div>
      <span id="lastUpdate" style="font-size:12px;color:var(--text-dim)"></span>
    </div>
  </header>

  <main class="container" style="padding-top:16px;padding-bottom:40px">

    <div id="upgradeBanner" style="display:none" class="upgrade-banner"></div>

    <div class="panel">
      <div class="panel-title">Node</div>
      <div class="stats-grid" style="margin:0 0 4px">
        <div class="stat-card">
          <div class="stat-label">Status</div>
          <div class="stat-value" id="statusValue"><span class="status-dot offline"></span>Offline</div>
        </div>
        <div class="stat-card">
          <div class="stat-label">Block Height</div>
          <div class="stat-value accent" id="heightValue">—</div>
        </div>
        <div class="stat-card">
          <div class="stat-label">Peers</div>
          <div class="stat-value" id="peersValue">—</div>
        </div>
        <div class="stat-card">
          <div class="stat-label">Uptime</div>
          <div class="stat-value" id="uptimeValue">—</div>
        </div>
      </div>
      <div class="node-controls">
        <button class="btn primary" id="startBtn" onclick="doAction('start')">&#x25B6; Start Node</button>
        <button class="btn" id="restartBtn" onclick="doRestart()" style="background:var(--accent);color:#000;font-weight:600">&#x21BB; Restart</button>
        <button class="btn danger" id="stopBtn" onclick="doAction('stop')">&#x25A0; Stop Node</button>
      </div>
    </div>

    <div class="panel" id="walletPanel">
      <div class="panel-title">Wallet</div>
      <div id="walletEmpty" style="color:var(--text-dim);font-size:13px">No wallet yet — start the node to generate one</div>
      <div id="walletLoaded" style="display:none">
        <div class="wallet-hero">
          <div>
            <div class="wallet-balance-label">Balance</div>
            <div class="wallet-balance" id="walletBalance">—</div>
          </div>
          <div class="wallet-addr-wrap">
            <div class="wallet-addr-label">Address</div>
            <div class="wallet-addr"><span id="walletAddrText" style="flex:1;min-width:0;word-break:break-all"></span><button class="copy-btn" onclick="copyText(cachedAddress)">Copy</button></div>
          </div>
        </div>
        <div class="quick-actions" id="walletActions">
          <div class="quick-action" onclick="importToExtension()" id="qaImportExt">
            <span class="qa-icon"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg></span>
            <div><div class="qa-label">Import to Extension</div><div class="qa-hint" id="qaImportHint">One-click import to browser wallet</div></div>
          </div>
          <div class="quick-action warn" onclick="showExportKey()">
            <span class="qa-icon"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="7.5" cy="15.5" r="5.5"/><path d="m21 2-9.6 9.6"/><path d="m15.5 7.5 3 3L22 7l-3-3"/></svg></span>
            <div><div class="qa-label">Export Private Key</div><div class="qa-hint">Manual copy for backup</div></div>
          </div>
          <div class="quick-action" onclick="openExplorer()">
            <span class="qa-icon"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.35-4.35"/></svg></span>
            <div><div class="qa-label">View on Explorer</div><div class="qa-hint">Transaction history</div></div>
          </div>
          <div class="quick-action" id="qaRegister" onclick="handleRegisterAgent()">
            <span class="qa-icon"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="4" width="16" height="16" rx="2"/><rect x="9" y="9" width="6" height="6"/><path d="M15 2v2M9 2v2M15 20v2M9 20v2M2 15h2M2 9h2M20 15h2M20 9h2"/></svg></span>
            <div><div class="qa-label" id="qaRegisterLabel">Register Agent</div><div class="qa-hint" id="qaRegisterHint">On-chain identity</div></div>
          </div>
        </div>
      </div>
    </div>

    <div class="panel">
      <div class="panel-title">Node Info</div>
      <div id="nodeInfo">Loading...</div>
    </div>

    <div class="panel">
      <div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:12px">
        <div class="panel-title" style="margin-bottom:0">Recent Logs</div>
        <button class="btn" style="font-size:12px;padding:5px 12px" onclick="refreshLogs()">&#x21BB; Refresh</button>
      </div>
      <div class="logs-box" id="logsBox">Loading...</div>
    </div>
  </main>

  <footer style="border-top:1px solid var(--border);padding:24px 0;margin-top:16px">
    <div class="container" style="display:flex;flex-wrap:wrap;gap:20px;align-items:center;justify-content:space-between">
      <div style="display:flex;align-items:center;gap:8px">
        <img src="https://explorer.clawlabz.xyz/favicon.png" style="width:18px;height:18px;border-radius:4px;opacity:0.7">
        <span style="font-size:12px;color:var(--text-dim)">© 2026 ClawLabz</span>
      </div>
      <div style="display:flex;gap:20px;flex-wrap:wrap">
        <a href="https://chain.clawlabz.xyz" target="_blank" style="font-size:12px;color:var(--text-dim);text-decoration:none;transition:0.2s" onmouseover="this.style.color='var(--accent)'" onmouseout="this.style.color='var(--text-dim)'">Chain</a>
        <a href="https://explorer.clawlabz.xyz" target="_blank" style="font-size:12px;color:var(--text-dim);text-decoration:none;transition:0.2s" onmouseover="this.style.color='var(--accent)'" onmouseout="this.style.color='var(--text-dim)'">Explorer</a>
        <a href="https://chrome.google.com/webstore/search/ClawNetwork" target="_blank" style="font-size:12px;color:var(--text-dim);text-decoration:none;transition:0.2s" onmouseover="this.style.color='var(--accent)'" onmouseout="this.style.color='var(--text-dim)'">Wallet Extension</a>
      </div>
    </div>
  </footer>

  <div class="toast" id="toast"></div>

  <div class="modal-overlay" id="registerModal" onclick="if(event.target===this)closeRegisterModal()">
    <div class="modal">
      <div class="modal-title">Register Agent</div>
      <p style="font-size:13px;color:var(--text-dim);margin:0 0 12px">Register your wallet as an AI Agent on ClawNetwork. The name is your on-chain identity — it does not need to be unique globally (the wallet address is what's unique). Registration is gas-free on mainnet.</p>
      <input id="registerNameInput" class="modal-input" type="text" placeholder="my-agent-name" maxlength="32" onkeydown="if(event.key==='Enter')submitRegisterAgent()" />
      <div class="modal-hint">Allowed: letters, numbers, hyphens, underscores. Max 32 chars.</div>
      <div class="modal-actions">
        <button class="modal-close" onclick="closeRegisterModal()">Cancel</button>
        <button class="btn primary" onclick="submitRegisterAgent()">Register</button>
      </div>
    </div>
  </div>

  <div class="modal-overlay" id="installModal" onclick="if(event.target===this)closeInstallModal()">
    <div class="modal">
      <div class="modal-title">Install ClawNetwork Wallet</div>
      <p style="font-size:13px;color:var(--text-dim);margin:0 0 16px;line-height:1.6">The ClawNetwork browser extension is not detected. Install it first, then click Import to Extension to import your node wallet.</p>
      <div style="display:flex;gap:10px;flex-direction:column">
        <a href="https://chrome.google.com/webstore/search/ClawNetwork" target="_blank" class="btn primary" style="text-decoration:none;justify-content:center;padding:10px 16px">Open Chrome Web Store</a>
        <a href="https://chain.clawlabz.xyz" target="_blank" style="font-size:12px;color:var(--text-dim);text-decoration:none;text-align:center" onmouseover="this.style.color='var(--accent)'" onmouseout="this.style.color='var(--text-dim)'">Learn more at chain.clawlabz.xyz →</a>
      </div>
      <div class="modal-actions" style="margin-top:16px">
        <button class="modal-close" onclick="closeInstallModal()">Close</button>
      </div>
    </div>
  </div>

  <div class="modal-overlay" id="exportModal" onclick="if(event.target===this)closeExportModal()">
    <div class="modal">
      <div class="modal-title">Export Private Key</div>
      <div class="modal-warn">
        &#x26A0;&#xFE0F; <strong>Never share your private key.</strong> Anyone with this key has full control of your wallet and funds. Only use this to import into your own browser extension or backup.
      </div>
      <div class="modal-key" id="exportKeyDisplay">Loading...</div>
      <div class="modal-actions">
        <button class="btn primary" onclick="copyExportKey()">Copy Private Key</button>
        <button class="modal-close" onclick="closeExportModal()">Close</button>
      </div>
    </div>
  </div>

  <script>
    const API = '';
    let autoRefresh = null;

    function toast(msg) {
      const el = document.getElementById('toast');
      el.textContent = msg;
      el.classList.add('show');
      setTimeout(() => el.classList.remove('show'), 3000);
    }

    let cachedAddress = '';
    let cachedNetwork = '';
    let cachedKey = '';
    let cachedAgentName = '';   // '' = not registered, string = registered name

    function copyText(text) {
      navigator.clipboard.writeText(text).then(() => toast('Copied!')).catch(() => {});
    }

    function copyAddress() {
      if (!cachedAddress) { toast('No wallet address'); return; }
      copyText(cachedAddress);
      toast('Address copied!');
    }

    async function showExportKey() {
      document.getElementById('exportKeyDisplay').textContent = 'Loading...';
      document.getElementById('exportModal').classList.add('open');
      try {
        const res = await fetch(API + '/api/wallet/export');
        const data = await res.json();
        if (data.error) { document.getElementById('exportKeyDisplay').textContent = data.error; return; }
        cachedKey = data.secretKey;
        document.getElementById('exportKeyDisplay').textContent = data.secretKey;
      } catch (e) { document.getElementById('exportKeyDisplay').textContent = 'Failed to load'; }
    }

    function closeExportModal() {
      document.getElementById('exportModal').classList.remove('open');
      cachedKey = '';
      document.getElementById('exportKeyDisplay').textContent = '';
    }

    function copyExportKey() {
      if (!cachedKey) return;
      copyText(cachedKey);
      toast('Private key copied! Paste into browser extension to import.');
    }

    function openExplorer() {
      if (!cachedAddress) { toast('No wallet address'); return; }
      window.open('https://explorer.clawlabz.xyz/address/' + cachedAddress, '_blank');
    }

    function openFaucet() {
      window.open('https://chain.clawlabz.xyz/faucet', '_blank');
    }

    // Detect ClawNetwork extension provider (for enhanced flow when available)
    let hasExtension = false;
    function checkExtension() {
      if (window.clawNetwork && window.clawNetwork.isClawNetwork) {
        hasExtension = true;
      }
    }
    checkExtension();
    setTimeout(checkExtension, 1000);
    setTimeout(checkExtension, 3000);

    async function importToExtension() {
      // Try externally_connectable direct channel first (bypasses page JS context)
      const extIds = await detectExtensionIds();
      if (extIds.length > 0) {
        toast('Connecting to extension (secure channel)...');
        try {
          const res = await fetch(API + '/api/wallet/export');
          const data = await res.json();
          if (!data.secretKey) { toast('No private key found'); return; }
          // Direct to background — private key never in page JS event loop
          const extId = extIds[0];
          await chromeExtSend(extId, { method: 'claw_requestAccounts' });
          toast('Approve the import in your extension popup...');
          await chromeExtSend(extId, { method: 'claw_importAccountKey', params: [data.secretKey, 'ClawNetwork Node'] });
          toast('Account imported to extension!');
          return;
        } catch (e) { /* fall through to provider method */ }
      }
      // Fallback: use window.clawNetwork provider
      if (!window.clawNetwork) {
        document.getElementById('installModal').classList.add('open');
        return;
      }
      toast('Connecting to extension...');
      try {
        await window.clawNetwork.request({ method: 'claw_requestAccounts' });
        const res = await fetch(API + '/api/wallet/export');
        const data = await res.json();
        if (!data.secretKey) { toast('No private key found'); return; }
        toast('Approve the import in your extension popup...');
        await window.clawNetwork.request({ method: 'claw_importAccountKey', params: [data.secretKey, 'ClawNetwork Node'] });
        toast('Account imported to extension!');
      } catch (e) { toast('Import failed: ' + (e.message || e)); }
    }

    function chromeExtSend(extId, msg) {
      return new Promise((resolve, reject) => {
        if (!chrome || !chrome.runtime || !chrome.runtime.sendMessage) { reject(new Error('No chrome.runtime')); return; }
        chrome.runtime.sendMessage(extId, msg, (response) => {
          if (chrome.runtime.lastError) { reject(new Error(chrome.runtime.lastError.message)); return; }
          if (response && response.success === false) { reject(new Error(response.error || 'Failed')); return; }
          resolve(response);
        });
      });
    }

    async function detectExtensionIds() {
      // Try known extension IDs or probe for externally_connectable
      // In production, the extension ID is stable after Chrome Web Store publish
      // For dev, try to detect via management API or stored ID
      const ids = [];
      try {
        if (chrome && chrome.runtime && chrome.runtime.sendMessage) {
          // Try sending a ping to see if any extension responds
          // This requires knowing the extension ID. For now, check localStorage.
          const stored = localStorage.getItem('clawnetwork_extension_id');
          if (stored) ids.push(stored);
        }
      } catch {}
      return ids;
    }

    async function transferFromDashboard() {
      const to = prompt('Recipient address (64 hex chars):');
      if (!to) return;
      const amount = prompt('Amount (CLAW):');
      if (!amount) return;
      if (window.clawNetwork) {
        try {
          toast('Approve transfer in extension...');
          await window.clawNetwork.request({ method: 'claw_requestAccounts' });
          const result = await window.clawNetwork.request({ method: 'claw_transfer', params: [to, amount] });
          toast('Transfer sent! Hash: ' + (result && result.txHash ? result.txHash.slice(0, 16) + '...' : 'submitted'));
        } catch (e) { toast('Transfer failed: ' + (e.message || e)); }
      } else {
        try {
          const res = await fetch(API + '/api/transfer', { method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify({to, amount}) });
          const data = await res.json();
          toast(data.ok ? 'Transfer sent! Hash: ' + (data.txHash || '').slice(0, 16) + '...' : 'Error: ' + data.error);
        } catch (e) { toast('Transfer failed: ' + e.message); }
      }
      setTimeout(fetchStatus, 3000);
    }

    function closeInstallModal() {
      document.getElementById('installModal').classList.remove('open');
    }

    function handleRegisterAgent() {
      if (cachedAgentName) {
        toast('Already registered as "' + cachedAgentName + '"');
        return;
      }
      openRegisterModal();
    }

    function openRegisterModal() {
      document.getElementById('registerNameInput').value = '';
      document.getElementById('registerModal').classList.add('open');
      setTimeout(() => document.getElementById('registerNameInput').focus(), 50);
    }

    function closeRegisterModal() {
      document.getElementById('registerModal').classList.remove('open');
    }

    async function submitRegisterAgent() {
      const raw = document.getElementById('registerNameInput').value.trim();
      const name = raw.replace(/[^a-zA-Z0-9_-]/g, '').slice(0, 32);
      if (!name) { toast('Please enter an agent name'); return; }
      closeRegisterModal();
      if (window.clawNetwork) {
        try {
          toast('Approve registration in extension...');
          await window.clawNetwork.request({ method: 'claw_requestAccounts' });
          await window.clawNetwork.request({ method: 'claw_registerAgent', params: [name] });
          toast('Agent "' + name + '" registered!');
        } catch (e) { toast('Registration failed: ' + (e.message || e)); }
      } else {
        try {
          const res = await fetch(API + '/api/agent/register', { method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify({name}) });
          const data = await res.json();
          toast(data.ok ? 'Agent "' + name + '" registered!' : 'Error: ' + data.error);
        } catch (e) { toast('Registration failed: ' + e.message); }
      }
    }

    async function fetchStatus() {
      try {
        const res = await fetch(API + '/api/status');
        const data = await res.json();
        renderStatus(data);
        document.getElementById('lastUpdate').textContent = 'Updated: ' + new Date().toLocaleTimeString();
      } catch (e) {
        console.error(e);
        renderStatus({ running: false, blockHeight: null, peerCount: null, walletAddress: '', network: 'mainnet', syncMode: 'light', rpcUrl: 'http://localhost:19877', pluginVersion: '${VERSION}', restartCount: 0, dataDir: '', balance: '', syncing: false, uptimeFormatted: '—', pid: null });
      }
    }

    function renderStatus(s) {
      const statusEl = document.getElementById('statusValue');
      if (s.running) {
        let dotClass = 'online', label = 'Online';
        if (s.syncing && s.peerless) { dotClass = 'syncing'; label = 'No Peers'; }
        else if (s.syncing) { dotClass = 'syncing'; label = 'Syncing'; }
        statusEl.innerHTML = '<span class="status-dot ' + dotClass + '"></span>' + label;
        statusEl.className = 'stat-value' + (s.syncing ? '' : ' green');
      } else {
        statusEl.innerHTML = '<span class="status-dot offline"></span>Offline';
        statusEl.className = 'stat-value danger';
      }

      document.getElementById('heightValue').textContent = s.blockHeight !== null ? s.blockHeight.toLocaleString() : '—';
      document.getElementById('peersValue').textContent = s.peerCount !== null ? s.peerCount : '—';
      document.getElementById('uptimeValue').textContent = s.uptimeFormatted || '—';
      document.getElementById('startBtn').disabled = s.running;
      document.getElementById('stopBtn').disabled = !s.running;
      document.getElementById('startBtn').style.opacity = s.running ? '0.4' : '1';
      document.getElementById('stopBtn').style.opacity = !s.running ? '0.4' : '1';

      // Handle upgrade banner
      const bannerEl = document.getElementById('upgradeBanner');
      if (s.upgradeLevel && s.upgradeLevel !== 'up_to_date' && s.upgradeLevel !== 'unknown') {
        bannerEl.style.display = '';
        const recommended = s.upgradeLevel === 'recommended';
        const required = s.upgradeLevel === 'required';
        const critical = s.upgradeLevel === 'critical';
        bannerEl.className = 'upgrade-banner ' + s.upgradeLevel;
        let bannerHtml = '<div class="upgrade-text">';
        if (critical) {
          bannerHtml += '⚠ CRITICAL UPDATE REQUIRED — ' + (s.changelog || 'Security update required') + '. Node stopped for security.';
        } else if (required) {
          bannerHtml += 'Update recommended: v' + (s.latestVersion || '') + ' — ' + (s.changelog || 'Update available');
        } else if (recommended) {
          bannerHtml += 'Update available: v' + (s.latestVersion || '') + ' — ' + (s.changelog || 'New version available');
        }
        bannerHtml += '</div><div class="upgrade-actions">';
        bannerHtml += '<button class="upgrade-btn" id="upgradeBtn">Update Now</button>';
        if (recommended) {
          bannerHtml += '<button class="upgrade-dismiss" id="dismissBtn">Dismiss</button>';
        }
        bannerHtml += '</div>';
        bannerEl.innerHTML = bannerHtml;
        var ubtn = document.getElementById('upgradeBtn');
        if (ubtn) ubtn.onclick = function() { doAction('upgrade'); };
        var dbtn = document.getElementById('dismissBtn');
        if (dbtn) dbtn.onclick = function() { bannerEl.style.display = 'none'; };
      } else {
        bannerEl.style.display = 'none';
      }

      // Wallet
      cachedAddress = s.walletAddress || '';
      cachedNetwork = s.network || '';
      if (s.walletAddress) {
        document.getElementById('walletEmpty').style.display = 'none';
        document.getElementById('walletLoaded').style.display = '';
        document.getElementById('walletAddrText').textContent = s.walletAddress;
        document.getElementById('walletBalance').textContent = s.balance || '—';
        // Agent status
        cachedAgentName = s.agentName || '';
        const regCard = document.getElementById('qaRegister');
        const regLabel = document.getElementById('qaRegisterLabel');
        const regHint = document.getElementById('qaRegisterHint');
        if (cachedAgentName) {
          regLabel.textContent = 'Agent Registered';
          regHint.innerHTML = '<span style="color:var(--green)">&#x2713; ' + cachedAgentName + '</span>';
          regCard.style.borderColor = 'var(--green)';
          regCard.style.opacity = '0.85';
        } else {
          regLabel.textContent = 'Register Agent';
          regHint.textContent = 'On-chain identity';
          regCard.style.borderColor = '';
          regCard.style.opacity = '';
        }
        // Extension detection hint
        const hasExt = !!(window.clawNetwork && window.clawNetwork.isClawNetwork);
        document.getElementById('qaImportHint').textContent = hasExt ? 'Extension detected — click to import' : 'Install wallet extension first';
      } else {
        document.getElementById('walletEmpty').style.display = '';
        document.getElementById('walletLoaded').style.display = 'none';
      }

      // Node info
      let versionStatusHtml = s.binaryVersion || '—';
      if (s.upgradeLevel === 'up_to_date') {
        versionStatusHtml = (s.binaryVersion || '—') + ' <span style="color:var(--green)">✓</span>';
      } else if (s.upgradeLevel === 'recommended') {
        versionStatusHtml = (s.binaryVersion || '—') + ' <span style="color:#ffaa00">→ ' + (s.latestVersion || '') + '</span>';
      } else if (s.upgradeLevel === 'required') {
        versionStatusHtml = (s.binaryVersion || '—') + ' <span style="color:#ff8c3a">⚠ Update recommended</span>';
      } else if (s.upgradeLevel === 'critical') {
        versionStatusHtml = (s.binaryVersion || '—') + ' <span style="color:var(--danger)">🔴 CRITICAL</span>';
      }
      const rows = [
        ['Network', s.network],
        ['Sync Mode', s.syncMode],
        ['RPC URL', s.rpcUrl],
        ['Binary Version', versionStatusHtml],
        ['Plugin Version', s.pluginVersion],
        ['PID', s.pid || '—'],
        ['Restart Count', s.restartCount],
        ['Data Dir', s.dataDir],
      ];
      document.getElementById('nodeInfo').innerHTML = rows.map(function(r) {
        return '<div class="info-row"><span class="info-label">' + r[0] + '</span><span class="info-value">' + r[1] + '</span></div>';
      }).join('');
    }

    async function doAction(action) {
      try {
        const res = await fetch(API + '/api/action/' + action, { method: 'POST' });
        const data = await res.json();
        toast(data.message || data.error || 'Done');
        setTimeout(fetchStatus, 1500);
      } catch (e) { toast('Error: ' + e.message); }
    }

    async function doRestart() {
      toast('Restarting node...');
      try {
        var res = await fetch(API + '/api/action/restart', { method: 'POST' });
        var data = await res.json();
        toast(data.message || data.error || 'Done');
        setTimeout(fetchStatus, 3000);
      } catch (e) { toast('Error: ' + e.message); }
    }

    async function refreshLogs() {
      try {
        const res = await fetch(API + '/api/logs');
        const data = await res.json();
        const box = document.getElementById('logsBox');
        box.textContent = data.logs || 'No logs yet';
        box.scrollTop = box.scrollHeight;
      } catch (e) { document.getElementById('logsBox').textContent = 'Failed to load logs'; }
    }

    fetchStatus();
    refreshLogs();
    autoRefresh = setInterval(fetchStatus, 10000);
  </script>
</body>
</html>`
}

// ── UI Server (standalone script, forked as background process) ──

const UI_SERVER_SCRIPT = `
const http = require('http');
const fs = require('fs');
const os = require('os');
const path = require('path');

// OPENCLAW_BASE_DIR and PLUGIN_VERSION are injected as const by startUiServer() prepend.
// Use global lookup to avoid const/var redeclaration conflict.
const _BASE = (typeof OPENCLAW_BASE_DIR !== 'undefined') ? OPENCLAW_BASE_DIR : path.join(os.homedir(), '.openclaw');
const _PVER = (typeof PLUGIN_VERSION !== 'undefined') ? PLUGIN_VERSION : 'unknown';
const OC_WORKSPACE = path.join(_BASE, 'workspace', 'clawnetwork');
const OC_BIN_DIR = path.join(_BASE, 'bin');
const OC_WALLET_PATH = path.join(OC_WORKSPACE, 'wallet.json');
const OC_PID_FILE = path.join(OC_WORKSPACE, 'node.pid');
const OC_CONFIG_PATH = path.join(OC_WORKSPACE, 'config.json');
const OC_STOP_SIGNAL = path.join(OC_WORKSPACE, 'stop.signal');
const OC_LOG_PATH_DEFAULT = path.join(OC_WORKSPACE, 'node.log');
const OC_DATA_DIR = path.join(OC_WORKSPACE, 'chain-data');

const PORT = parseInt(process.argv[2] || '19877', 10);
const RPC_PORT = parseInt(process.argv[3] || '9710', 10);
const LOG_PATH = process.argv[4] || OC_LOG_PATH_DEFAULT;
const PORT_FILE = path.join(_BASE, 'clawnetwork-ui-port');
const MAX_RETRIES = 10;

async function fetchJson(url) {
  const r = await fetch(url);
  return r.json();
}

async function rpcCall(method, params) {
  const r = await fetch('http://localhost:' + RPC_PORT, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', method, params: params || [], id: Date.now() }),
  });
  const d = await r.json();
  if (d.error) throw new Error(d.error.message || JSON.stringify(d.error));
  return d.result;
}

function formatClaw(raw) {
  const v = BigInt(raw);
  const ONE = BigInt(1e9);
  const w = v / ONE;
  const f = v % ONE;
  if (f === 0n) return w + ' CLAW';
  return w + '.' + f.toString().padStart(9, '0').replace(/0+$/, '') + ' CLAW';
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let data = '';
    req.on('data', (chunk) => { data += chunk; });
    req.on('end', () => { try { resolve(JSON.parse(data || '{}')); } catch { resolve({}); } });
    req.on('error', reject);
    setTimeout(() => reject(new Error('Body read timeout')), 10000);
  });
}

function findNodeBinary() {
  const binName = process.platform === 'win32' ? 'claw-node.exe' : 'claw-node';
  let binary = path.join(OC_BIN_DIR, binName);
  if (fs.existsSync(binary)) return binary;
  binary = path.join(os.homedir(), '.clawnetwork', 'bin', 'claw-node');
  if (fs.existsSync(binary)) return binary;
  return null;
}

async function handle(req, res) {
  const url = new URL(req.url, 'http://localhost:' + PORT);
  const p = url.pathname;
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');
  if (req.method === 'OPTIONS') { res.writeHead(204); res.end(); return; }

  const json = (s, d) => { res.writeHead(s, { 'content-type': 'application/json' }); res.end(JSON.stringify(d)); };

  if (p === '/' || p === '/index.html') {
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
    res.end(HTML);
    return;
  }
  if (p === '/api/status') {
    try {
      const h = await fetchJson('http://localhost:' + RPC_PORT + '/health');
      let balance = '';
      let walletAddress = '';
      let agentName = '';
      let upgradeLevel = 'unknown';
      let latestVersion = '';
      let releaseUrl = '';
      let changelog = '';
      let announcement = null;
      // Fetch version info if available (Phase 1 endpoint)
      try {
        const v = await fetchJson('http://localhost:' + RPC_PORT + '/version');
        if (v && v.upgrade_level) {
          upgradeLevel = v.upgrade_level;
          latestVersion = v.latest_version || '';
          releaseUrl = v.release_url || '';
          changelog = v.changelog || '';
          announcement = v.announcement || null;
        }
      } catch {}
      try {
        const walletPath = OC_WALLET_PATH;
        const w = JSON.parse(fs.readFileSync(walletPath, 'utf8'));
        walletAddress = w.address || '';
        if (w.address) {
          const b = await rpcCall('claw_getBalance', [w.address]); balance = formatClaw(b);
          try { const ag = await rpcCall('claw_getAgent', [w.address]); agentName = (ag && ag.name) ? ag.name : ''; } catch {}
        }
      } catch {}
      json(200, {
        running: h.status === 'ok' || h.status === 'degraded',
        blockHeight: h.height,
        peerCount: h.peer_count,
        network: h.chain_id,
        syncMode: 'light',
        rpcUrl: 'http://localhost:' + RPC_PORT,
        walletAddress,
        binaryVersion: h.version,
        pluginVersion: _PVER,
        uptime: h.uptime_secs,
        uptimeFormatted: h.uptime_secs < 60 ? h.uptime_secs + 's' : h.uptime_secs < 3600 ? Math.floor(h.uptime_secs/60) + 'm' : Math.floor(h.uptime_secs/3600) + 'h ' + Math.floor((h.uptime_secs%3600)/60) + 'm',
        restartCount: 0, dataDir: path.join(os.homedir(), '.clawnetwork'), balance, agentName, syncing: h.status === 'degraded', peerless: h.peer_count === 0, lastBlockAgeSecs: h.last_block_age_secs,
        upgradeLevel, latestVersion, releaseUrl, changelog, announcement,
      });
    } catch {
        const walletAddr = (() => { try { return JSON.parse(fs.readFileSync(OC_WALLET_PATH, 'utf8')).address; } catch { return ''; } })();
        json(200, { running: false, blockHeight: null, peerCount: null, walletAddress: walletAddr, network: 'mainnet', syncMode: 'light', rpcUrl: 'http://localhost:' + RPC_PORT, pluginVersion: _PVER, restartCount: 0, dataDir: path.join(os.homedir(), '.clawnetwork'), balance: '', agentName: '', syncing: false, uptimeFormatted: '—', pid: null, upgradeLevel: 'unknown', latestVersion: '', releaseUrl: '', changelog: '', announcement: null });
      }
    return;
  }
  if (p === '/api/logs') {
    try {
      if (!fs.existsSync(LOG_PATH)) { json(200, { logs: 'No logs yet' }); return; }
      const c = fs.readFileSync(LOG_PATH, 'utf8').split('\\n');
      json(200, { logs: c.slice(-80).join('\\n') });
    } catch (e) { json(500, { error: e.message }); }
    return;
  }
  if (p === '/api/wallet/export') {
    // Only allow from localhost (127.0.0.1) — never expose to network
    const host = req.headers.host || '';
    if (!host.startsWith('127.0.0.1') && !host.startsWith('localhost')) {
      json(403, { error: 'Wallet export only available from localhost' });
      return;
    }
    try {
      const walletPath = OC_WALLET_PATH;
      const w = JSON.parse(fs.readFileSync(walletPath, 'utf8'));
      json(200, { address: w.address, secretKey: w.secret_key || w.secretKey || w.private_key || '' });
    } catch (e) { json(400, { error: 'No wallet found' }); }
    return;
  }
  // ── Business API endpoints (mirrors Gateway methods) ──
  if (p === '/api/wallet/balance') {
    try {
      const walletPath = OC_WALLET_PATH;
      const w = JSON.parse(fs.readFileSync(walletPath, 'utf8'));
      const address = new URL(req.url, 'http://localhost').searchParams.get('address') || w.address;
      const b = await rpcCall('claw_getBalance', [address]);
      json(200, { address, balance: String(b), formatted: formatClaw(b) });
    } catch (e) { json(400, { error: e.message }); }
    return;
  }
  if (p === '/api/transfer' && req.method === 'POST') {
    try {
      const body = await readBody(req);
      const { to, amount } = body;
      if (!to || !amount) { json(400, { error: 'Missing params: to, amount' }); return; }
      if (!/^[0-9a-f]{64}$/i.test(to)) { json(400, { error: 'Invalid address (64 hex chars)' }); return; }
      if (!/^\\d+(\\.\\d+)?$/.test(amount) || parseFloat(amount) <= 0) { json(400, { error: 'Invalid amount' }); return; }
      const bin = findNodeBinary();
      if (!bin) { json(400, { error: 'claw-node binary not found' }); return; }
      const { execFileSync } = require('child_process');
      const out = execFileSync(bin, ['transfer', to, amount, '--rpc', 'http://localhost:' + RPC_PORT, '--data-dir', OC_DATA_DIR], { encoding: 'utf8', timeout: 30000, env: { HOME: os.homedir(), PATH: process.env.PATH || '' } });
      const h = out.match(/[0-9a-f]{64}/i);
      json(200, { ok: true, txHash: h ? h[0] : '', to, amount });
    } catch (e) { json(500, { error: e.message }); }
    return;
  }
  if (p === '/api/stake' && req.method === 'POST') {
    try {
      const body = await readBody(req);
      const { amount, action } = body;
      if (!amount && action !== 'claim') { json(400, { error: 'Missing amount' }); return; }
      const bin = findNodeBinary();
      if (!bin) { json(400, { error: 'claw-node binary not found' }); return; }
      const { execFileSync } = require('child_process');
      const cmd = action === 'withdraw' ? 'unstake' : action === 'claim' ? 'claim-stake' : 'stake';
      const args = [cmd].concat(amount ? [amount] : []).concat(['--rpc', 'http://localhost:' + RPC_PORT, '--data-dir', OC_DATA_DIR]);
      const out = execFileSync(bin, args, { encoding: 'utf8', timeout: 30000, env: { HOME: os.homedir(), PATH: process.env.PATH || '' } });
      json(200, { ok: true, raw: out.trim() });
    } catch (e) { json(500, { error: e.message }); }
    return;
  }
  if (p === '/api/agent/register' && req.method === 'POST') {
    try {
      const body = await readBody(req);
      const name = (body.name || 'openclaw-agent-' + Date.now().toString(36)).replace(/[^a-zA-Z0-9_-]/g, '').slice(0, 32);
      const bin = findNodeBinary();
      if (!bin) { json(400, { error: 'claw-node binary not found' }); return; }
      const { execFileSync } = require('child_process');
      const out = execFileSync(bin, ['register-agent', '--name', name, '--rpc', 'http://localhost:' + RPC_PORT, '--data-dir', OC_DATA_DIR], { encoding: 'utf8', timeout: 30000, env: { HOME: os.homedir(), PATH: process.env.PATH || '' } });
      const h = out.match(/[0-9a-f]{64}/i);
      json(200, { ok: true, txHash: h ? h[0] : '', name });
    } catch (e) { json(500, { error: e.message }); }
    return;
  }
  if (p === '/api/service/register' && req.method === 'POST') {
    try {
      const body = await readBody(req);
      const { serviceType, endpoint, description, priceAmount } = body;
      if (!serviceType || !endpoint) { json(400, { error: 'Missing: serviceType, endpoint' }); return; }
      const bin = findNodeBinary();
      if (!bin) { json(400, { error: 'claw-node binary not found' }); return; }
      const { execFileSync } = require('child_process');
      const out = execFileSync(bin, ['register-service', '--service-type', serviceType, '--endpoint', endpoint, '--description', description || '', '--price', priceAmount || '0', '--rpc', 'http://localhost:' + RPC_PORT, '--data-dir', OC_DATA_DIR], { encoding: 'utf8', timeout: 30000, env: { HOME: os.homedir(), PATH: process.env.PATH || '' } });
      json(200, { ok: true, raw: out.trim() });
    } catch (e) { json(500, { error: e.message }); }
    return;
  }
  if (p === '/api/service/search') {
    try {
      const t = new URL(req.url, 'http://localhost').searchParams.get('type');
      const result = await rpcCall('claw_getServices', t ? [t] : []);
      json(200, { services: result });
    } catch (e) { json(500, { error: e.message }); }
    return;
  }
  if (p === '/api/node/config') {
    try {
      const cfgPath = OC_CONFIG_PATH;
      const cfg = fs.existsSync(cfgPath) ? JSON.parse(fs.readFileSync(cfgPath, 'utf8')) : {};
      json(200, { ...cfg, rpcPort: RPC_PORT, uiPort: PORT });
    } catch (e) { json(200, { rpcPort: RPC_PORT, uiPort: PORT }); }
    return;
  }
  if (p.startsWith('/api/action/') && req.method === 'POST') {
    const a = p.split('/').pop();
    if (a === 'faucet') {
      try {
        const w = JSON.parse(fs.readFileSync(OC_WALLET_PATH, 'utf8'));
        const r = await rpcCall('claw_faucet', [w.address]);
        json(200, { message: 'Faucet success', ...r });
      } catch (e) { json(400, { error: e.message }); }
      return;
    }
    if (a === 'start') {
      try {
        // Check if already running — PID file is the only authority
        const pidFile = OC_PID_FILE;
        try {
          const pid = parseInt(fs.readFileSync(pidFile, 'utf8').trim(), 10);
          if (pid > 0) {
            try { process.kill(pid, 0); json(200, { message: 'Node already running', pid }); return; } catch {
              // PID stale, clean up
              try { fs.unlinkSync(pidFile); } catch {}
            }
          }
        } catch {}
        // Find binary
        const binDir = OC_BIN_DIR;
        const dataDir = path.join(os.homedir(), '.clawnetwork');
        const binName = process.platform === 'win32' ? 'claw-node.exe' : 'claw-node';
        let binary = path.join(binDir, binName);
        if (!fs.existsSync(binary)) { binary = path.join(dataDir, 'bin', 'claw-node'); }
        if (!fs.existsSync(binary)) { json(400, { error: 'claw-node binary not found. Run: openclaw clawnetwork:download' }); return; }
        // Read config for network/ports
        const cfgPath = OC_CONFIG_PATH;
        let network = 'mainnet', p2pPort = 9711, syncMode = 'light', extraPeers = [];
        try {
          const cfg = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
          if (cfg.network) network = cfg.network;
          if (cfg.p2pPort) p2pPort = cfg.p2pPort;
          if (cfg.syncMode) syncMode = cfg.syncMode;
          if (cfg.extraBootstrapPeers) extraPeers = cfg.extraBootstrapPeers;
        } catch {}
        const bootstrapPeers = { mainnet: ['/ip4/178.156.162.162/tcp/9711', '/ip4/39.102.144.231/tcp/9711'], testnet: ['/ip4/178.156.162.162/tcp/9721', '/ip4/39.102.144.231/tcp/9721'], devnet: [] };
        const peers = [...(bootstrapPeers[network] || []), ...extraPeers];
        const args = ['start', '--network', network, '--rpc-port', String(RPC_PORT), '--p2p-port', String(p2pPort), '--sync-mode', syncMode, '--data-dir', OC_DATA_DIR, '--allow-genesis'];
        for (const peer of peers) { args.push('--bootstrap', peer); }
        // Spawn detached
        const logPath = OC_LOG_PATH_DEFAULT;
        const logFd = fs.openSync(logPath, 'a');
        const { spawn: nodeSpawn } = require('child_process');
        const child = nodeSpawn(binary, args, {
          stdio: ['ignore', logFd, logFd],
          detached: true,
          env: { HOME: os.homedir(), PATH: process.env.PATH || '/usr/local/bin:/usr/bin:/bin', RUST_LOG: process.env.RUST_LOG || 'claw=info' },
        });
        child.unref();
        fs.closeSync(logFd);
        fs.writeFileSync(pidFile, String(child.pid));
        // Remove stop signal if exists
        const stopFile = OC_STOP_SIGNAL;
        try { fs.unlinkSync(stopFile); } catch {}
        json(200, { message: 'Node started', pid: child.pid });
      } catch (e) { json(500, { error: e.message }); }
      return;
    }
    if (a === 'stop') {
      try {
        const pidFile = OC_PID_FILE;
        let pid = null;
        try { pid = parseInt(fs.readFileSync(pidFile, 'utf8').trim(), 10); } catch {}
        if (pid && pid > 0) {
          try { process.kill(pid, 'SIGTERM'); } catch {}
        }
        // Write stop signal for restart loop
        const stopFile = OC_STOP_SIGNAL;
        try { fs.writeFileSync(stopFile, String(Date.now())); } catch {}
        // Wait for our process to exit (max 5s), using PID-specific check
        if (pid && pid > 0) {
          for (let w = 0; w < 10; w++) {
            try { process.kill(pid, 0); } catch { break; }
            require('child_process').execSync('sleep 0.5', { timeout: 2000 });
          }
        }
        try { fs.unlinkSync(pidFile); } catch {}
        json(200, { message: 'Node stopped' });
      } catch (e) { json(500, { error: e.message }); }
      return;
    }
    if (a === 'restart') {
      // Stop, wait, start — all server-side
      try {
        const pidFile = OC_PID_FILE;
        try {
          const pid = parseInt(fs.readFileSync(pidFile, 'utf8').trim(), 10);
          if (pid > 0) try { process.kill(pid, 'SIGTERM'); } catch {}
        } catch {}
        const stopFile = OC_STOP_SIGNAL;
        try { fs.writeFileSync(stopFile, String(Date.now())); } catch {}
        // Wait for our process to exit (PID-specific)
        try {
          const stoppedPid = parseInt(fs.readFileSync(pidFile, 'utf8').trim(), 10);
          if (stoppedPid > 0) {
            for (let w = 0; w < 10; w++) {
              try { process.kill(stoppedPid, 0); } catch { break; }
              require('child_process').execSync('sleep 0.5', { timeout: 2000 });
            }
          }
        } catch {}
        try { fs.unlinkSync(pidFile); } catch {}
        try { fs.unlinkSync(stopFile); } catch {}
        // Now start (reuse start logic inline)
        const binDir = OC_BIN_DIR;
        const binName = process.platform === 'win32' ? 'claw-node.exe' : 'claw-node';
        let binary = path.join(binDir, binName);
        if (!fs.existsSync(binary)) { binary = path.join(os.homedir(), '.clawnetwork/bin/claw-node'); }
        if (!fs.existsSync(binary)) { json(400, { error: 'claw-node binary not found' }); return; }
        const cfgPath = OC_CONFIG_PATH;
        let network = 'mainnet', p2pPort = 9711, syncMode = 'light', extraPeers = [];
        try {
          const cfg = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
          if (cfg.network) network = cfg.network;
          if (cfg.p2pPort) p2pPort = cfg.p2pPort;
          if (cfg.syncMode) syncMode = cfg.syncMode;
          if (cfg.extraBootstrapPeers) extraPeers = cfg.extraBootstrapPeers;
        } catch {}
        const bootstrapPeers = { mainnet: ['/ip4/178.156.162.162/tcp/9711', '/ip4/39.102.144.231/tcp/9711'], testnet: ['/ip4/178.156.162.162/tcp/9721', '/ip4/39.102.144.231/tcp/9721'], devnet: [] };
        const peers = [...(bootstrapPeers[network] || []), ...extraPeers];
        const args = ['start', '--network', network, '--rpc-port', String(RPC_PORT), '--p2p-port', String(p2pPort), '--sync-mode', syncMode, '--data-dir', OC_DATA_DIR, '--allow-genesis'];
        for (const peer of peers) { args.push('--bootstrap', peer); }
        const logPath = OC_LOG_PATH_DEFAULT;
        const logFd = fs.openSync(logPath, 'a');
        const { spawn: nodeSpawn } = require('child_process');
        const child = nodeSpawn(binary, args, {
          stdio: ['ignore', logFd, logFd],
          detached: true,
          env: { HOME: os.homedir(), PATH: process.env.PATH || '/usr/local/bin:/usr/bin:/bin', RUST_LOG: process.env.RUST_LOG || 'claw=info' },
        });
        child.unref();
        fs.closeSync(logFd);
        fs.writeFileSync(pidFile, String(child.pid));
        json(200, { message: 'Node restarted', pid: child.pid });
      } catch (e) { json(500, { error: e.message }); }
      return;
    }
    if (a === 'upgrade') {
      try {
        // 1. Stop running node
        const pidFile = OC_PID_FILE;
        try {
          const pid = parseInt(fs.readFileSync(pidFile, 'utf8').trim(), 10);
          if (pid > 0) try { process.kill(pid, 'SIGTERM'); } catch {}
        } catch {}
        // No pkill — only target our own PID

        // 2. Download latest binary
        const binDir = OC_BIN_DIR;
        const binName = process.platform === 'win32' ? 'claw-node.exe' : 'claw-node';
        const target = process.platform === 'darwin'
          ? (process.arch === 'arm64' ? 'macos-aarch64' : 'macos-x86_64')
          : process.platform === 'win32' ? 'windows-x86_64' : 'linux-x86_64';
        const ext = process.platform === 'win32' ? 'zip' : 'tar.gz';

        // Fetch latest release tag
        let version = 'latest';
        try {
          const res = await fetch('https://api.github.com/repos/clawlabz/claw-network/releases/latest');
          if (res.ok) { const d = await res.json(); if (d.tag_name) version = d.tag_name; }
        } catch {}

        const baseUrl = version === 'latest'
          ? 'https://github.com/clawlabz/claw-network/releases/latest/download'
          : 'https://github.com/clawlabz/claw-network/releases/download/' + version;
        const downloadUrl = baseUrl + '/claw-node-' + target + '.' + ext;

        const tmpFile = path.join(os.tmpdir(), 'claw-node-upgrade-' + Date.now() + '.' + ext);
        require('child_process').execFileSync('curl', ['-sSfL', '-o', tmpFile, downloadUrl], { timeout: 120000 });

        // Ensure bin directory exists
        if (!fs.existsSync(binDir)) { fs.mkdirSync(binDir, { recursive: true }); }

        // Extract binary
        if (ext === 'tar.gz') {
          require('child_process').execFileSync('tar', ['xzf', tmpFile, '-C', binDir], { timeout: 30000 });
        } else {
          // Windows zip handling
          const AdmZip = require('adm-zip');
          const zip = new AdmZip(tmpFile);
          zip.extractAllTo(binDir, true);
        }
        fs.chmodSync(path.join(binDir, binName), 0o755);
        try { fs.unlinkSync(tmpFile); } catch {}

        // 3. Get new version
        let newVersion = 'unknown';
        try {
          newVersion = require('child_process').execFileSync(path.join(binDir, binName), ['--version'], { encoding: 'utf8', timeout: 5000 }).trim();
        } catch {}

        json(200, { message: 'Upgraded to ' + newVersion + '. Click Restart to apply.', newVersion });
      } catch (e) { json(500, { error: e.message }); }
      return;
    }
    json(400, { error: 'Unknown action: ' + a });
    return;
  }
  json(404, { error: 'Not found' });
}

function tryListen(attempt) {
  if (attempt >= MAX_RETRIES) { console.error('Failed to bind UI server'); process.exit(1); }
  const port = PORT + attempt;
  const srv = http.createServer((req, res) => handle(req, res).catch(e => { try { res.writeHead(500); res.end(e.message); } catch {} }));
  srv.on('error', () => tryListen(attempt + 1));
  srv.listen(port, '127.0.0.1', () => {
    fs.mkdirSync(path.dirname(PORT_FILE), { recursive: true });
    fs.writeFileSync(PORT_FILE, JSON.stringify({ port, pid: process.pid, startedAt: new Date().toISOString() }));
    console.log('ClawNetwork Dashboard: http://127.0.0.1:' + port);
    process.on('SIGINT', () => { try { fs.unlinkSync(PORT_FILE); } catch {} process.exit(0); });
    process.on('SIGTERM', () => { try { fs.unlinkSync(PORT_FILE); } catch {} process.exit(0); });
  });
}
tryListen(0);
`

function startUiServer(cfg: PluginConfig, api: OpenClawApi): string | null {
  // Always kill old UI server and restart with fresh code.
  // The UI server is a detached process that survives gateway restarts,
  // so we must replace it to pick up plugin updates.
  stopUiServer()

  const scriptPath = path.join(WORKSPACE_DIR, 'ui-server.js')
  ensureDir(WORKSPACE_DIR)

  // Write HTML to a separate file, script reads it at startup
  const htmlPath = path.join(WORKSPACE_DIR, 'ui-dashboard.html')
  fs.writeFileSync(htmlPath, buildUiHtml(cfg))

  // Inject base dir, version, and HTML path into script (read from file, no template escaping issues)
  const fullScript = `const OPENCLAW_BASE_DIR = ${JSON.stringify(getBaseDir())};\nconst PLUGIN_VERSION = ${JSON.stringify(VERSION)};\nconst HTML_PATH = ${JSON.stringify(htmlPath)};\nconst HTML = require('fs').readFileSync(HTML_PATH, 'utf8');\n${UI_SERVER_SCRIPT}`
  fs.writeFileSync(scriptPath, fullScript)

  try {
    const child = fork(scriptPath, [String(cfg.uiPort), String(activeRpcPort ?? cfg.rpcPort), LOG_PATH], {
      detached: true,
      stdio: 'ignore',
    })
    child.unref()
    api.logger?.info?.(`[clawnetwork] dashboard starting on http://127.0.0.1:${cfg.uiPort}`)

    // Wait briefly for port file
    for (let i = 0; i < 10; i++) {
      const url = getDashboardUrl()
      if (url) return url
      // Busy-wait 200ms (can't use async sleep here)
      const start = Date.now()
      while (Date.now() - start < 200) { /* spin */ }
    }
    return `http://127.0.0.1:${cfg.uiPort}`
  } catch (e: unknown) {
    api.logger?.warn?.(`[clawnetwork] failed to start dashboard: ${(e as Error).message}`)
    return null
  }
}

function stopUiServer(): void {
  try {
    const raw = fs.readFileSync(UI_PORT_FILE, 'utf8')
    const info = JSON.parse(raw)
    if (info.pid) {
      try { process.kill(info.pid, 'SIGTERM') } catch { /* ok */ }
    }
  } catch { /* no file */ }
  try { fs.unlinkSync(UI_PORT_FILE) } catch { /* ok */ }
}

function getDashboardUrl(): string | null {
  try {
    const raw = fs.readFileSync(UI_PORT_FILE, 'utf8')
    const info = JSON.parse(raw)
    return `http://127.0.0.1:${info.port}`
  } catch { return null }
}

// ============================================================
// CLI Command Handlers
// ============================================================

const CLI_COMMANDS = [
  'clawnetwork:status', 'clawnetwork:start', 'clawnetwork:stop',
  'clawnetwork:wallet', 'clawnetwork:wallet:import', 'clawnetwork:wallet:export',
  'clawnetwork:faucet', 'clawnetwork:transfer', 'clawnetwork:stake',
  'clawnetwork:logs', 'clawnetwork:config', 'clawnetwork:ui',
  'clawnetwork:service:register', 'clawnetwork:service:search',
]

// ============================================================
// Main Plugin Registration
// ============================================================

export default function register(api: OpenClawApi) {
  const cfg = getConfig(api)

  // ── Gateway Methods ──

  api.registerGatewayMethod?.('clawnetwork.status', ctx => {
    checkHealth(cfg.rpcPort).then(health => {
      lastHealth = health
      ctx.respond?.(true, buildStatus(cfg) as unknown as Record<string, unknown>)
    }).catch(() => ctx.respond?.(true, buildStatus(cfg) as unknown as Record<string, unknown>))
  })

  api.registerGatewayMethod?.('clawnetwork.balance', ctx => {
    const address = (ctx.params?.address as string) || loadWallet()?.address || ''
    if (!address) { ctx.respond?.(false, { error: 'No wallet address' }); return }
    if (!isValidAddress(address)) { ctx.respond?.(false, { error: 'Invalid address format (expected 64-char hex)' }); return }
    rpcCall(cfg.rpcPort, 'claw_getBalance', [address])
      .then(result => ctx.respond?.(true, { address, balance: String(result), formatted: formatClaw(String(result as string)) }))
      .catch(err => ctx.respond?.(false, { error: (err as Error).message }))
  })

  api.registerGatewayMethod?.('clawnetwork.transfer', ctx => {
    const to = ctx.params?.to as string
    const amount = ctx.params?.amount as string
    if (!to || !amount) { ctx.respond?.(false, { error: 'Missing params: to, amount' }); return }
    if (!isValidAddress(to)) { ctx.respond?.(false, { error: 'Invalid address (expected 64-char hex)' }); return }
    if (!isValidAmount(amount)) { ctx.respond?.(false, { error: 'Invalid amount (must be positive number)' }); return }
    const binary = findBinary()
    if (!binary) { ctx.respond?.(false, { error: 'claw-node binary not found' }); return }
    try {
      const output = execFileSync(binary, [
        'transfer', to, amount, '--rpc', `http://localhost:${activeRpcPort ?? cfg.rpcPort}`, '--data-dir', DATA_DIR,
      ], {
        encoding: 'utf8',
        timeout: 30_000,
        env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
      })
      const hashMatch = output.match(/[0-9a-f]{64}/i)
      ctx.respond?.(true, { txHash: hashMatch?.[0] ?? '', to, amount, raw: output.trim() })
    } catch (e: unknown) {
      ctx.respond?.(false, { error: (e as Error).message })
    }
  })

  api.registerGatewayMethod?.('clawnetwork.agent-register', ctx => {
    const rawName = (ctx.params?.name as string) || `openclaw-agent-${Date.now().toString(36)}`
    const name = sanitizeAgentName(rawName)
    if (!name || name.length < 2) { ctx.respond?.(false, { error: 'Invalid agent name' }); return }
    const binary = findBinary()
    if (!binary) { ctx.respond?.(false, { error: 'claw-node binary not found' }); return }
    try {
      const output = execFileSync(binary, [
        'register-agent', '--name', name,
        '--rpc', `http://localhost:${activeRpcPort ?? cfg.rpcPort}`, '--data-dir', DATA_DIR,
      ], {
        encoding: 'utf8',
        timeout: 30_000,
        env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
      })
      const hashMatch = output.match(/[0-9a-f]{64}/i)
      ctx.respond?.(true, { txHash: hashMatch?.[0] ?? '', name, raw: output.trim() })
    } catch (e: unknown) {
      ctx.respond?.(false, { error: (e as Error).message })
    }
  })

  api.registerGatewayMethod?.('clawnetwork.faucet', ctx => {
    const wallet = loadWallet()
    if (!wallet?.address) { ctx.respond?.(false, { error: 'No wallet' }); return }
    rpcCall(cfg.rpcPort, 'claw_faucet', [wallet.address])
      .then(result => ctx.respond?.(true, result as Record<string, unknown>))
      .catch(err => ctx.respond?.(false, { error: (err as Error).message }))
  })

  api.registerGatewayMethod?.('clawnetwork.start', ctx => {
    const binary = findBinary()
    if (!binary) { ctx.respond?.(false, { error: 'claw-node binary not found. Set autoDownload=true or install manually.' }); return }
    if (nodeProcess && !nodeProcess.killed) { ctx.respond?.(true, { message: 'Node already running', ...buildStatus(cfg) }); return }
    initNode(binary, cfg.network, api)
    startNodeProcess(binary, cfg, api)
    ctx.respond?.(true, { message: 'Node starting...', ...buildStatus(cfg) })
  })

  api.registerGatewayMethod?.('clawnetwork.stop', ctx => {
    stopNode(api)
    ctx.respond?.(true, { message: 'Node stopped' })
  })

  api.registerGatewayMethod?.('clawnetwork.service-register', ctx => {
    const serviceType = ctx.params?.serviceType as string
    const endpoint = ctx.params?.endpoint as string
    const description = (ctx.params?.description as string) || ''
    const priceAmount = (ctx.params?.priceAmount as string) || '0'
    if (!serviceType || !endpoint) { ctx.respond?.(false, { error: 'Missing params: serviceType, endpoint' }); return }
    const binary = findBinary()
    if (!binary) { ctx.respond?.(false, { error: 'claw-node binary not found' }); return }
    try {
      const output = execFileSync(binary, [
        'register-service',
        '--service-type', serviceType,
        '--endpoint', endpoint,
        '--description', description,
        '--price', priceAmount,
        '--rpc', `http://localhost:${activeRpcPort ?? cfg.rpcPort}`, '--data-dir', DATA_DIR,
      ], {
        encoding: 'utf8',
        timeout: 30_000,
        env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
      })
      ctx.respond?.(true, { ok: true, raw: output.trim() })
    } catch (e: unknown) {
      ctx.respond?.(false, { error: (e as Error).message })
    }
  })

  api.registerGatewayMethod?.('clawnetwork.service-search', ctx => {
    const serviceType = ctx.params?.serviceType as string | undefined
    const params = serviceType ? [serviceType] : []
    rpcCall(cfg.rpcPort, 'claw_getServices', params)
      .then(result => ctx.respond?.(true, { services: result }))
      .catch(err => ctx.respond?.(false, { error: (err as Error).message }))
  })

  // ── CLI Commands ──

  api.registerCli?.(({ program }) => {
    function cmd(parent: CliCommandChain | CliProgram, name: string) {
      return parent.command(name).allowExcessArguments(true)
    }

    function out(data: unknown) {
      process.stdout.write(JSON.stringify(data, null, 2) + '\n')
    }

    const handleStatus = async () => {
      const health = await checkHealth(cfg.rpcPort)
      lastHealth = health
      const status = buildStatus(cfg)
      // Enrich with balance
      let balance = ''
      const wallet = loadWallet()
      if (wallet?.address && status.running) {
        try {
          const raw = await rpcCall(cfg.rpcPort, 'claw_getBalance', [wallet.address])
          balance = formatClaw(String(raw as string))
        } catch { /* ok */ }
      }
      const dashboard = getDashboardUrl()
      out({ ...status, balance: balance || undefined, dashboard: dashboard || undefined })
    }

    const handleStart = async () => {
      // Check if already running (in-memory or detached via PID file)
      const state = isNodeRunning()
      if (state.running) {
        out({ message: 'Node already running', pid: state.pid })
        return
      }
      let binary = findBinary()
      if (!binary) {
        if (cfg.autoDownload) {
          process.stdout.write('Downloading claw-node...\n')
          binary = await downloadBinary(api)
        } else {
          out({ error: 'claw-node not found. Run: curl -sSf https://raw.githubusercontent.com/clawlabz/claw-network/main/claw-node/scripts/install.sh | bash' })
          return
        }
      } else {
        // Auto-upgrade if binary is older than required minimum
        const currentVersion = getBinaryVersion(binary)
        if (currentVersion && isVersionOlder(currentVersion, MIN_NODE_VERSION)) {
          api.logger?.info?.(`[clawnetwork] claw-node ${currentVersion} is outdated (need >=${MIN_NODE_VERSION}), upgrading...`)
          process.stdout.write(`Upgrading claw-node ${currentVersion} → ${MIN_NODE_VERSION}+...\n`)
          try {
            binary = await downloadBinary(api)
          } catch (e: unknown) {
            api.logger?.warn?.(`[clawnetwork] auto-upgrade failed: ${(e as Error).message}, continuing with ${currentVersion}`)
          }
        }
      }
      initNode(binary, cfg.network, api)
      startNodeProcess(binary, cfg, api)
      // Start UI dashboard
      const dashUrl = startUiServer(cfg, api)
      out({ message: 'Node started', pid: nodeProcess?.pid, network: cfg.network, rpc: `http://localhost:${cfg.rpcPort}`, dashboard: dashUrl || `http://127.0.0.1:${cfg.uiPort}` })
    }

    const handleStop = () => {
      stopMinerHeartbeatLoop()
      stopNode(api)
      stopUiServer()
      out({ message: 'Node stopped' })
    }

    const handleWallet = async () => {
      const wallet = ensureWallet(cfg.network, api)
      let balance = 'unknown (node offline)'
      try {
        const raw = await rpcCall(cfg.rpcPort, 'claw_getBalance', [wallet.address])
        balance = formatClaw(String(raw as string))
      } catch { /* node might be down */ }
      out({ address: wallet.address, balance, network: wallet.network, createdAt: wallet.createdAt })
    }

    const handleWalletImport = (privateKeyHex?: string) => {
      if (typeof privateKeyHex !== 'string' || !privateKeyHex) {
        process.stdout.write('Usage: openclaw clawnetwork wallet import <private-key-hex>\n')
        return
      }
      if (!isValidPrivateKey(privateKeyHex)) {
        out({ error: 'Invalid private key: must be 64 hex characters' })
        return
      }
      const wallet: WalletData = {
        address: '',
        secretKey: privateKeyHex,
        createdAt: new Date().toISOString(),
        network: cfg.network,
      }
      const binary = findBinary()
      if (binary) {
        try {
          execFileSync(binary, ['key', 'import', privateKeyHex, '--data-dir', DATA_DIR], {
            encoding: 'utf8',
            timeout: 10_000,
            env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
          })
          const showOut = execFileSync(binary, ['key', 'show', '--data-dir', DATA_DIR], {
            encoding: 'utf8',
            timeout: 5000,
            env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
          })
          const match = showOut.match(/[0-9a-f]{64}/i)
          if (match) wallet.address = match[0]
        } catch { /* ok */ }
      }
      saveWallet(wallet)
      out({ message: 'Wallet imported', address: wallet.address || '(address resolved on node start)' })
    }

    const handleWalletExport = () => {
      const wallet = loadWallet()
      if (!wallet) { out({ error: 'No wallet found. Run: openclaw clawnetwork wallet' }); return }
      out({ address: wallet.address, secretKey: wallet.secretKey, _warning: 'NEVER share your secret key with anyone' })
    }

    const handleFaucet = async () => {
      const wallet = ensureWallet(cfg.network, api)
      if (!wallet.address) { out({ error: 'No wallet address' }); return }
      try {
        const result = await rpcCall(cfg.rpcPort, 'claw_faucet', [wallet.address])
        out(result)
      } catch (e: unknown) {
        out({ error: (e as Error).message, hint: 'Faucet is only available on testnet/devnet' })
      }
    }

    const handleTransfer = async (to?: string, amount?: string) => {
      if (typeof to !== 'string' || typeof amount !== 'string') {
        process.stdout.write('Usage: openclaw clawnetwork transfer <to-address> <amount>\n')
        return
      }
      if (!isValidAddress(to)) { out({ error: 'Invalid address: must be 64 hex characters' }); return }
      if (!isValidAmount(amount)) { out({ error: 'Invalid amount: must be a positive number' }); return }
      const binary = findBinary()
      if (!binary) { out({ error: 'claw-node binary not found' }); return }
      try {
        const output = execFileSync(binary, [
          'transfer', to, amount, '--rpc', `http://localhost:${activeRpcPort ?? cfg.rpcPort}`, '--data-dir', DATA_DIR,
        ], {
          encoding: 'utf8',
          timeout: 30_000,
          env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
        })
        out({ ok: true, to, amount, raw: output.trim() })
      } catch (e: unknown) {
        out({ error: (e as Error).message })
      }
    }

    const handleStake = async (amount?: string) => {
      if (typeof amount !== 'string') {
        process.stdout.write('Usage: openclaw clawnetwork stake <amount>\n')
        return
      }
      if (!isValidAmount(amount)) { out({ error: 'Invalid amount' }); return }
      const binary = findBinary()
      if (!binary) { out({ error: 'claw-node binary not found' }); return }
      try {
        const output = execFileSync(binary, [
          'stake', amount, '--rpc', `http://localhost:${activeRpcPort ?? cfg.rpcPort}`, '--data-dir', DATA_DIR,
        ], {
          encoding: 'utf8',
          timeout: 30_000,
          env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
        })
        out({ ok: true, amount, raw: output.trim() })
      } catch (e: unknown) {
        out({ error: (e as Error).message })
      }
    }

    const handleServiceRegister = async (serviceType?: string, endpoint?: string) => {
      if (typeof serviceType !== 'string' || typeof endpoint !== 'string') {
        process.stdout.write('Usage: openclaw clawnetwork service register <type> <endpoint>\n')
        return
      }
      const binary = findBinary()
      if (!binary) { out({ error: 'claw-node binary not found' }); return }
      try {
        const output = execFileSync(binary, [
          'register-service', '--service-type', serviceType, '--endpoint', endpoint,
          '--rpc', `http://localhost:${activeRpcPort ?? cfg.rpcPort}`, '--data-dir', DATA_DIR,
        ], {
          encoding: 'utf8',
          timeout: 30_000,
          env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
        })
        out({ ok: true, serviceType, endpoint, raw: output.trim() })
      } catch (e: unknown) {
        out({ error: (e as Error).message })
      }
    }

    const handleServiceSearch = async (serviceType?: string) => {
      try {
        const params = typeof serviceType === 'string' ? [serviceType] : []
        const result = await rpcCall(cfg.rpcPort, 'claw_getServices', params)
        out({ services: result })
      } catch (e: unknown) {
        out({ error: (e as Error).message })
      }
    }

    const handleLogs = () => {
      if (!fs.existsSync(LOG_PATH)) { process.stdout.write('No logs yet\n'); return }
      const content = fs.readFileSync(LOG_PATH, 'utf8')
      const lines = content.split('\n')
      process.stdout.write(lines.slice(-80).join('\n') + '\n')
    }

    const handleConfig = () => { out(cfg) }

    const handleUi = async () => {
      const existing = getDashboardUrl()
      if (existing) {
        process.stdout.write(`Dashboard already running: ${existing}\n`)
        const open = process.platform === 'darwin' ? 'open' : process.platform === 'win32' ? 'start' : 'xdg-open'
        try { execFileSync(open, [existing], { timeout: 5000 }) } catch { /* ok */ }
        return
      }
      const url = startUiServer(cfg, api)
      if (url) {
        process.stdout.write(`Dashboard: ${url}\n`)
        const open = process.platform === 'darwin' ? 'open' : process.platform === 'win32' ? 'start' : 'xdg-open'
        try { execFileSync(open, [url], { timeout: 5000 }) } catch { /* ok */ }
      } else {
        out({ error: 'Failed to start dashboard' })
      }
    }

    // Space format: `openclaw clawnetwork <sub>`
    const group = cmd(program, 'clawnetwork').description('ClawNetwork blockchain node management')
    cmd(group, 'status').description('Show node status').action(handleStatus)
    cmd(group, 'start').description('Start the blockchain node').action(handleStart)
    cmd(group, 'stop').description('Stop the blockchain node').action(handleStop)
    cmd(group, 'faucet').description('Get testnet CLAW from faucet').action(handleFaucet)
    cmd(group, 'transfer').description('Transfer CLAW').argument('<to>', 'Recipient address').argument('<amount>', 'Amount in CLAW').action(handleTransfer)
    cmd(group, 'stake').description('Stake CLAW').argument('<amount>', 'Amount to stake').action(handleStake)
    cmd(group, 'logs').description('Show recent node logs').action(handleLogs)
    cmd(group, 'config').description('Show current config').action(handleConfig)
    cmd(group, 'ui').description('Open visual dashboard').action(handleUi)

    // Wallet subcommands
    const walletGroup = cmd(group, 'wallet').description('Wallet management')
    cmd(walletGroup, 'show').description('Show wallet address and balance').action(handleWallet)
    cmd(walletGroup, 'import').description('Import wallet from private key').argument('<key>', 'Private key hex').action(handleWalletImport)
    cmd(walletGroup, 'export').description('Export wallet private key').action(handleWalletExport)

    // Service subcommands
    const serviceGroup = cmd(group, 'service').description('Service discovery')
    cmd(serviceGroup, 'register').description('Register a service').argument('<type>', 'Service type').argument('<endpoint>', 'Endpoint URL').action(handleServiceRegister)
    cmd(serviceGroup, 'search').description('Search services').argument('[type]', 'Filter by type').action(handleServiceSearch)

    // Colon format: `openclaw clawnetwork:status`
    for (const prefix of ['clawnetwork']) {
      cmd(program, `${prefix}:status`).description('Show node status').action(handleStatus)
      cmd(program, `${prefix}:start`).description('Start the blockchain node').action(handleStart)
      cmd(program, `${prefix}:stop`).description('Stop the blockchain node').action(handleStop)
      cmd(program, `${prefix}:wallet`).description('Show wallet').action(handleWallet)
      cmd(program, `${prefix}:wallet:import`).description('Import wallet').argument('<key>', 'Private key hex').action(handleWalletImport)
      cmd(program, `${prefix}:wallet:export`).description('Export wallet').action(handleWalletExport)
      cmd(program, `${prefix}:faucet`).description('Get testnet CLAW').action(handleFaucet)
      cmd(program, `${prefix}:transfer`).description('Transfer CLAW').argument('<to>', 'Recipient').argument('<amount>', 'Amount').action(handleTransfer)
      cmd(program, `${prefix}:stake`).description('Stake CLAW').argument('<amount>', 'Amount').action(handleStake)
      cmd(program, `${prefix}:logs`).description('Show recent node logs').action(handleLogs)
      cmd(program, `${prefix}:config`).description('Show current config').action(handleConfig)
      cmd(program, `${prefix}:ui`).description('Open dashboard').action(handleUi)
      cmd(program, `${prefix}:service:register`).description('Register service').argument('<type>', 'Type').argument('<endpoint>', 'URL').action(handleServiceRegister)
      cmd(program, `${prefix}:service:search`).description('Search services').argument('[type]', 'Type filter').action(handleServiceSearch)
    }

  }, { commands: CLI_COMMANDS })

  // ── Service Lifecycle ──

  api.registerService?.({
    id: 'clawnetwork-node',
    start: () => {
      api.logger?.info?.(`[clawnetwork] plugin v${VERSION} loaded, network=${cfg.network}, autoStart=${cfg.autoStart}`)

      if (!cfg.autoStart) return

      ;(async () => {
        try {
          // Check if already running (e.g. from a previous detached start)
          const state = isNodeRunning()
          if (state.running) {
            api.logger?.info?.(`[clawnetwork] node already running (pid=${state.pid})`)

            // Check if local binary is newer than the running process — restart if so.
            // Also check GitHub for even newer versions and download if available.
            try {
              const binary = findBinary()
              const localBinaryVersion = binary ? getBinaryVersion(binary) : null

              // Read actual runtime port from last run (may differ from config if port was auto-shifted)
              let runtimeRpcPort = cfg.rpcPort
              try {
                const rt = JSON.parse(fs.readFileSync(path.join(WORKSPACE_DIR, 'runtime.json'), 'utf8'))
                if (rt.rpcPort) { runtimeRpcPort = rt.rpcPort; activeRpcPort = rt.rpcPort }
                if (rt.p2pPort) { activeP2pPort = rt.p2pPort }
              } catch { /* no runtime file, use config */ }

              // Get the RUNNING process version from health endpoint
              let runningProcessVersion: string | null = null
              try {
                const health = await fetch(`http://localhost:${runtimeRpcPort}/health`)
                if (health.ok) {
                  const hd = await health.json() as Record<string, unknown>
                  if (typeof hd.version === 'string') runningProcessVersion = hd.version.replace(/^v/, '')
                }
              } catch { /* health endpoint not ready */ }

              // Step A: If GitHub has a newer binary than what's on disk, download it first
              if (cfg.autoDownload && localBinaryVersion) {
                try {
                  const res = await fetch(`https://api.github.com/repos/${GITHUB_REPO}/releases/latest`)
                  if (res.ok) {
                    const data = await res.json() as Record<string, unknown>
                    const latestVersion = typeof data.tag_name === 'string' ? data.tag_name.replace(/^v/, '') : null
                    if (latestVersion && isVersionOlder(localBinaryVersion, latestVersion)) {
                      api.logger?.info?.(`[clawnetwork] downloading newer binary: ${localBinaryVersion} → ${latestVersion}`)
                      try { await downloadBinary(api) } catch (e: unknown) {
                        api.logger?.warn?.(`[clawnetwork] download failed: ${(e as Error).message}`)
                      }
                    }
                  }
                } catch { /* network error, skip */ }
              }

              // Step B: If the local binary is newer than the running process, restart with it
              const finalBinary = findBinary()
              const finalBinaryVersion = finalBinary ? getBinaryVersion(finalBinary) : null
              if (finalBinaryVersion && runningProcessVersion && isVersionOlder(runningProcessVersion, finalBinaryVersion)) {
                api.logger?.info?.(`[clawnetwork] running node ${runningProcessVersion} is outdated, restarting with ${finalBinaryVersion}...`)
                stopNode(api)
                await sleep(3_000)
                initNode(finalBinary, cfg.network, api)
                startNodeProcess(finalBinary, cfg, api)
                api.logger?.info?.(`[clawnetwork] node upgraded to ${finalBinaryVersion}`)
              }
            } catch (e: unknown) {
              api.logger?.warn?.(`[clawnetwork] upgrade check failed: ${(e as Error).message}`)
            }

            startHealthCheck(cfg, api)
            startUiServer(cfg, api)
            const wallet = ensureWallet(cfg.network, api)
            await sleep(5_000)
            await autoRegisterMiner(cfg, wallet, api)
            return
          }

          // Step 1: Ensure binary (auto-upgrade if outdated)
          let binary = findBinary()
          if (!binary) {
            if (cfg.autoDownload) {
              api.logger?.info?.('[clawnetwork] claw-node not found, downloading...')
              binary = await downloadBinary(api)
            } else {
              api.logger?.error?.('[clawnetwork] claw-node not found and autoDownload is disabled')
              return
            }
          } else if (cfg.autoDownload) {
            const cv = getBinaryVersion(binary)
            if (cv && isVersionOlder(cv, MIN_NODE_VERSION)) {
              api.logger?.info?.(`[clawnetwork] claw-node ${cv} outdated (need >=${MIN_NODE_VERSION}), upgrading...`)
              try { binary = await downloadBinary(api) } catch (e: unknown) {
                api.logger?.warn?.(`[clawnetwork] auto-upgrade failed: ${(e as Error).message}`)
              }
            }
          }

          // Step 2: Init
          initNode(binary, cfg.network, api)

          // Step 3: Wallet
          const wallet = ensureWallet(cfg.network, api)

          // Step 4: Save config for UI server to read
          const cfgPath = path.join(WORKSPACE_DIR, 'config.json')
          fs.writeFileSync(cfgPath, JSON.stringify({ network: cfg.network, rpcPort: cfg.rpcPort, p2pPort: cfg.p2pPort, syncMode: cfg.syncMode, extraBootstrapPeers: cfg.extraBootstrapPeers }))

          // Step 5: Start node
          startNodeProcess(binary, cfg, api)

          // Step 6: Start UI dashboard
          startUiServer(cfg, api)

          // Step 7: Wait for node to sync, then auto-register
          await sleep(15_000)
          await autoRegisterAgent(cfg, wallet, api)

          // Step 8: Auto-register as miner + start heartbeat loop
          await autoRegisterMiner(cfg, wallet, api)

        } catch (err: unknown) {
          api.logger?.error?.(`[clawnetwork] startup failed: ${(err as Error).message}`)
        }
      })()
    },
    stop: () => {
      api.logger?.info?.('[clawnetwork] shutting down...')
      stopMinerHeartbeatLoop()
      stopNode(api)
      stopUiServer()
    },
  })
}
