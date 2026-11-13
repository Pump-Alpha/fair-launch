// Checks that the hand-written client in vow.ts matches the Rust program.
// Run: npm test (in app/)

import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { PublicKey } from "@solana/web3.js";
import {
  DISC_CREATE_VOW,
  DISC_WITHDRAW,
  DISC_VOW_ACCOUNT,
  VOW_ACCOUNT_SIZE,
  bs58,
  createVowData,
  decodeVow,
  planWithdraw,
  vested,
  withdrawData,
  sharePercent,
  vowAddress,
} from "./vow.ts";

const first8 = (s: string) => Uint8Array.from(createHash("sha256").update(s).digest().subarray(0, 8));

test("discriminators are Anchor's sha256 prefixes", () => {
  assert.deepEqual(DISC_CREATE_VOW, first8("global:create_vow"));
  assert.deepEqual(DISC_WITHDRAW, first8("global:withdraw"));
  assert.deepEqual(DISC_VOW_ACCOUNT, first8("account:Vow"));
});

test("vow address matches the Rust program's PDA", () => {
  // Expected value printed by Pubkey::find_program_address in Rust for the same inputs.
  const mint = new PublicKey(new Uint8Array(32).fill(1));
  const locker = new PublicKey(new Uint8Array(32).fill(2));
  assert.equal(vowAddress(mint, locker, 3n).toBase58(), "88JoAKbMdfQjHPMRo2wxyTUjJL2jDSpHsWVy4kfn2QaV");
});

test("create_vow data is discriminator + nonce + amount + unlock_at", () => {
  const data = createVowData(1n, 500n, 1_800_000_000n);
  assert.equal(data.length, 32);
  const v = new DataView(data.buffer);
  assert.equal(v.getBigUint64(8, true), 1n);
  assert.equal(v.getBigUint64(16, true), 500n);
  assert.equal(v.getBigInt64(24, true), 1_800_000_000n);
});

test("decodeVow reads the account layout", () => {
  const data = new Uint8Array(VOW_ACCOUNT_SIZE);
  data.set(DISC_VOW_ACCOUNT, 0);
  data.fill(1, 8, 40); // mint
  data.fill(2, 40, 72); // locker
  const v = new DataView(data.buffer);
  v.setBigUint64(72, 3n, true);
  v.setBigUint64(80, 100n, true);
  v.setBigUint64(88, 1000n, true);
  v.setBigInt64(96, 10n, true);
  v.setBigInt64(104, 20n, true);
  v.setBigInt64(112, 0n, true);
  v.setBigUint64(120, 7n, true);
  v.setBigUint64(128, 5n, true);
  v.setUint16(136, 5000, true);
  data[138] = 1;
  data[139] = 254;
  const addr = new PublicKey(new Uint8Array(32).fill(9));
  const vow = decodeVow(addr, data)!;
  assert.equal(vow.mint.toBase58(), new PublicKey(new Uint8Array(32).fill(1)).toBase58());
  assert.equal(vow.locker.toBase58(), new PublicKey(new Uint8Array(32).fill(2)).toBase58());
  assert.equal(vow.nonce, 3n);
  assert.equal(vow.amount, 100n);
  assert.equal(vow.supplyAtLock, 1000n);
  assert.equal(vow.unlockAt, 20);
  assert.equal(vow.releasedAt, 0);
  assert.equal(vow.withdrawn, 7n);
  assert.equal(vow.burned, 5n);
  assert.equal(vow.penaltyBps, 5000);
  assert.equal(vow.lockerIsMintAuthority, true);
  assert.equal(vow.bump, 254);
  data[0] ^= 1;
  assert.equal(decodeVow(addr, data), null, "wrong discriminator is not a vow");
});

test("withdraw data is discriminator + amount + max_burn", () => {
  const data = withdrawData(500n, 250n);
  assert.equal(data.length, 24);
  const v = new DataView(data.buffer);
  assert.equal(v.getBigUint64(8, true), 500n);
  assert.equal(v.getBigUint64(16, true), 250n);
});

test("vesting and the early-exit split match the program's unit tests", () => {
  const NOW = 1_800_000_000;
  assert.equal(vested(1000n, NOW, NOW + 1000, NOW + 250), 250n);
  assert.equal(vested(10n, NOW, NOW + 3, NOW + 1), 3n);
  assert.equal(vested(1000n, NOW, NOW + 1000, NOW + 5000), 1000n);
  const vow = { amount: 1000n, createdAt: NOW, unlockAt: NOW + 1000, withdrawn: 0n, burned: 0n, penaltyBps: 5000 } as never;
  assert.deepEqual(planWithdraw(vow, NOW + 300, 300n), { toLocker: 300n, burn: 0n, early: 0n });
  assert.deepEqual(planWithdraw(vow, NOW + 300, 500n), { toLocker: 400n, burn: 100n, early: 200n });
  assert.deepEqual(planWithdraw(vow, NOW, 3n), { toLocker: 1n, burn: 2n, early: 3n });
  const after = { amount: 1000n, createdAt: NOW, unlockAt: NOW + 1000, withdrawn: 400n, burned: 100n, penaltyBps: 5000 } as never;
  assert.deepEqual(planWithdraw(after, NOW + 500, 100n), { toLocker: 50n, burn: 50n, early: 100n });
  assert.deepEqual(planWithdraw(after, NOW + 700, 200n), { toLocker: 200n, burn: 0n, early: 0n });
});

test("share of supply matches the program's rounding", () => {
  assert.equal(sharePercent(100n, 1000n), 10);
  assert.equal(sharePercent(1n, 3n), 33.33);
  assert.equal(sharePercent(5n, 0n), 0);
});

test("bs58 matches web3.js", () => {
  const bytes = new Uint8Array(32).fill(7);
  assert.equal(bs58(bytes), new PublicKey(bytes).toBase58());
  assert.equal(bs58(Uint8Array.from([0, 0, 1])), "112");
});
