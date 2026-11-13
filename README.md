# fair-launch

*Working name, subject to change.* A UBC 2026 Hackathon project (DeFi and Onchain Applications track,
Best use of Solana).

## The problem

Anyone can launch a token on Solana in seconds, and buyers can't tell a fair launch from a setup:
a creator can buy most of the supply at launch and sell it into the first buyers. Track records of
creator wallets don't solve it, because a bad actor just uses a fresh wallet.

## The idea

Let honest creators prove themselves instead. A creator makes a **vow**: they lock part of their
token's supply in a vault that a Solana program controls, until a date they choose. The program, not
a company, holds them to it: an early withdrawal is refused on chain. Anyone can paste a token address
and see its badge, read straight from the chain.

## What we're building (in order)

1. **Vow program** (Anchor, Solana devnet): create a vow, lock tokens, refuse early withdrawal,
   release after the unlock date. Each vow is an on-chain record: creator, token, amount, share of
   supply, unlock time.
2. **Badge page**: paste a token address and see "Creator locked 10% of supply until Dec 20" or
   "No vow". Connect a devnet wallet to make a vow.
3. **Demo token launcher**: mint a test token on devnet in one click so anyone can try the full flow.
4. **Stretch**: linear vesting instead of one unlock date; an embeddable badge image; real-time vow
   alerts.

Out of scope: mainnet, real funds, and any list or label of real wallets.

## Status

- Day 1 (2026-11-12): repo, plan and setup.
- Day 2 (2026-11-13): vow program (`programs/vow`) with unit tests and end-to-end tests. Not yet
  deployed to devnet.

## Setup, build and test

Install the tools: [docs/SETUP.md](docs/SETUP.md). Then, in this folder:

```sh
anchor build          # compiles the program; first run creates target/deploy/vow-keypair.json
anchor keys sync      # first time only: writes that keypair's program ID into the code
anchor build          # rebuild with the right ID
cargo test -p vow     # unit tests + end-to-end tests on LiteSVM (an in-memory Solana)
```

## How it works

```
creator wallet ──create_vow(nonce, amount, unlock_at)──▶ vow program
      │                                                     │ writes
      │ tokens ──────────────────────────────────────▶ vault (token account owned by the vow PDA)
      │                                                     │
      │◀──────── release_vow() after unlock_at ─────────────┘ tokens back, vault closed,
                                                              vow record kept, marked released
```

- **Vow record** (a PDA at seeds `["vow", mint, locker, nonce]`): mint, locker, nonce, amount,
  supply at lock, created / unlock / released times, whether the locker was the mint authority.
  `mint` is at byte offset 8 and `locker` at 40, so a page can fetch every vow on a token with one
  `getProgramAccounts` memcmp filter.
- **Vault**: the vow PDA's associated token account. Only the program can sign for the PDA, and it
  only does so in `release_vow`, after the unlock time, back to the locker.
- **Token programs**: classic SPL Token and Token-2022.
- **No admin**: there is no instruction that lets anyone, including us, move locked tokens early.

## Economic design and risk assumptions

*Draft; to be finished before submission.*

- **What a vow proves**: that a set amount of supply cannot be sold before a date. It does not
  prove the creator holds nothing else: they may keep other tokens in other wallets. The badge
  shows locked share of supply, not "the creator's whole stake".
- **Who locked it**: anyone can lock tokens they hold. The record says whether the locker was the
  token's mint authority at the time; the badge must show that, so a stranger's vow can't pose as
  the creator's.
- **Dilution**: if the mint authority isn't revoked, the creator could mint more and shrink the
  locked share. The badge should read the mint live and show whether minting is still possible.
- **Dangerous Token-2022 extensions are refused**: permanent delegate (could pull tokens out of the
  vault), transfer hook, default frozen accounts, transfer fees, non-transferable and the rest.
  Only metadata, group and mint-close-authority extensions are allowed.
- **Freeze authority**: a mint with a freeze authority could freeze the vault. That only keeps the
  tokens locked longer (it hurts the creator, not buyers), but the badge should show it.
- **Lock limits**: 60 seconds to 10 years. One minute is for demos; a launchpad using vows would
  set its own minimum (e.g. 30 days).
- **Not audited.** Devnet only.

## Program IDs

None deployed yet.
