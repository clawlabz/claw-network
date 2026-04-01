/* eslint-disable @typescript-eslint/no-require-imports */
declare const process: { stdout: { write: (s: string) => void }; env: Record<string, string | undefined>; platform: string; arch: string; kill: (pid: number, sig: string) => boolean; pid: number; on: (event: string, handler: (...args: unknown[]) => void) => void }
declare function require(id: string): any
declare function setTimeout(fn: () => void, ms: number): unknown
declare function clearTimeout(id: unknown): void
declare function setInterval(fn: () => void, ms: number): unknown
declare function clearInterval(id: unknown): void
declare function fetch(url: string, init?: Record<string, unknown>): Promise<{ status: number; ok: boolean; text: () => Promise<string>; json: () => Promise<unknown> }>

const VERSION = '0.1.0'
const PLUGIN_ID = 'clawnetwork'
const GITHUB_REPO = 'clawlabz/claw-network'
const DEFAULT_RPC_PORT = 9710
const DEFAULT_P2P_PORT = 9711
const DEFAULT_NETWORK = 'mainnet'
const DEFAULT_SYNC_MODE = 'light'
const DEFAULT_HEALTH_CHECK_SECONDS = 30
const DEFAULT_UI_PORT = 19877
const MAX_RESTART_ATTEMPTS = 3

