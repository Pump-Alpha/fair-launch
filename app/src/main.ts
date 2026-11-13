// The badge page: check a token's vows, make a vow, release your own, and launch a test token.
// Plain TypeScript and the DOM, no framework, so every line can be read top to bottom.

import { Buffer } from "buffer";
(globalThis as unknown as { Buffer: typeof Buffer }).Buffer = Buffer; // web3.js expects Node's Buffer

import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  type TransactionInstruction,
} from "@solana/web3.js";
import {
  AuthorityType,
  MINT_SIZE,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountIdempotentInstruction,
  createInitializeMint2Instruction,
  createMintToInstruction,
  createSetAuthorityInstruction,
  getAssociatedTokenAddressSync,
  getMint,
  getMinimumBalanceForRentExemptMint,
  type Mint,
} from "@solana/spl-token";
import {
  PROGRAM_ID,
  createVowInstruction,
  fetchVowsByLocker,
  fetchVowsForMint,
  nextNonce,
  releaseVowInstruction,
  sharePercent,
  type Vow,
} from "./vow.ts";

// ---------- network and wallet ----------

const RPC: Record<string, string> = {
  devnet: "https://api.devnet.solana.com",
  localnet: "http://127.0.0.1:8899",
};

interface PhantomProvider {
  isPhantom?: boolean;
  publicKey: PublicKey | null;
  connect(): Promise<{ publicKey: PublicKey }>;
  signTransaction(tx: Transaction): Promise<Transaction>;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const clusterSelect = $<HTMLSelectElement>("cluster");
const params = new URLSearchParams(location.search);
clusterSelect.value = params.get("cluster") === "localnet" ? "localnet" : "devnet";

let connection = new Connection(RPC[clusterSelect.value], "confirmed");
let wallet: PublicKey | null = null;
let lastLaunchedMint: PublicKey | null = null;

clusterSelect.addEventListener("change", () => {
  connection = new Connection(RPC[clusterSelect.value], "confirmed");
  const url = new URL(location.href);
  url.searchParams.set("cluster", clusterSelect.value);
  history.replaceState(null, "", url);
  void refreshWallet();
});

$("program-id").textContent = PROGRAM_ID.toBase58();

function phantom(): PhantomProvider | null {
  const w = window as unknown as { phantom?: { solana?: PhantomProvider }; solana?: PhantomProvider };
  return w.phantom?.solana ?? (w.solana?.isPhantom ? w.solana : null);
}

$("connect").addEventListener("click", async () => {
  const p = phantom();
  if (!p) {
    window.open("https://phantom.com/", "_blank");
    return;
  }
  const { publicKey } = await p.connect();
  wallet = new PublicKey(publicKey.toString());
  await refreshWallet();
});

async function refreshWallet() {
  if (!wallet) return;
  const lamports = await connection.getBalance(wallet).catch(() => null);
  $("wallet").textContent =
    `${short(wallet)} · ${lamports === null ? "?" : (lamports / 1e9).toFixed(3)} SOL`;
  $<HTMLButtonElement>("connect").textContent = "Connected";
  await renderMine();
}

/** Sign with Phantom and send through our own RPC, so the network picker decides the network. */
async function send(ixs: TransactionInstruction[], extraSigners: Keypair[] = []): Promise<string> {
  const p = phantom();
  if (!p || !wallet) throw new Error("Connect Phantom first");
  const tx = new Transaction().add(...ixs);
  tx.feePayer = wallet;
  const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash();
  tx.recentBlockhash = blockhash;
  if (extraSigners.length) tx.partialSign(...extraSigners);
  const signed = await p.signTransaction(tx);
  const sig = await connection.sendRawTransaction(signed.serialize());
  await connection.confirmTransaction({ signature: sig, blockhash, lastValidBlockHeight }, "confirmed");
  return sig;
}

// ---------- helpers ----------

const short = (k: PublicKey) => `${k.toBase58().slice(0, 4)}…${k.toBase58().slice(-4)}`;
const when = (unix: number) => new Date(unix * 1000).toLocaleString();
const nowUnix = () => Math.floor(Date.now() / 1000);

function escape(s: string): string {
  return s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

function explorer(kind: "address" | "tx", id: string): string {
  const cluster = clusterSelect.value === "devnet" ? "devnet" : "custom&customUrl=http%3A%2F%2F127.0.0.1%3A8899";
  return `https://explorer.solana.com/${kind}/${id}?cluster=${cluster}`;
}

function countdown(unlockAt: number): string {
  const s = unlockAt - nowUnix();
  if (s <= 0) return "unlockable now";
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
  return d > 0 ? `${d}d ${h}h left` : h > 0 ? `${h}h ${m}m left` : `${m}m ${s % 60}s left`;
}

function formatTokens(amount: bigint, decimals: number): string {
  const base = 10n ** BigInt(decimals);
  const whole = amount / base;
  const frac = amount % base;
  const fracStr = decimals ? frac.toString().padStart(decimals, "0").replace(/0+$/, "") : "";
  return whole.toLocaleString() + (fracStr ? "." + fracStr.slice(0, 4) : "");
}

function parseTokens(text: string, decimals: number): bigint {
  const [whole, frac = ""] = text.trim().split(".");
  if (!/^\d+$/.test(whole || "0") || !/^\d*$/.test(frac)) throw new Error("Amount must be a number");
  return BigInt(whole || "0") * 10n ** BigInt(decimals) + BigInt((frac + "0".repeat(decimals)).slice(0, decimals) || "0");
}

// The program's errors, numbered from 6000 in the order they appear in lib.rs.
const PROGRAM_ERRORS = [
  "Unlock time must be at least one minute from now",
  "Unlock time must be within 10 years",
  "Amount must be more than zero",
  "This vow is still locked",
  "This vow was already released",
  "Only the wallet that made the vow can release it",
  "This token has a Token-2022 extension that could break the lock",
];

function explainError(e: unknown, logs?: string[] | null): string {
  const text = [String((e as Error)?.message ?? e), ...(logs ?? [])].join("\n");
  const hex = text.match(/custom program error: 0x([0-9a-f]+)/i);
  if (hex) {
    const code = parseInt(hex[1], 16);
    if (code >= 6000 && code < 6000 + PROGRAM_ERRORS.length) return PROGRAM_ERRORS[code - 6000];
  }
  if (/Invalid public key/i.test(text)) return "That isn't a valid Solana address";
  return text.split("\n")[0];
}

function message(el: HTMLElement, kind: "ok" | "bad" | "info", html: string) {
  el.innerHTML = `<div class="msg ${kind}">${html}</div>`;
}

async function tokenProgramOf(mint: PublicKey): Promise<PublicKey> {
  const info = await connection.getAccountInfo(mint);
  if (!info) throw new Error("No account at that address on this network");
  if (info.owner.equals(TOKEN_PROGRAM_ID) || info.owner.equals(TOKEN_2022_PROGRAM_ID)) return info.owner;
  throw new Error("That address isn't a token mint");
}

// ---------- check a token ----------

$("check-form").addEventListener("submit", (e) => {
  e.preventDefault();
  void check($<HTMLInputElement>("check-mint").value.trim());
});

async function check(address: string) {
  const out = $("check-result");
  out.innerHTML = `<p class="muted">Reading the chain…</p>`;
  try {
    const mint = new PublicKey(address);
    const program = await tokenProgramOf(mint);
    const info: Mint = await getMint(connection, mint, "confirmed", program);
    const vows = await fetchVowsForMint(connection, mint);
    out.innerHTML = renderBadge(info, program, vows);
  } catch (err) {
    message(out, "bad", escape(explainError(err)));
  }
}

function renderBadge(mint: Mint, program: PublicKey, vows: Vow[]): string {
  const now = nowUnix();
  const active = vows.filter((v) => v.releasedAt === 0 && v.unlockAt > now);
  const byCreator = active.filter((v) => v.lockerIsMintAuthority);
  const lockedNow = active.reduce((n, v) => n + v.amount, 0n);
  const creatorLocked = byCreator.reduce((n, v) => n + v.amount, 0n);
  const soonest = active.length ? Math.min(...active.map((v) => v.unlockAt)) : 0;

  const badge = active.length
    ? `<div class="badge ok"><div class="big">${sharePercent(lockedNow, mint.supply)}%</div>
        <div>of supply locked on chain${byCreator.length ? `, ${sharePercent(creatorLocked, mint.supply)}% by the token's creator` : ", none by the token's creator"}.<br/>
        First unlock ${escape(when(soonest))} (${countdown(soonest)}).</div></div>`
    : `<div class="badge none"><div class="big">No vow</div>
        <div>Nothing on this token is locked by the vow program right now.</div></div>`;

  const minting = mint.mintAuthority
    ? `<span class="flag warn">still possible</span> (authority ${short(mint.mintAuthority)}): more tokens can be made, which would shrink every share`
    : `<span class="flag ok">revoked</span>: supply is fixed`;
  const freezing = mint.freezeAuthority
    ? `<span class="flag warn">possible</span> (authority ${short(mint.freezeAuthority)})`
    : `<span class="flag ok">none</span>`;

  const rows = vows
    .map((v) => {
      const status = v.releasedAt ? `released ${escape(when(v.releasedAt))}` : v.unlockAt > now ? `locked · ${countdown(v.unlockAt)}` : "unlockable";
      return `<tr><td><a href="${explorer("address", v.address.toBase58())}" target="_blank" rel="noopener">${short(v.address)}</a></td>
        <td>${formatTokens(v.amount, mint.decimals)} (${sharePercent(v.amount, v.supplyAtLock)}%)</td>
        <td>${escape(when(v.unlockAt))}</td><td>${status}</td>
        <td>${v.lockerIsMintAuthority ? "creator (mint authority)" : "another holder"}</td></tr>`;
    })
    .join("");

  return `${badge}
    <dl class="facts">
      <dt>Supply</dt><dd>${formatTokens(mint.supply, mint.decimals)}</dd>
      <dt>Minting</dt><dd>${minting}</dd>
      <dt>Freezing</dt><dd>${freezing}</dd>
      <dt>Token program</dt><dd>${program.equals(TOKEN_2022_PROGRAM_ID) ? "Token-2022" : "SPL Token"}</dd>
    </dl>
    ${vows.length ? `<div class="table-wrap"><table><thead><tr><th>Vow</th><th>Locked</th><th>Unlocks</th><th>Status</th><th>Locked by</th></tr></thead><tbody>${rows}</tbody></table></div>` : ""}`;
}

// ---------- make a vow ----------

const unlockInput = $<HTMLInputElement>("make-unlock");
function setUnlockMinutes(minutes: number) {
  const d = new Date(Date.now() + minutes * 60_000);
  d.setSeconds(0, 0);
  if (minutes < 5) d.setMinutes(d.getMinutes() + 1); // keep above the program's 1-minute minimum
  const local = new Date(d.getTime() - d.getTimezoneOffset() * 60_000);
  unlockInput.value = local.toISOString().slice(0, 16);
}
document.querySelectorAll<HTMLButtonElement>(".quick button").forEach((b) =>
  b.addEventListener("click", () => setUnlockMinutes(Number(b.dataset.minutes))),
);
setUnlockMinutes(2);

$("make-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const out = $("make-result");
  const btn = $<HTMLButtonElement>("make-submit");
  btn.disabled = true;
  try {
    if (!wallet) throw new Error("Connect Phantom first");
    const mint = new PublicKey($<HTMLInputElement>("make-mint").value.trim());
    const program = await tokenProgramOf(mint);
    const info = await getMint(connection, mint, "confirmed", program);
    const amount = parseTokens($<HTMLInputElement>("make-amount").value, info.decimals);
    const unlockAt = BigInt(Math.floor(new Date(unlockInput.value).getTime() / 1000));
    const nonce = await nextNonce(connection, mint, wallet);
    message(out, "info", "Approve the transaction in Phantom…");
    const sig = await send([
      createVowInstruction({
        locker: wallet,
        mint,
        lockerTokens: getAssociatedTokenAddressSync(mint, wallet, false, program),
        tokenProgram: program,
        nonce,
        amount,
        unlockAt,
      }),
    ]);
    message(out, "ok", `Locked. <a href="${explorer("tx", sig)}" target="_blank" rel="noopener">View transaction</a>`);
    $<HTMLInputElement>("check-mint").value = mint.toBase58();
    await check(mint.toBase58());
    await refreshWallet();
  } catch (err) {
    message(out, "bad", escape(explainError(err, (err as { logs?: string[] }).logs)));
  } finally {
    btn.disabled = false;
  }
});

// ---------- my vows ----------

async function renderMine() {
  const out = $("mine-result");
  if (!wallet) return;
  try {
    const vows = await fetchVowsByLocker(connection, wallet);
    if (!vows.length) {
      out.textContent = "No vows from this wallet on this network yet.";
      return;
    }
    const now = nowUnix();
    out.classList.remove("muted");
    out.innerHTML = `<div class="table-wrap"><table><thead><tr><th>Token</th><th>Amount (base units)</th><th>Unlocks</th><th></th></tr></thead><tbody>${vows
      .map((v, i) => {
        const action = v.releasedAt
          ? `released`
          : v.unlockAt > now
            ? `<button class="secondary" data-try="${i}">Try to withdraw now</button>`
            : `<button data-release="${i}">Release</button>`;
        return `<tr><td><code>${short(v.mint)}</code></td><td>${v.amount.toLocaleString()}</td>
          <td>${escape(when(v.unlockAt))}<br/><span class="muted">${v.releasedAt ? "" : countdown(v.unlockAt)}</span></td><td>${action}</td></tr>`;
      })
      .join("")}</tbody></table></div><div id="mine-msg"></div>`;
    out.querySelectorAll<HTMLButtonElement>("[data-release]").forEach((b) =>
      b.addEventListener("click", () => void release(vows[Number(b.dataset.release)], false)),
    );
    out.querySelectorAll<HTMLButtonElement>("[data-try]").forEach((b) =>
      b.addEventListener("click", () => void release(vows[Number(b.dataset.try)], true)),
    );
  } catch (err) {
    message(out, "bad", escape(explainError(err)));
  }
}

/** Release a vow. With `dryRun`, ask the network to simulate it instead: the demo of the program
 * refusing an early withdrawal, without spending a fee or bothering Phantom. */
async function release(vow: Vow, dryRun: boolean) {
  const out = $("mine-msg");
  try {
    if (!wallet) throw new Error("Connect Phantom first");
    const program = await tokenProgramOf(vow.mint);
    const ix = releaseVowInstruction({
      vow,
      lockerTokens: getAssociatedTokenAddressSync(vow.mint, wallet, false, program),
      tokenProgram: program,
    });
    if (dryRun) {
      const tx = new Transaction().add(ix);
      tx.feePayer = wallet;
      tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;
      const sim = await connection.simulateTransaction(tx);
      if (sim.value.err) {
        message(out, "bad", `Refused by the program: <b>${escape(explainError(sim.value.err, sim.value.logs))}</b>. Unlocks ${escape(when(vow.unlockAt))}.`);
      } else {
        message(out, "ok", "The program would allow it: the vow has unlocked. Press Release.");
      }
      return;
    }
    message(out, "info", "Approve the transaction in Phantom…");
    const sig = await send([ix]);
    message(out, "ok", `Released. <a href="${explorer("tx", sig)}" target="_blank" rel="noopener">View transaction</a>`);
    await refreshWallet();
  } catch (err) {
    message(out, "bad", escape(explainError(err, (err as { logs?: string[] }).logs)));
  }
}

// ---------- demo token ----------

$("launch").addEventListener("click", async () => {
  const out = $("demo-result");
  try {
    if (!wallet) throw new Error("Connect Phantom first");
    // A fresh keypair for the mint account. It only signs this one transaction (creating the
    // account) and is never saved; the token is then controlled by your wallet.
    const mintKp = Keypair.generate();
    const decimals = 6;
    const ata = getAssociatedTokenAddressSync(mintKp.publicKey, wallet, false, TOKEN_PROGRAM_ID);
    const rent = await getMinimumBalanceForRentExemptMint(connection);
    message(out, "info", "Approve the transaction in Phantom…");
    const sig = await send(
      [
        SystemProgram.createAccount({
          fromPubkey: wallet,
          newAccountPubkey: mintKp.publicKey,
          space: MINT_SIZE,
          lamports: rent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeMint2Instruction(mintKp.publicKey, decimals, wallet, null, TOKEN_PROGRAM_ID),
        createAssociatedTokenAccountIdempotentInstruction(wallet, ata, wallet, mintKp.publicKey, TOKEN_PROGRAM_ID),
        createMintToInstruction(mintKp.publicKey, ata, wallet, 1_000_000n * 10n ** BigInt(decimals), [], TOKEN_PROGRAM_ID),
      ],
      [mintKp],
    );
    lastLaunchedMint = mintKp.publicKey;
    $<HTMLButtonElement>("revoke").disabled = false;
    $<HTMLInputElement>("make-mint").value = mintKp.publicKey.toBase58();
    $<HTMLInputElement>("make-amount").value = "100000";
    $<HTMLInputElement>("check-mint").value = mintKp.publicKey.toBase58();
    message(out, "ok", `Launched <code>${mintKp.publicKey.toBase58()}</code> with 1,000,000 supply. <a href="${explorer("tx", sig)}" target="_blank" rel="noopener">View transaction</a>. The vow form is filled in with 10% of it.`);
    await refreshWallet();
  } catch (err) {
    message(out, "bad", escape(explainError(err)));
  }
});

$("revoke").addEventListener("click", async () => {
  const out = $("demo-result");
  try {
    if (!wallet || !lastLaunchedMint) throw new Error("Launch a test token first");
    message(out, "info", "Approve the transaction in Phantom…");
    const sig = await send([
      createSetAuthorityInstruction(lastLaunchedMint, wallet, AuthorityType.MintTokens, null, [], TOKEN_PROGRAM_ID),
    ]);
    message(out, "ok", `Mint authority revoked: supply is now fixed. <a href="${explorer("tx", sig)}" target="_blank" rel="noopener">View transaction</a>`);
    $<HTMLButtonElement>("revoke").disabled = true;
    await check(lastLaunchedMint.toBase58());
  } catch (err) {
    message(out, "bad", escape(explainError(err)));
  }
});

// Keep countdowns fresh.
setInterval(() => {
  const mint = $<HTMLInputElement>("check-mint").value.trim();
  if (mint && $("check-result").querySelector(".badge")) void check(mint);
  if (wallet) void renderMine();
}, 30_000);
