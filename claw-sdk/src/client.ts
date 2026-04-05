// ---------------------------------------------------------------------------
// ClawNetwork SDK — ClawClient (high-level API)
// ---------------------------------------------------------------------------

import { RpcClient, DEFAULT_RPC_URL } from './rpc.js';
import { toHex, fromHex } from './wallet.js';
import {
  signableBytes,
  serializeTransaction,
  encodeAgentRegisterPayload,
  encodeTokenTransferPayload,
  encodeTokenCreatePayload,
  encodeTokenMintTransferPayload,
  encodeReputationAttestPayload,
  encodeServiceRegisterPayload,
} from './serialization.js';
import { TxType } from './types.js';
import type {
  Transaction,
  ClawClientConfig,
  WalletLike,
  AgentIdentity,
  TokenDef,
  ReputationAttestation,
  ServiceEntry,
  BlockInfo,
  TransactionReceipt,
  AgentRegisterParams,
  TokenTransferParams,
  TokenCreateParams,
  TokenMintTransferParams,
  ReputationAttestParams,
  ServiceRegisterParams,
} from './types.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function requireWallet(wallet: WalletLike | undefined): WalletLike {
  if (!wallet) throw new Error('Wallet is required for signing transactions');
  return wallet;
}

// ---------------------------------------------------------------------------
// Sub-modules exposed as client.agent, client.token, etc.
// ---------------------------------------------------------------------------

class AgentModule {
  constructor(
    private rpc: RpcClient,
    private sendTx: (txType: TxType, payload: Uint8Array) => Promise<string>,
  ) {}

  /** Register an agent on-chain. Returns the tx hash. */
  async register(params: AgentRegisterParams): Promise<string> {
    const payload = encodeAgentRegisterPayload(params.name, params.metadata);
    return this.sendTx(TxType.AgentRegister, payload);
  }

  /** Look up an agent by address (hex). */
  async get(address: string): Promise<AgentIdentity | null> {
    return this.rpc.call<AgentIdentity | null>('claw_getAgent', [address]);
  }
}

class TokenModule {
  constructor(
    private rpc: RpcClient,
    private sendTx: (txType: TxType, payload: Uint8Array) => Promise<string>,
  ) {}

  /** Create a new custom token. Returns tx hash. */
  async create(params: TokenCreateParams): Promise<string> {
    const payload = encodeTokenCreatePayload(
      params.name,
      params.symbol,
      params.decimals,
      params.totalSupply,
    );
    return this.sendTx(TxType.TokenCreate, payload);
  }

  /** Transfer a custom token. Returns tx hash. */
  async transfer(params: TokenMintTransferParams): Promise<string> {
    const payload = encodeTokenMintTransferPayload(
      fromHex(params.tokenId),
      fromHex(params.to),
      params.amount,
    );
    return this.sendTx(TxType.TokenMintTransfer, payload);
  }

  /** Get custom token balance. */
  async getBalance(address: string, tokenId: string): Promise<bigint> {
    const result = await this.rpc.call<string>('claw_getTokenBalance', [
      address,
      tokenId,
    ]);
    return BigInt(result);
  }

  /** Get token definition info. */
  async getInfo(tokenId: string): Promise<TokenDef | null> {
    return this.rpc.call<TokenDef | null>('claw_getTokenInfo', [tokenId]);
  }
}

/**
 * @deprecated Reputation system is deprecated. Use Agent Score system instead.
 */
class ReputationModule {
  constructor(
    private rpc: RpcClient,
    private sendTx: (txType: TxType, payload: Uint8Array) => Promise<string>,
  ) {}

  /**
   * Submit a reputation attestation. Returns tx hash.
   * @deprecated Reputation system is deprecated. Use Agent Score system instead.
   */
  async attest(params: ReputationAttestParams): Promise<string> {
    const payload = encodeReputationAttestPayload(
      fromHex(params.to),
      params.category,
      params.score,
      params.platform,
      params.memo,
    );
    return this.sendTx(TxType.ReputationAttest, payload);
  }

  /**
   * Get all reputation attestations for an address.
   * @deprecated Reputation system is deprecated. Use Agent Score system instead.
   */
  async get(address: string): Promise<ReputationAttestation[]> {
    return this.rpc.call<ReputationAttestation[]>('claw_getReputation', [
      address,
    ]);
  }
}