// Built-in bootstrap peers for each network
const BOOTSTRAP_PEERS: Record<string, string[]> = {
  mainnet: [
    '/ip4/178.156.162.162/tcp/9711',
  ],
  testnet: [
    '/ip4/178.156.162.162/tcp/9721',
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

function homePath(...segments: string[]): string {
  return path.join(os.homedir(), ...segments)
}

const WORKSPACE_DIR = homePath('.openclaw', 'workspace', 'clawnetwork')
const BIN_DIR = homePath('.openclaw', 'bin')
const DATA_DIR = homePath('.clawnetwork')
const WALLET_PATH = path.join(WORKSPACE_DIR, 'wallet.json')
const LOG_PATH = path.join(WORKSPACE_DIR, 'node.log')
const UI_PORT_FILE = homePath('.openclaw', 'clawnetwork-ui-port')

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
    // If checksum verification fails but file was downloaded, warn but continue
    const msg = (e as Error).message
    if (msg.includes('SHA256 mismatch')) throw e
    api.logger?.warn?.(`[clawnetwork] checksum verification skipped: ${msg}`)
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
    const output = execFileSync(binaryPath, ['init', '--network', network], {
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
      execFileSync(binary, ['key', 'import', secretKeyHex], {
        encoding: 'utf8',
        timeout: 10_000,
        env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
      })
      const showOut = execFileSync(binary, ['key', 'show'], {
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

function isNodeRunning(): { running: boolean; pid: number | null } {
  // Check in-memory process first
  if (nodeProcess && !nodeProcess.killed) return { running: true, pid: nodeProcess.pid }
  // Check PID file (for detached processes from previous CLI invocations)
  const pidFile = path.join(WORKSPACE_DIR, 'node.pid')
  try {
    const pid = parseInt(fs.readFileSync(pidFile, 'utf8').trim(), 10)
    if (pid > 0) {
      try { execFileSync('kill', ['-0', String(pid)], { timeout: 2000 }); return { running: true, pid } } catch { /* dead */ }
    }
  } catch { /* no file */ }
  // Last resort: check if RPC port is responding (covers orphaned processes)
  try {
    execFileSync('curl', ['-sf', '--max-time', '1', 'http://localhost:9710/health'], { timeout: 3000, encoding: 'utf8' })
    // Port is responding — find PID by port
    try {
      const lsof = execFileSync('lsof', ['-ti', ':9710'], { timeout: 3000, encoding: 'utf8' }).trim()
      const pid = parseInt(lsof.split('\n')[0], 10)
      if (pid > 0) return { running: true, pid }
    } catch { /* ok */ }
    return { running: true, pid: null }
  } catch { /* not responding */ }
  return { running: false, pid: null }
}

function buildStatus(cfg: PluginConfig): NodeStatus {
  const wallet = loadWallet()
  const nodeState = isNodeRunning()
  const uptime = nodeStartedAt ? Math.floor((Date.now() - nodeStartedAt) / 1000) : null
  return {
    running: nodeState.running,
    pid: nodeState.pid,
    blockHeight: lastHealth.blockHeight,
    peerCount: lastHealth.peerCount,
    network: cfg.network,
    syncMode: cfg.syncMode,
    rpcUrl: `http://localhost:${cfg.rpcPort}`,
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
  // Guard: check in-memory reference, PID file, AND health endpoint
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

  const args = ['start', '--network', cfg.network, '--rpc-port', String(cfg.rpcPort), '--p2p-port', String(cfg.p2pPort), '--sync-mode', cfg.syncMode, '--allow-genesis']

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
    lastHealth = await checkHealth(cfg.rpcPort)
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

  // Find PID: in-memory process or PID file (for detached processes)
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
    try { process.kill(pid, 'SIGTERM') } catch { /* already dead */ }
    setTimeout(() => {
      try { process.kill(pid as number, 'SIGKILL') } catch { /* ok */ }
    }, 10_000)
  }

  // Write stop signal file (tells restart loop in other CLI processes to stop)
  const stopFile = path.join(WORKSPACE_DIR, 'stop.signal')
  try { fs.writeFileSync(stopFile, String(Date.now())) } catch { /* ok */ }

  // Also kill any claw-node processes by name (covers orphans from restart loops)
  try { execFileSync('pkill', ['-f', 'claw-node start'], { timeout: 3000 }) } catch { /* ok */ }

  nodeProcess = null
  nodeStartedAt = null
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
      'agent', 'register', '--name', agentName,
      '--rpc', `http://localhost:${cfg.rpcPort}`,
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

// Heartbeat interval: MINER_GRACE_BLOCKS is 2000, at 3s/block = ~6000s.
// Send heartbeat every ~1000 blocks (~50 min) to stay well within grace period.
const MINER_HEARTBEAT_INTERVAL_MS = 50 * 60 * 1000 // 50 minutes
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
      '--rpc', `http://localhost:${cfg.rpcPort}`,
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

  try {
    const output = execFileSync(binary, [
      'miner-heartbeat',
      '--rpc', `http://localhost:${cfg.rpcPort}`,
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
  <link rel="icon" href="https://cdn.clawlabz.xyz/brand/favicon.png">
  <style>
    :root {
      --bg: #0a0a12;
      --bg-panel: #12121f;
      --border: #1e1e3a;
      --accent: #00ccff;
      --accent-dim: rgba(0, 204, 255, 0.15);
      --green: #00ff88;
      --green-dim: rgba(0, 255, 136, 0.15);
      --purple: #8b5cf6;
      --text: #e0e0f0;
      --text-dim: #666688;
      --danger: #ff4455;
      --font: system-ui, -apple-system, sans-serif;
      --font-mono: 'SF Mono', 'Fira Code', Consolas, monospace;
      --radius: 10px;
      --shadow: 0 4px 24px rgba(0, 0, 0, 0.4);
    }
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    body { background: var(--bg); color: var(--text); font-family: var(--font); line-height: 1.6; min-height: 100vh; }
    .container { max-width: 960px; margin: 0 auto; padding: 0 20px; }
    @keyframes pulse { 0%,100%{opacity:1} 50%{opacity:0.4} }

    .header { background: var(--bg-panel); border-bottom: 1px solid var(--border); padding: 16px 0; position: sticky; top: 0; z-index: 100; }
    .header .container { display: flex; align-items: center; justify-content: space-between; }
    .logo { font-size: 22px; font-weight: 800; letter-spacing: -0.5px; }
    .logo-claw { color: var(--accent); }
    .logo-net { color: var(--green); }
    .header-badge { font-size: 11px; background: var(--accent-dim); color: var(--accent); padding: 2px 8px; border-radius: 4px; }

    .stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 16px; margin: 24px 0; }
    .stat-card { background: var(--bg-panel); border: 1px solid var(--border); border-radius: var(--radius); padding: 20px; }
    .stat-label { font-size: 12px; color: var(--text-dim); text-transform: uppercase; letter-spacing: 1px; }
    .stat-value { font-size: 28px; font-weight: 700; font-family: var(--font-mono); margin-top: 4px; }
    .stat-value.green { color: var(--green); }
    .stat-value.accent { color: var(--accent); }
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

    .btn { display: inline-flex; align-items: center; gap: 6px; padding: 8px 16px; border-radius: 6px; border: 1px solid var(--border); background: var(--bg-panel); color: var(--text); font-size: 13px; cursor: pointer; transition: 0.2s; }
    .btn:hover { border-color: var(--accent); color: var(--accent); }
    .btn.danger:hover { border-color: var(--danger); color: var(--danger); }
    .btn.primary { background: var(--accent-dim); border-color: var(--accent); color: var(--accent); }
    .btn-group { display: flex; gap: 8px; margin: 16px 0; flex-wrap: wrap; }

    .logs-box { background: #080810; border: 1px solid var(--border); border-radius: var(--radius); padding: 16px; font-family: var(--font-mono); font-size: 12px; max-height: 300px; overflow-y: auto; white-space: pre-wrap; color: var(--text-dim); line-height: 1.8; }

    .wallet-addr { font-family: var(--font-mono); font-size: 13px; background: var(--bg); padding: 8px 12px; border-radius: 6px; border: 1px solid var(--border); word-break: break-all; display: flex; align-items: center; gap: 8px; }
    .copy-btn { background: none; border: none; color: var(--accent); cursor: pointer; font-size: 14px; padding: 2px 6px; }
    .copy-btn:hover { opacity: 0.7; }

    .toast { position: fixed; bottom: 24px; right: 24px; background: var(--bg-panel); border: 1px solid var(--accent); color: var(--accent); padding: 12px 20px; border-radius: 8px; font-size: 13px; opacity: 0; transition: 0.3s; z-index: 1000; }
    .toast.show { opacity: 1; }
  </style>
</head>
<body>
  <header class="header">
    <div class="container">
      <div style="display:flex;align-items:center;gap:14px">
        <div class="logo"><span class="logo-claw">Claw</span><span class="logo-net">Network</span></div>
        <span class="header-badge">Node Dashboard</span>
      </div>
      <span id="lastUpdate" style="font-size:12px;color:var(--text-dim)"></span>
    </div>
  </header>

  <main class="container" style="padding-top:8px;padding-bottom:40px">
    <div class="stats-grid">
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

    <div class="btn-group">
      <button class="btn primary" onclick="doAction('start')">Start Node</button>
      <button class="btn danger" onclick="doAction('stop')">Stop Node</button>
      <button class="btn" onclick="doAction('faucet')">Faucet (testnet)</button>
      <button class="btn" onclick="refreshLogs()">Refresh Logs</button>
    </div>

    <div class="panel">
      <div class="panel-title">Wallet</div>
      <div id="walletInfo">Loading...</div>
    </div>

    <div class="panel">
      <div class="panel-title">Node Info</div>
      <div id="nodeInfo">Loading...</div>
    </div>

    <div class="panel">
      <div class="panel-title">Recent Logs</div>
      <div class="logs-box" id="logsBox">Loading...</div>
    </div>
  </main>

  <div class="toast" id="toast"></div>

  <script>
    const API = '';
    let autoRefresh = null;

    function toast(msg) {
      const el = document.getElementById('toast');
      el.textContent = msg;
      el.classList.add('show');
      setTimeout(() => el.classList.remove('show'), 3000);
    }

    function copyText(text) {
      navigator.clipboard.writeText(text).then(() => toast('Copied!')).catch(() => {});
    }

    async function fetchStatus() {
      try {
        const res = await fetch(API + '/api/status');
        const data = await res.json();
        renderStatus(data);
        document.getElementById('lastUpdate').textContent = 'Updated: ' + new Date().toLocaleTimeString();
      } catch (e) { console.error(e); }
    }

    function renderStatus(s) {
      const statusEl = document.getElementById('statusValue');
      if (s.running) {
        const dotClass = s.syncing ? 'syncing' : 'online';
        const label = s.syncing ? 'Syncing' : 'Online';
        statusEl.innerHTML = '<span class="status-dot ' + dotClass + '"></span>' + label;
        statusEl.className = 'stat-value green';
      } else {
        statusEl.innerHTML = '<span class="status-dot offline"></span>Offline';
        statusEl.className = 'stat-value danger';
      }

      document.getElementById('heightValue').textContent = s.blockHeight !== null ? s.blockHeight.toLocaleString() : '—';
      document.getElementById('peersValue').textContent = s.peerCount !== null ? s.peerCount : '—';
      document.getElementById('uptimeValue').textContent = s.uptimeFormatted || '—';

      // Wallet
      const wHtml = s.walletAddress
        ? '<div class="wallet-addr">' + s.walletAddress + ' <button class="copy-btn" onclick="copyText(\\''+s.walletAddress+'\\')">Copy</button></div>' +
          (s.balance ? '<div style="margin-top:8px;font-size:14px;color:var(--green)">' + s.balance + '</div>' : '')
        : '<div style="color:var(--text-dim)">No wallet yet — start the node to generate one</div>';
      document.getElementById('walletInfo').innerHTML = wHtml;

      // Node info
      const rows = [
        ['Network', s.network],
        ['Sync Mode', s.syncMode],
        ['RPC URL', s.rpcUrl],
        ['Binary Version', s.binaryVersion || '—'],
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

const PORT = parseInt(process.argv[2] || '19877', 10);
const RPC_PORT = parseInt(process.argv[3] || '9710', 10);
const LOG_PATH = process.argv[4] || path.join(os.homedir(), '.openclaw/workspace/clawnetwork/node.log');
const PORT_FILE = path.join(os.homedir(), '.openclaw/clawnetwork-ui-port');
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
      try {
        const walletPath = path.join(os.homedir(), '.openclaw/workspace/clawnetwork/wallet.json');
        const w = JSON.parse(fs.readFileSync(walletPath, 'utf8'));
        if (w.address) { const b = await rpcCall('claw_getBalance', [w.address]); balance = formatClaw(b); }
      } catch {}
      json(200, {
        running: h.status === 'ok',
        blockHeight: h.height,
        peerCount: h.peer_count,
        network: h.chain_id,
        syncMode: 'light',
        rpcUrl: 'http://localhost:' + RPC_PORT,
        walletAddress: (() => { try { return JSON.parse(fs.readFileSync(path.join(os.homedir(), '.openclaw/workspace/clawnetwork/wallet.json'), 'utf8')).address; } catch { return ''; } })(),
        binaryVersion: h.version,
        pluginVersion: '0.1.0',
        uptime: h.uptime_secs,
        uptimeFormatted: h.uptime_secs < 60 ? h.uptime_secs + 's' : h.uptime_secs < 3600 ? Math.floor(h.uptime_secs/60) + 'm' : Math.floor(h.uptime_secs/3600) + 'h ' + Math.floor((h.uptime_secs%3600)/60) + 'm',
        restartCount: 0, dataDir: path.join(os.homedir(), '.clawnetwork'), balance, syncing: h.status === 'degraded',
      });
    } catch { json(200, { running: false, blockHeight: null, peerCount: null }); }
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
  if (p.startsWith('/api/action/') && req.method === 'POST') {
    const a = p.split('/').pop();
    if (a === 'faucet') {
      try {
        const w = JSON.parse(fs.readFileSync(path.join(os.homedir(), '.openclaw/workspace/clawnetwork/wallet.json'), 'utf8'));
        const r = await rpcCall('claw_faucet', [w.address]);
        json(200, { message: 'Faucet success', ...r });
      } catch (e) { json(400, { error: e.message }); }
      return;
    }
    json(400, { error: 'Use CLI for start/stop: openclaw clawnetwork:start/stop' });
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
  // Check if already running
  const existing = getDashboardUrl()
  if (existing) {
    api.logger?.info?.(`[clawnetwork] dashboard already running: ${existing}`)
    return existing
  }

  // Write the standalone UI server script to a temp file and fork it
  const scriptPath = path.join(WORKSPACE_DIR, 'ui-server.js')
  ensureDir(WORKSPACE_DIR)

  // Write HTML to a separate file, script reads it at startup
  const htmlPath = path.join(WORKSPACE_DIR, 'ui-dashboard.html')
  fs.writeFileSync(htmlPath, buildUiHtml(cfg))

  // Inject HTML path into script (read from file, no template escaping issues)
  const fullScript = `const HTML_PATH = ${JSON.stringify(htmlPath)};\nconst HTML = require('fs').readFileSync(HTML_PATH, 'utf8');\n${UI_SERVER_SCRIPT}`
  fs.writeFileSync(scriptPath, fullScript)

  try {
    const child = fork(scriptPath, [String(cfg.uiPort), String(cfg.rpcPort), LOG_PATH], {
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
        'transfer', to, amount, '--rpc', `http://localhost:${cfg.rpcPort}`,
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
        'agent', 'register', '--name', name,
        '--rpc', `http://localhost:${cfg.rpcPort}`,
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
        'service', 'register',
        '--type', serviceType,
        '--endpoint', endpoint,
        '--description', description,
        '--price', priceAmount,
        '--rpc', `http://localhost:${cfg.rpcPort}`,
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
          execFileSync(binary, ['key', 'import', privateKeyHex], {
            encoding: 'utf8',
            timeout: 10_000,
            env: { HOME: os.homedir(), PATH: process.env.PATH || '' },
          })
          const showOut = execFileSync(binary, ['key', 'show'], {
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
          'transfer', to, amount, '--rpc', `http://localhost:${cfg.rpcPort}`,
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
          'stake', 'deposit', amount, '--rpc', `http://localhost:${cfg.rpcPort}`,
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
          'service', 'register', '--type', serviceType, '--endpoint', endpoint,
          '--rpc', `http://localhost:${cfg.rpcPort}`,
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
            api.logger?.info?.(`[clawnetwork] node already running (pid=${state.pid}), skipping auto-start`)
            startHealthCheck(cfg, api)
            return
          }

          // Step 1: Ensure binary
          let binary = findBinary()
          if (!binary) {
            if (cfg.autoDownload) {
              api.logger?.info?.('[clawnetwork] claw-node not found, downloading...')
              binary = await downloadBinary(api)
            } else {
              api.logger?.error?.('[clawnetwork] claw-node not found and autoDownload is disabled')
              return
            }
          }

          // Step 2: Init
          initNode(binary, cfg.network, api)

          // Step 3: Wallet
          const wallet = ensureWallet(cfg.network, api)

          // Step 4: Start node
          startNodeProcess(binary, cfg, api)

          // Step 5: Start UI dashboard
          startUiServer(cfg, api)

          // Step 6: Wait for node to sync, then auto-register
          await sleep(15_000)
          await autoRegisterAgent(cfg, wallet, api)

          // Step 7: Auto-register as miner + start heartbeat loop
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
