// Checks that the hand-written client in vow.ts matches the Rust program.
// Run: npm test (in app/)

import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { PublicKey } from "@solana/web3.js";
import {
  DISC_CREATE_VOW,
  DISC_RELEASE_VOW,
  DISC_VOW_ACCOUNT,
  VOW_ACCOUNT_SIZE,
  bs58,
  createVowData,
  decodeVow,
  sharePercent,
  vowAddress,
} from "./vow.ts";

const first8 = (s: string) => Uint8Array.from(createHash("sha256").update(s).digest().subarray(0, 8));

test("discriminators are Anchor's sha256 prefixes", () => {
  assert.deepEqual(DISC_CREATE_VOW, first8("global:create_vow"));
  assert.deepEqual(DISC_RELEASE_VOW, first8("global:release_vow"));
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
  data[120] = 1;
  data[121] = 254;
  const addr = new PublicKey(new Uint8Array(32).fill(9));
  const vow = decodeVow(addr, data)!;
  assert.equal(vow.mint.toBase58(), new PublicKey(new Uint8Array(32).fill(1)).toBase58());
  assert.equal(vow.locker.toBase58(), new PublicKey(new Uint8Array(32).fill(2)).toBase58());
  assert.equal(vow.nonce, 3n);
  assert.equal(vow.amount, 100n);
  assert.equal(vow.supplyAtLock, 1000n);
  assert.equal(vow.unlockAt, 20);
  assert.equal(vow.releasedAt, 0);
  assert.equal(vow.lockerIsMintAuthority, true);
  assert.equal(vow.bump, 254);
  data[0] ^= 1;
  assert.equal(decodeVow(addr, data), null, "wrong discriminator is not a vow");
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