class ServiceModule {
  constructor(
    private rpc: RpcClient,
    private sendTx: (txType: TxType, payload: Uint8Array) => Promise<string>,
  ) {}

  /** Register a service. Returns tx hash. */
  async register(params: ServiceRegisterParams): Promise<string> {
    const payload = encodeServiceRegisterPayload(
      params.serviceType,
      params.description,
      fromHex(params.priceToken),
      params.priceAmount,
      params.endpoint,
      params.active,
    );
    return this.sendTx(TxType.ServiceRegister, payload);
  }

  /** Search services, optionally by type. */
  async search(filter?: { serviceType?: string }): Promise<ServiceEntry[]> {
    const params: unknown[] = filter?.serviceType
      ? [filter.serviceType]
      : [];
    return this.rpc.call<ServiceEntry[]>('claw_getServices', params);
  }
}

class BlockModule {
  constructor(private rpc: RpcClient) {}

  /** Get the latest block number (height). */
  async getLatest(): Promise<number> {
    return this.rpc.call<number>('claw_blockNumber');
  }

  /** Get a block by height. */
  async getByNumber(height: number): Promise<BlockInfo | null> {
    return this.rpc.call<BlockInfo | null>('claw_getBlockByNumber', [height]);
  }
}

// ---------------------------------------------------------------------------
// ClawClient
// ---------------------------------------------------------------------------

export class ClawClient {
  private rpc: RpcClient;
  private wallet?: WalletLike;

  readonly agent: AgentModule;
  readonly token: TokenModule;
  readonly reputation: ReputationModule;
  readonly service: ServiceModule;
  readonly block: BlockModule;

  constructor(config: ClawClientConfig = {}) {
    this.rpc = new RpcClient(config.rpcUrl ?? DEFAULT_RPC_URL);
    this.wallet = config.wallet;

    const sendTx = this.buildAndSendTx.bind(this);
    this.agent = new AgentModule(this.rpc, sendTx);
    this.token = new TokenModule(this.rpc, sendTx);
    this.reputation = new ReputationModule(this.rpc, sendTx);
    this.service = new ServiceModule(this.rpc, sendTx);
    this.block = new BlockModule(this.rpc);
  }

  // --- Top-level convenience methods ---

  /** Transfer native CLAW tokens. Returns tx hash. */
  async transfer(params: TokenTransferParams): Promise<string> {
    const payload = encodeTokenTransferPayload(
      fromHex(params.to),
      params.amount,
    );
    return this.buildAndSendTx(TxType.TokenTransfer, payload);
  }

  /** Get native CLAW balance for an address (hex). */
  async getBalance(address: string): Promise<bigint> {
    const result = await this.rpc.call<string>('claw_getBalance', [address]);
    return BigInt(result);
  }

  /** Get the current nonce for an address (hex). */
  async getNonce(address: string): Promise<number> {
    return this.rpc.call<number>('claw_getNonce', [address]);
  }

  /** Get a transaction receipt by hash (hex). */
  async getTransactionReceipt(
    txHash: string,
  ): Promise<TransactionReceipt | null> {
    return this.rpc.call<TransactionReceipt | null>(
      'claw_getTransactionReceipt',
      [txHash],
    );
  }

  // --- Internal ---

  /**
   * Build a signed transaction and submit it via RPC.
   * Automatically fetches the current nonce.
   */
  private async buildAndSendTx(
    txType: TxType,
    payload: Uint8Array,
  ): Promise<string> {
    const wallet = requireWallet(this.wallet);

    // Fetch current nonce and increment
    const currentNonce = await this.getNonce(wallet.address);
    const nonce = BigInt(currentNonce + 1);

    // Build unsigned transaction
    const tx: Transaction = {
      txType,
      from: wallet.publicKey,
      nonce,
      payload,
      signature: new Uint8Array(64), // placeholder
    };

    // Sign
    const msg = signableBytes(tx);
    tx.signature = await wallet.sign(msg);

    // Serialize and send
    const serialized = serializeTransaction(tx);
    const hex = toHex(serialized);
    return this.rpc.call<string>('claw_sendTransaction', [hex]);
  }
}
