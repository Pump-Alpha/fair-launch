//! End-to-end tests of the vow program on LiteSVM, a fast in-memory Solana.
//!
//! The vow unlocks evenly over time; taking out more than has unlocked burns half of the early
//! part. These load the compiled program from `target/deploy/vow.so`, so run `anchor build` first, then
//! `cargo test -p vow` (or `anchor test`). Each test runs for both token programs: classic SPL
//! Token and Token-2022.

use anchor_lang::{InstructionData, ToAccountMetas};
use litesvm::LiteSVM;
use litesvm_token::{
    get_spl_account,
    spl_token::state::{Account as TokenAccountState, Mint as MintState},
    CreateAssociatedTokenAccount, CreateMint, MintTo,
};
use solana_clock::Clock;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;
use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::Instruction;

const TOKEN: Pubkey = anchor_spl::token::ID;
const TOKEN_2022: Pubkey = anchor_spl::token_2022::ID;
const SUPPLY: u64 = 1_000_000_000_000; // 1,000,000 tokens at 6 decimals
const LOCKED: u64 = 100_000_000_000; // 10% of supply

struct Setup {
    svm: LiteSVM,
    creator: Keypair,
    mint: Pubkey,
    creator_tokens: Pubkey,
    token_program: Pubkey,
}

fn setup(token_program: Pubkey) -> Setup {
    let mut svm = LiteSVM::new();
    let so = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/deploy/vow.so");
    assert!(
        std::path::Path::new(so).exists(),
        "target/deploy/vow.so not found: run `anchor build` before `cargo test`"
    );
    svm.add_program_from_file(vow::ID, so).unwrap();
    // LiteSVM's clock starts at 0. Start at a realistic time (Jan 2027) so times read like real ones.
    warp_to(&mut svm, 1_800_000_000);

    let creator = Keypair::new();
    svm.airdrop(&creator.pubkey(), 10_000_000_000).unwrap();
    let mint = CreateMint::new(&mut svm, &creator)
        .decimals(6)
        .token_program_id(&token_program)
        .send()
        .unwrap();
    let creator_tokens = CreateAssociatedTokenAccount::new(&mut svm, &creator, &mint)
        .token_program_id(&token_program)
        .send()
        .unwrap();
    MintTo::new(&mut svm, &creator, &mint, &creator_tokens, SUPPLY)
        .token_program_id(&token_program)
        .send()
        .unwrap();
    Setup { svm, creator, mint, creator_tokens, token_program }
}

fn now(svm: &LiteSVM) -> i64 {
    svm.get_sysvar::<Clock>().unix_timestamp
}

fn warp_to(svm: &mut LiteSVM, unix: i64) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp = unix;
    svm.set_sysvar(&clock);
    svm.expire_blockhash();
}

fn vow_address(mint: &Pubkey, locker: &Pubkey, nonce: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[vow::VOW_SEED, mint.as_ref(), locker.as_ref(), &nonce.to_le_bytes()],
        &vow::ID,
    )
    .0
}

fn ata(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &anchor_spl::associated_token::ID,
    )
    .0
}

fn send(ix: Instruction, svm: &mut LiteSVM, signer: &Keypair) -> Result<(), String> {
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&signer.pubkey()),
        &[signer],
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx)
        .map(|_| ())
        .map_err(|e| format!("{:?}\n{}", e.err, e.meta.logs.join("\n")))
}

