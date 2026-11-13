# Setup (macOS)

About 30-60 minutes, most of it waiting for Rust to compile. Everything is free.

## 1. Install the Solana tools

Solana's official installer sets up Rust, the Solana CLI, Anchor, Surfpool (a local test validator), Node.js and Yarn in
one go ([Solana docs: Installation](https://solana.com/docs/intro/installation)). In Terminal:

```sh
curl --proto '=https' --tlsv1.2 -sSfL https://solana-install.solana.workers.dev | bash
```

Close and reopen Terminal, then check:

```sh
rustc --version && solana --version && anchor --version && node --version && yarn --version
```

Paste the output to Claude so we pin the same versions in the project.

## 2. Point the CLI at devnet and make a devnet wallet

```sh
solana config set --url devnet
solana-keygen new
```

`solana-keygen new` creates a **devnet-only** test wallet at `~/.config/solana/id.json`, outside this
repo. It asks for an optional passphrase and prints a seed phrase. Don't use this wallet for anything
real, and never put the file or the phrase in the repo or a chat.

## 3. Get free devnet SOL

```sh
solana airdrop 2
solana balance
```

If the airdrop is rate-limited, use the web faucet at [faucet.solana.com](https://faucet.solana.com)
with the address from `solana address`.

## 4. Phantom on devnet (for the badge page)

In Phantom: Settings, Developer Settings, turn on Testnet Mode and choose Solana Devnet. Use a
separate Phantom account for testing, not one that holds real funds.

## 5. Clone this repo

```sh
git clone https://github.com/Pump-Alpha/fair-launch.git
cd fair-launch
```

Work in this clone, not inside any other project's folder.
