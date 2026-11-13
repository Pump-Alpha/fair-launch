// Talking to the vow program from the browser, without Anchor's TypeScript client: we build the
// instructions and read the accounts by hand. The program is small enough that this is clearer
// than a generated client, and it needs no IDL file.
//
// Byte layouts come from programs/vow/src/lib.rs. Anchor prefixes every instruction and account
// with an 8-byte "discriminator": the first 8 bytes of sha256("global:<instruction>") or
// sha256("account:<Account>"). The values below were printed from the Rust crate and are checked
// again in vow.test.ts.

import {
  PublicKey,
  SystemProgram,
  TransactionInstruction,
  type Connection,
} from "@solana/web3.js";
import { ASSOCIATED_TOKEN_PROGRAM_ID, getAssociatedTokenAddressSync } from "@solana/spl-token";
import { Buffer } from "buffer";

export const PROGRAM_ID = new PublicKey("3B7936bNMDnFP4no8B9hjqVWrpVwssgH473N3wruUkSD");

export const DISC_CREATE_VOW = Uint8Array.from([78, 21, 71, 231, 45, 131, 0, 45]);
export const DISC_WITHDRAW = Uint8Array.from([183, 18, 70, 156, 148, 109, 161, 34]);
export const DISC_VOW_ACCOUNT = Uint8Array.from([133, 240, 130, 20, 17, 215, 31, 12]);

/** Size of a vow account: 8-byte discriminator + 132 bytes of fields. */
export const VOW_ACCOUNT_SIZE = 140;
/** Byte offset of `mint` inside a vow account (right after the discriminator). */
export const MINT_OFFSET = 8;
/** Byte offset of `locker`. */
export const LOCKER_OFFSET = 40;

export interface Vow {
  address: PublicKey;
  mint: PublicKey;
  locker: PublicKey;
  nonce: bigint;
  amount: bigint;
  supplyAtLock: bigint;
  createdAt: number; // unix seconds; vesting starts here
  unlockAt: number; // ...and ends here, fully unlocked
  releasedAt: number; // 0 while tokens remain in the vault
  withdrawn: bigint; // sent back to the locker so far
  burned: bigint; // burned as the early-exit penalty so far
  penaltyBps: number; // share of early withdrawals burned (5000 = 50%)
  lockerIsMintAuthority: boolean;
  bump: number;
}

function u64le(n: bigint): Uint8Array {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, n, true);
  return b;
}

function i64le(n: bigint): Uint8Array {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigInt64(0, n, true);
  return b;
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

/** The vow record's address: a PDA at seeds ["vow", mint, locker, nonce as u64 little-endian]. */
export function vowAddress(mint: PublicKey, locker: PublicKey, nonce: bigint): PublicKey {
  return PublicKey.findProgramAddressSync(
    [new TextEncoder().encode("vow"), mint.toBytes(), locker.toBytes(), u64le(nonce)],
    PROGRAM_ID,
  )[0];
}

/** The vault: the vow PDA's associated token account. */
export function vaultAddress(vow: PublicKey, mint: PublicKey, tokenProgram: PublicKey): PublicKey {
  return getAssociatedTokenAddressSync(mint, vow, true, tokenProgram);
}

export function createVowData(nonce: bigint, amount: bigint, unlockAt: bigint): Uint8Array {
  return concat(DISC_CREATE_VOW, u64le(nonce), u64le(amount), i64le(unlockAt));
}

export function createVowInstruction(args: {
  locker: PublicKey;
  mint: PublicKey;
  lockerTokens: PublicKey;
  tokenProgram: PublicKey;
  nonce: bigint;
  amount: bigint;
  unlockAt: bigint;
}): TransactionInstruction {
  const vow = vowAddress(args.mint, args.locker, args.nonce);
  // Account order and flags must match `CreateVow` in lib.rs.
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: args.locker, isSigner: true, isWritable: true },
      { pubkey: args.mint, isSigner: false, isWritable: false },
      { pubkey: args.lockerTokens, isSigner: false, isWritable: true },
      { pubkey: vow, isSigner: false, isWritable: true },
      { pubkey: vaultAddress(vow, args.mint, args.tokenProgram), isSigner: false, isWritable: true },
      { pubkey: args.tokenProgram, isSigner: false, isWritable: false },
      { pubkey: ASSOCIATED_TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: Buffer.from(createVowData(args.nonce, args.amount, args.unlockAt)),
  });
}

export function withdrawData(amount: bigint, maxBurn: bigint): Uint8Array {
  return concat(DISC_WITHDRAW, u64le(amount), u64le(maxBurn));
}

export function withdrawInstruction(args: {
  vow: Vow;
  lockerTokens: PublicKey;
  tokenProgram: PublicKey;
  amount: bigint;
  maxBurn: bigint;
}): TransactionInstruction {
  const { vow } = args;
  // Account order and flags must match `Withdraw` in lib.rs. The mint is writable because a
  // burn lowers its supply.
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: vow.locker, isSigner: true, isWritable: true },
      { pubkey: vow.mint, isSigner: false, isWritable: true },
      { pubkey: args.lockerTokens, isSigner: false, isWritable: true },
      { pubkey: vow.address, isSigner: false, isWritable: true },
      { pubkey: vaultAddress(vow.address, vow.mint, args.tokenProgram), isSigner: false, isWritable: true },
      { pubkey: args.tokenProgram, isSigner: false, isWritable: false },
    ],
    data: Buffer.from(withdrawData(args.amount, args.maxBurn)),
  });
}