fn create_ix(s: &Setup, locker: &Pubkey, locker_tokens: Pubkey, nonce: u64, amount: u64, unlock_at: i64) -> Instruction {
    let vow = vow_address(&s.mint, locker, nonce);
    Instruction {
        program_id: vow::ID,
        accounts: vow::accounts::CreateVow {
            locker: *locker,
            mint: s.mint,
            locker_tokens,
            vow,
            vault: ata(&vow, &s.mint, &s.token_program),
            token_program: s.token_program,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: vow::instruction::CreateVow { nonce, amount, unlock_at }.data(),
    }
}

fn withdraw_ix(s: &Setup, locker: &Pubkey, locker_tokens: Pubkey, vow: Pubkey, amount: u64, max_burn: u64) -> Instruction {
    Instruction {
        program_id: vow::ID,
        accounts: vow::accounts::Withdraw {
            locker: *locker,
            mint: s.mint,
            locker_tokens,
            vow,
            vault: ata(&vow, &s.mint, &s.token_program),
            token_program: s.token_program,
        }
        .to_account_metas(None),
        data: vow::instruction::Withdraw { amount, max_burn }.data(),
    }
}

fn balance(svm: &LiteSVM, account: &Pubkey) -> u64 {
    get_spl_account::<TokenAccountState>(svm, account).unwrap().amount
}

fn supply(svm: &LiteSVM, mint: &Pubkey) -> u64 {
    get_spl_account::<MintState>(svm, mint).unwrap().supply
}

fn read_vow(svm: &LiteSVM, address: &Pubkey) -> vow::Vow {
    use anchor_lang::AccountDeserialize;
    let account = svm.get_account(address).expect("vow account exists");
    vow::Vow::try_deserialize(&mut account.data.as_slice()).unwrap()
}

fn full_life_cycle(token_program: Pubkey) {
    let mut s = setup(token_program);
    let creator = s.creator.insecure_clone();
    let me = creator.pubkey();
    let start = now(&s.svm);
    let unlock_at = start + 1_000; // vests evenly over 1,000 seconds

    send(create_ix(&s, &me, s.creator_tokens, 0, LOCKED, unlock_at), &mut s.svm, &creator)
        .expect("create_vow");

    let vow = vow_address(&s.mint, &me, 0);
    let vault = ata(&vow, &s.mint, &s.token_program);
    assert_eq!(balance(&s.svm, &vault), LOCKED);
    assert_eq!(balance(&s.svm, &s.creator_tokens), SUPPLY - LOCKED);

    let record = read_vow(&s.svm, &vow);
    assert_eq!(record.mint, s.mint);
    assert_eq!(record.locker, me);
    assert_eq!(record.amount, LOCKED);
    assert_eq!(record.supply_at_lock, SUPPLY);
    assert_eq!(record.created_at, start);
    assert_eq!(record.unlock_at, unlock_at);
    assert_eq!(record.released_at, 0);
    assert_eq!(record.penalty_bps, vow::EARLY_EXIT_PENALTY_BPS);
    assert!(record.locker_is_mint_authority, "creator made the mint, so is its authority");

    // At 20%, the unlocked fifth comes out whole.
    let fifth = LOCKED / 5;
    warp_to(&mut s.svm, start + 200);
    send(withdraw_ix(&s, &me, s.creator_tokens, vow, fifth, 0), &mut s.svm, &creator)
        .expect("withdraw the unlocked part");
    assert_eq!(balance(&s.svm, &s.creator_tokens), SUPPLY - LOCKED + fifth);
    assert_eq!(supply(&s.svm, &s.mint), SUPPLY, "nothing burned");

    // Asking for more without agreeing to a burn is refused, and nothing moves.
    s.svm.expire_blockhash(); // same instruction as above: a new blockhash keeps it a new transaction
    let err = send(withdraw_ix(&s, &me, s.creator_tokens, vow, fifth, 0), &mut s.svm, &creator)
        .expect_err("early withdrawal without max_burn must fail");
    assert!(err.contains("BurnAboveLimit"), "{err}");
    assert_eq!(balance(&s.svm, &vault), LOCKED - fifth);

    // Agreeing to the burn: still at 20%, so this fifth is all early and half of it burns.
    send(withdraw_ix(&s, &me, s.creator_tokens, vow, fifth, fifth / 2), &mut s.svm, &creator)
        .expect("early withdrawal");
    assert_eq!(balance(&s.svm, &s.creator_tokens), SUPPLY - LOCKED + fifth + fifth / 2);
    assert_eq!(supply(&s.svm, &s.mint), SUPPLY - fifth / 2, "half the early part burned");
    assert_eq!(balance(&s.svm, &vault), LOCKED - 2 * fifth);
    let record = read_vow(&s.svm, &vow);
    assert_eq!(record.withdrawn, fifth + fifth / 2);
    assert_eq!(record.burned, fifth / 2);

    // At the end the rest comes out whole, the vault closes and the record stays.
    warp_to(&mut s.svm, unlock_at);
    let rest = LOCKED - 2 * fifth;
    send(withdraw_ix(&s, &me, s.creator_tokens, vow, rest, 0), &mut s.svm, &creator)
        .expect("withdraw the rest");
    assert_eq!(balance(&s.svm, &s.creator_tokens), SUPPLY - fifth / 2);
    assert!(s.svm.get_account(&vault).map_or(true, |a| a.lamports == 0), "vault closed");
    let record = read_vow(&s.svm, &vow);
    assert_eq!(record.released_at, unlock_at);
    assert_eq!(record.withdrawn + record.burned, LOCKED);
}

#[test]
fn full_life_cycle_spl_token() {
    full_life_cycle(TOKEN);
}

#[test]
fn full_life_cycle_token_2022() {
    full_life_cycle(TOKEN_2022);
}

#[test]
fn lock_shorter_than_a_minute_is_refused() {
    let mut s = setup(TOKEN_2022);
    let creator = s.creator.insecure_clone();
    let unlock_at = now(&s.svm) + 30;
    let err = send(create_ix(&s, &creator.pubkey(), s.creator_tokens, 0, LOCKED, unlock_at), &mut s.svm, &creator)
        .expect_err("30-second lock must fail");
    assert!(err.contains("LockTooShort"), "{err}");
}

#[test]
fn dumping_everything_at_once_burns_half() {
    let mut s = setup(TOKEN_2022);
    let creator = s.creator.insecure_clone();
    let me = creator.pubkey();
    let unlock_at = now(&s.svm) + 30 * 24 * 60 * 60;
    send(create_ix(&s, &me, s.creator_tokens, 0, LOCKED, unlock_at), &mut s.svm, &creator)
        .expect("create_vow");
    let vow = vow_address(&s.mint, &me, 0);
    // Right away (same second): nothing has unlocked, so all of it is early.
    send(withdraw_ix(&s, &me, s.creator_tokens, vow, LOCKED, LOCKED), &mut s.svm, &creator)
        .expect("dump");
    assert_eq!(balance(&s.svm, &s.creator_tokens), SUPPLY - LOCKED / 2);
    assert_eq!(supply(&s.svm, &s.mint), SUPPLY - LOCKED / 2);
    assert!(read_vow(&s.svm, &vow).released_at > 0);
}

#[test]
fn someone_else_cannot_withdraw() {
    let mut s = setup(TOKEN);
    let creator = s.creator.insecure_clone();
    let unlock_at = now(&s.svm) + 120;
    send(create_ix(&s, &creator.pubkey(), s.creator_tokens, 0, LOCKED, unlock_at), &mut s.svm, &creator)
        .expect("create_vow");
    let vow = vow_address(&s.mint, &creator.pubkey(), 0);
    warp_to(&mut s.svm, unlock_at + 10);

    // A stranger with their own token account for this mint tries to take the tokens.
    let stranger = Keypair::new();
    s.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let mint = s.mint;
    let stranger_tokens = CreateAssociatedTokenAccount::new(&mut s.svm, &stranger, &mint)
        .token_program_id(&TOKEN)
        .send()
        .unwrap();
    let err = send(withdraw_ix(&s, &stranger.pubkey(), stranger_tokens, vow, LOCKED, LOCKED), &mut s.svm, &stranger)
        .expect_err("stranger must not withdraw");
    assert!(err.contains("NotLocker") || err.contains("ConstraintHasOne"), "{err}");
    assert_eq!(balance(&s.svm, &ata(&vow, &s.mint, &TOKEN)), LOCKED);
}

#[test]
fn a_holder_who_is_not_the_mint_authority_is_marked_so() {
    let mut s = setup(TOKEN);
    let creator = s.creator.insecure_clone();
    // The creator gives a holder some tokens; the holder makes a vow.
    let holder = Keypair::new();
    s.svm.airdrop(&holder.pubkey(), 1_000_000_000).unwrap();
    let mint = s.mint;
    let holder_tokens = CreateAssociatedTokenAccount::new(&mut s.svm, &holder, &mint)
        .owner(&holder.pubkey())
        .token_program_id(&TOKEN)
        .send()
        .unwrap();
    MintTo::new(&mut s.svm, &creator, &mint, &holder_tokens, 5_000_000)
        .token_program_id(&TOKEN)
        .send()
        .unwrap();
    let unlock_at = now(&s.svm) + 600;
    send(create_ix(&s, &holder.pubkey(), holder_tokens, 0, 5_000_000, unlock_at), &mut s.svm, &holder)
        .expect("create_vow");
    let record = read_vow(&s.svm, &vow_address(&s.mint, &holder.pubkey(), 0));
    assert!(!record.locker_is_mint_authority);
}

#[test]
fn nothing_more_after_everything_is_out() {
    let mut s = setup(TOKEN_2022);
    let creator = s.creator.insecure_clone();
    let unlock_at = now(&s.svm) + 60;
    send(create_ix(&s, &creator.pubkey(), s.creator_tokens, 7, LOCKED, unlock_at), &mut s.svm, &creator)
        .expect("create_vow");
    let vow = vow_address(&s.mint, &creator.pubkey(), 7);
    warp_to(&mut s.svm, unlock_at);
    send(withdraw_ix(&s, &creator.pubkey(), s.creator_tokens, vow, LOCKED, 0), &mut s.svm, &creator)
        .expect("withdraw everything");
    warp_to(&mut s.svm, unlock_at + 5);
    assert!(send(withdraw_ix(&s, &creator.pubkey(), s.creator_tokens, vow, 1, 0), &mut s.svm, &creator).is_err());
}
