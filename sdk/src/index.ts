import { sha256 } from "@noble/hashes/sha256";

// ── Types ──────────────────────────────────────────────────────────────

export interface RpcRequest {
  jsonrpc: "2.0";
  method: string;
  params?: unknown[];
  id: number;
}

export interface RpcResponse<T> {
  jsonrpc: "2.0";
  result: T | null;
  error?: string;
  id: number;
}

export interface SlotInfo {
  slot: number;
  parentSlot?: number;
  numTransactions?: number;
  merkleRoot?: string;
}

export interface ProofResult {
  slot: number;
  txIndex: number;
  leaf: string;
  proof: string[];
  root: string;
  verified: boolean;
}

export interface TransactionResult {
  slot: number;
  txIndex: number;
  transaction: string;
  leaf: string;
  proof: string[];
  root: string;
  verified: boolean;
}

export interface BlockTransaction {
  index: number;
  transaction?: string;
  signatures: string[];
  leaf: string;
  proof: string[];
}

export interface BlockWithProofs {
  slot: number;
  parentSlot: number;
  numTransactions: number;
  merkleRoot: string;
  transactions: BlockTransaction[];
}

export interface SolanaBlock {
  blockhash: string | null;
  previousBlockhash: string | null;
  parentSlot: number;
  transactions: {
    transaction: string[] | { signatures: string[]; message: unknown };
    meta: unknown | null;
    version: string;
  }[];
  blockTime: number | null;
  blockHeight: number | null;
  numTransactions: number;
}

// ── Merkle proof verifier ──────────────────────────────────────────────

function hashPair(a: Uint8Array, b: Uint8Array): Uint8Array {
  const h = sha256.create();
  h.update(a);
  h.update(b);
  return h.digest();
}

export function verifyMerkleProof(
  rootHex: string,
  leafHex: string,
  proofHexes: string[],
  index: number,
): boolean {
  const root = hexToBytes(rootHex);
  let hash = hexToBytes(leafHex);
  let idx = index;

  for (const siblingHex of proofHexes) {
    const sibling = hexToBytes(siblingHex);
    hash = idx % 2 === 0 ? hashPair(hash, sibling) : hashPair(sibling, hash);
    idx = Math.floor(idx / 2);
  }

  return bytesEqual(hash, root);
}

// ── Helpers ────────────────────────────────────────────────────────────

function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.substr(i * 2, 2), 16);
  }
  return bytes;
}

function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

// ── Client ─────────────────────────────────────────────────────────────

export class Sg32Client {
  private url: string;
  private nextId = 1;

  constructor(url?: string) {
    this.url = url ?? "http://127.0.0.1:8899";
  }

  private async call<T>(method: string, params?: unknown[]): Promise<T> {
    const body: RpcRequest = {
      jsonrpc: "2.0",
      method,
      params,
      id: this.nextId++,
    };

    const res = await fetch(this.url + "/jsonrpc", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });

    const json: RpcResponse<T> = await res.json();
    if (json.error) throw new Error(json.error);
    return json.result as T;
  }

  // ── Standard Solana methods ──────────────────────────────────────

  /** Returns the latest known slot number. */
  getSlot(): Promise<number> {
    return this.call<number>("getSlot");
  }

  /** Returns a block with base64-encoded transactions (Solana format). */
  getBlock(slot: number, encoding: "base64" | "json" = "base64"): Promise<SolanaBlock> {
    return this.call<SolanaBlock>("getBlock", [slot, encoding]);
  }

  // ── Trustless extension methods ──────────────────────────────────

  /** Returns a Merkle proof for a transaction at the given slot and index. */
  getProof(slot: number, txIndex: number): Promise<ProofResult> {
    return this.call<ProofResult>("getProof", [slot, txIndex]);
  }

  /** Returns the full transaction bytes + Merkle proof. */
  getTransactionByIndex(slot: number, txIndex: number): Promise<TransactionResult> {
    return this.call<TransactionResult>("getTransactionByIndex", [slot, txIndex]);
  }

  /** Returns all transactions in a slot with their Merkle proofs. */
  getBlockWithProofs(slot: number, includeTxData?: boolean): Promise<BlockWithProofs> {
    return this.call<BlockWithProofs>("getBlockWithProofs", [
      slot,
      includeTxData ?? true,
    ]);
  }

  // ── Convenience ──────────────────────────────────────────────────

  /**
   * Fetches a transaction and verifies its Merkle proof.
   * Returns the transaction data only if verification passes.
   */
  async getVerifiedTransaction(slot: number, txIndex: number): Promise<{
    transaction: string;
    proof: ProofResult;
  }> {
    const tx = await this.getTransactionByIndex(slot, txIndex);
    if (!tx.verified) {
      throw new Error(`Merkle proof verification failed for slot ${slot} tx ${txIndex}`);
    }
    return { transaction: tx.transaction, proof: tx };
  }

  /**
   * Fetches proof and independently verifies it client-side.
   * Does NOT trust the node's `verified` field.
   */
  async getProofAndVerify(slot: number, txIndex: number): Promise<{
    proof: ProofResult;
    clientVerified: boolean;
  }> {
    const proof = await this.getProof(slot, txIndex);
    const clientVerified = verifyMerkleProof(
      proof.root,
      proof.leaf,
      proof.proof,
      proof.txIndex,
    );
    return { proof, clientVerified };
  }
}

// ── Standalone Merkle verification (no RPC needed) ────────────────────

export { verifyMerkleProof as verify };
export default Sg32Client;
