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

Day 1 (2026-11-12): repo created, plan and setup written. No program code yet.

## Setup

See [docs/SETUP.md](docs/SETUP.md).

## Economic design and risk assumptions

To be written as the program takes shape (required for the DeFi track).

## Program IDs

None deployed yet.