/** Tokens unlocked by `now`, evenly from createdAt to unlockAt (same rounding as `vested` in lib.rs). */
export function vested(amount: bigint, createdAt: number, unlockAt: number, now: number): bigint {
  if (now >= unlockAt) return amount;
  if (now <= createdAt) return 0n;
  return (amount * BigInt(now - createdAt)) / BigInt(unlockAt - createdAt);
}

/** Tokens still in the vow. */
export function remaining(vow: Vow): bigint {
  return vow.amount - vow.withdrawn - vow.burned;
}

/** Unlocked tokens the locker can take now with no burn. */
export function freeNow(vow: Vow, now: number): bigint {
  const v = vested(vow.amount, vow.createdAt, vow.unlockAt, now) - vow.withdrawn - vow.burned;
  return v > 0n ? v : 0n;
}

/** How a withdrawal of `amount` splits, mirroring `plan_withdraw` in lib.rs. */
export function planWithdraw(vow: Vow, now: number, amount: bigint) {
  const unlockedPart = amount < freeNow(vow, now) ? amount : freeNow(vow, now);
  const early = amount - unlockedPart;
  const burn = (early * BigInt(vow.penaltyBps) + 9_999n) / 10_000n;
  return { toLocker: amount - burn, burn, early };
}

/** Read a vow account's bytes. Returns null if they aren't a vow. */
export function decodeVow(address: PublicKey, data: Uint8Array): Vow | null {
  if (data.length < VOW_ACCOUNT_SIZE) return null;
  for (let i = 0; i < 8; i++) if (data[i] !== DISC_VOW_ACCOUNT[i]) return null;
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  return {
    address,
    mint: new PublicKey(data.slice(8, 40)),
    locker: new PublicKey(data.slice(40, 72)),
    nonce: v.getBigUint64(72, true),
    amount: v.getBigUint64(80, true),
    supplyAtLock: v.getBigUint64(88, true),
    createdAt: Number(v.getBigInt64(96, true)),
    unlockAt: Number(v.getBigInt64(104, true)),
    releasedAt: Number(v.getBigInt64(112, true)),
    withdrawn: v.getBigUint64(120, true),
    burned: v.getBigUint64(128, true),
    penaltyBps: v.getUint16(136, true),
    lockerIsMintAuthority: data[138] === 1,
    bump: data[139],
  };
}

/** Every vow on one token, via one RPC call filtered by the discriminator and the mint's bytes. */
export async function fetchVowsForMint(connection: Connection, mint: PublicKey): Promise<Vow[]> {
  const accounts = await connection.getProgramAccounts(PROGRAM_ID, {
    filters: [
      { dataSize: VOW_ACCOUNT_SIZE },
      { memcmp: { offset: 0, bytes: bs58(DISC_VOW_ACCOUNT) } },
      { memcmp: { offset: MINT_OFFSET, bytes: mint.toBase58() } },
    ],
  });
  return accounts
    .map((a) => decodeVow(a.pubkey, a.account.data))
    .filter((v): v is Vow => v !== null)
    .sort((a, b) => a.createdAt - b.createdAt);
}

/** Every vow one wallet has made, on any token. */
export async function fetchVowsByLocker(connection: Connection, locker: PublicKey): Promise<Vow[]> {
  const accounts = await connection.getProgramAccounts(PROGRAM_ID, {
    filters: [
      { dataSize: VOW_ACCOUNT_SIZE },
      { memcmp: { offset: 0, bytes: bs58(DISC_VOW_ACCOUNT) } },
      { memcmp: { offset: LOCKER_OFFSET, bytes: locker.toBase58() } },
    ],
  });
  return accounts
    .map((a) => decodeVow(a.pubkey, a.account.data))
    .filter((v): v is Vow => v !== null)
    .sort((a, b) => b.createdAt - a.createdAt);
}

/** Lowest nonce (0, 1, 2, ...) this wallet hasn't used on this token yet. */
export async function nextNonce(connection: Connection, mint: PublicKey, locker: PublicKey): Promise<bigint> {
  for (let n = 0n; n < 100n; n++) {
    const info = await connection.getAccountInfo(vowAddress(mint, locker, n));
    if (!info) return n;
  }
  throw new Error("This wallet already has 100 vows on this token");
}

/** Share of supply in percent, to two decimals (matches the program's basis points). */
export function sharePercent(amount: bigint, supply: bigint): number {
  if (supply === 0n) return 0;
  const bps = (amount * 10_000n) / supply;
  return Number(bps > 10_000n ? 10_000n : bps) / 100;
}

// Tiny base58 encoder (Bitcoin alphabet), so a byte filter can be passed to getProgramAccounts.
const ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
export function bs58(bytes: Uint8Array): string {
  let n = 0n;
  for (const b of bytes) n = n * 256n + BigInt(b);
  let out = "";
  while (n > 0n) {
    out = ALPHABET[Number(n % 58n)] + out;
    n /= 58n;
  }
  for (const b of bytes) {
    if (b !== 0) break;
    out = "1" + out;
  }
  return out;
}
