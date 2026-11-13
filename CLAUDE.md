# CLAUDE.md — fair-launch

Context for Claude. Read before any work here. Keep it current: update it in the same change when a
decision is made or a rule changes.

## What this is

A UBC 2026 Hackathon entry (online build Nov 12 6 PM CST to Nov 20 12 PM CST; finals in Austin
Nov 21). Team: Grayson (builds, with Claude Code) and his partner (markets, README economics, pitch).
Track: DeFi and Onchain Applications, chain award Best use of Solana. The owner is new to Solana
programs: explain non-obvious choices in comments and commit messages, and keep him able to explain
every file (the rules say builders stay the primary creators).

## Hard rules

1. **Devnet only.** No mainnet deploys, no mainnet RPC writes, no real SOL or tokens.
2. **No keys in the repo.** Never generate, commit, print or log a keypair, seed phrase or API key in
   this repo, devnet ones included. The Solana CLI keeps its devnet wallet in `~/.config/solana/`;
   program keypairs live in `target/deploy/` (gitignored).
3. **All code written during the hackathon.** Nothing copied from any other repo of the owner's.
   Open-source libraries are fine.
4. **No real wallet records.** No lists, scores or labels of real wallets or people, on any network.
   Demo data comes from devnet test wallets we create.
5. **Keep the owner's other projects out of this repo** (code, data, docs, pitch): his choice.
6. **Nothing that spends money** (paid plans, tickets, services) without the owner's OK in that session.
7. Work on a branch and open a pull request; the owner merges.

## Stack

Anchor (Rust) program on Solana devnet; TypeScript web page for the badge; Phantom wallet on devnet.
Setup: `docs/SETUP.md`.

## Submission checklist (Devpost)

Public repo, working demo, 2-3 min demo video, README with technical docs and architecture, program
ID(s), the DeFi track's "economic design and risk assumptions", written answers (what, problem, how,
unique, next).
