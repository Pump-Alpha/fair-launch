# fair-launch

*Working name, subject to change.* A UBC 2026 Hackathon project (DeFi and Onchain Applications track,
Best use of Solana).

## The problem

Anyone can launch a token on Solana in seconds, and buyers can't tell a fair launch from a setup:
a creator can buy most of the supply at launch and sell it into the first buyers. Track records of
creator wallets don't solve it, because a bad actor just uses a fresh wallet.

## The idea

Let honest creators prove themselves instead. A creator makes a **vow**: they put part of their
token's supply in a vault that a Solana program controls. It unlocks evenly until a date they choose,
so they can keep selling at a steady pace and the market keeps its liquidity. What the vow stops is
the dump: taking out more than has unlocked is allowed, but the program **burns half of the early
part**. The burn is the punishment, and it shrinks the supply, so every other holder's share grows.
The program, not a company, holds creators to it. Anyone can paste a token address and see its badge,
read straight from the chain.

## What we're building (in order)

1. **Vow program** (Anchor, Solana devnet): create a vow, unlock it gradually, withdraw what has
   unlocked, burn half of any early withdrawal. Each vow is an on-chain record: creator, token,
   amount, share of supply, vesting dates, taken out, burned.
2. **Badge page**: paste a token address and see "8% of supply vowed and still locked, unlocking
   until Dec 20; early exit burns half", or "No vow", and a warning if anyone has pulled out early.
   Connect a devnet wallet to make a vow.
3. **Demo token launcher**: mint a test token on devnet in one click so anyone can try the full flow.
4. **Stretch**: an embeddable badge image; real-time vow alerts; a SOL bond forfeited on early exit.

Out of scope: mainnet, real funds, and any list or label of real wallets.

## Status

- Day 1 (2026-11-12): repo, plan and setup.
- Day 2 (2026-11-13): vow program (`programs/vow`) with unit tests and end-to-end tests; badge
  page (`app/`). Then, from a lock to vesting with a burn on early exit (so creators can still sell
  slowly and liquidity is sustained). Not yet deployed to devnet.

## Setup, build and test

Install the tools: [docs/SETUP.md](docs/SETUP.md). Then, in this folder:

```sh
anchor build          # compiles the program; first run creates target/deploy/vow-keypair.json
anchor keys sync      # first time only: writes that keypair's program ID into the code
anchor build          # rebuild with the right ID
cargo test -p vow     # unit tests + end-to-end tests on LiteSVM (an in-memory Solana)
```

## Badge page (`app/`)

A web page that talks to the program from the browser: check any token's vows (share of supply
locked, unlock dates, who locked it, whether minting and freezing are still possible), connect
Phantom and make a vow, withdraw from your own vows (the page says before you sign how much would be
burned, and you confirm it), and launch a test token in one click.

```sh
cd app
npm install
npm test        # checks the hand-written client against the Rust program's layouts
npm run dev     # opens the page at http://localhost:5173
```

Pick Devnet or Localnet at the top. For Localnet, run `solana-test-validator` and deploy the
program to it first (`anchor deploy --provider.cluster localnet`).

## How it works

```
creator ──create_vow(nonce, amount, unlock_at)──▶ vow program ──▶ vow record
   │ tokens ───────────────────────────────────────────────▶ vault (owned by the vow PDA)
   │
   │◀── withdraw(amount, max_burn) ── unlocked part: whole
   │                                  early part:    half to the creator, half burned
   │                                  last withdrawal: vault closed, record kept
```

- **Vesting**: from the moment of the vow to `unlock_at`, tokens unlock evenly (rounded down).
- **Withdraw**: the unlocked tokens not yet taken come out whole. The rest of the request is early,
  and 50% of it is burned (`EARLY_EXIT_PENALTY_BPS`, stored in each vow). Early withdrawals use up
  future unlocks: everything taken, burned included, counts against the schedule.
  `max_burn` is the most the caller agrees to burn, so no one burns tokens by accident.
- **Vow record** (a PDA at seeds `["vow", mint, locker, nonce]`): mint, locker, nonce, amount,
  supply at lock, created / unlock / last-withdrawal times, withdrawn, burned, penalty, whether the
  locker was the mint authority. `mint` is at byte offset 8 and `locker` at 40, so a page can fetch
  every vow on a token with one `getProgramAccounts` memcmp filter.
- **Vault**: the vow PDA's associated token account. Only the program can sign for the PDA, and it
  only does so in `withdraw`, following the rule above, to the locker or the burn.
- **Token programs**: classic SPL Token and Token-2022.
- **No admin**: there is no instruction that lets anyone, including us, move vowed tokens another
  way or skip the burn.

## Economic design and risk assumptions

*Draft; to be finished before submission.*

- **Why vesting, not a lock**: a lock stops the creator selling at all, which isn't how a healthy
  launch works and takes supply off the market. Vesting lets them sell at a steady pace. What it
  stops is the dump: selling a large share at once costs them half of what they take early.
- **Why a burn**: the penalty needs no judge. "Pump and dump" can't be decided on chain without
  trusting someone, so the rule is objective: how fast tokens leave the vault. Burning the penalty
  (rather than paying it to us or to someone) means no one profits from a creator breaking the vow,
  and it lowers supply, so every other holder's share grows.
- **What a vow proves**: that a set amount of supply can only leave on schedule, or at a 50% cost.
  It does not prove the creator holds nothing else: they may keep other tokens in other wallets.
  The badge shows the vowed share of supply, not "the creator's whole stake".
- **What a buyer should read**: "still locked" (in the vault and not yet unlocked) is what the
  creator can't sell without paying the penalty. Any burn so far means someone pulled out early,
  and the badge says so.
- **Who locked it**: anyone can lock tokens they hold. The record says whether the locker was the
  token's mint authority at the time; the badge must show that, so a stranger's vow can't pose as
  the creator's.
- **Dilution**: if the mint authority isn't revoked, the creator could mint more and shrink the
  locked share. The badge should read the mint live and show whether minting is still possible.
- **Dangerous Token-2022 extensions are refused**: permanent delegate (could pull tokens out of the
  vault), transfer hook, default frozen accounts, transfer fees, non-transferable and the rest.
  Only metadata, group and mint-close-authority extensions are allowed.
- **Freeze authority**: a mint with a freeze authority could freeze the vault. That only keeps the
  tokens in longer (it hurts the creator, not buyers), but the badge shows it.
- **Vesting limits**: 60 seconds to 10 years. One minute is for demos; a launchpad using vows would
  set its own minimum (e.g. 30 days).
- **50% may not deter everyone**: a creator who expects to dump into a big rally might still pay
  it. Half of the early part is a large, visible cost, and the badge shows it to every later buyer.
  A launchpad could require a higher penalty; a SOL bond forfeited on early exit is a possible
  next step.
- **Not audited.** Devnet only.

## Program IDs

None deployed yet.
