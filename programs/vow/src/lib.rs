//! # Vow: a code-enforced fair-launch promise
//!
//! A token creator puts part of their token's supply in a vault this program controls. The
//! tokens unlock gradually (evenly from the moment of the vow until a date the creator chooses),
//! so the creator can keep selling at a steady pace and the market keeps its liquidity. What the
//! vow stops is the dump: taking out more than has unlocked so far is allowed, but the program
//! burns half of the early part (`EARLY_EXIT_PENALTY_BPS`). The burn is the punishment, and it
//! shrinks the supply, so everyone else's tokens become a bigger share. There is no admin key and
//! no way around the rule, for anyone, including us.
//!
//! Two instructions:
//! - `create_vow`: move `amount` tokens from the locker's wallet into a vault owned by the vow,
//!   and write the vow record (who, which token, how much, share of supply, vesting end).
//! - `withdraw`: take tokens out. What has unlocked comes out whole; anything beyond that is
//!   "early" and half of it is burned. `max_burn` is the most the caller agrees to have burned,
//!   so nobody burns tokens by accident (pass 0 to take only what has unlocked). When the vault is
//!   empty it is closed; the vow record stays on chain, so the history can always be read.
//!
//! Works with classic SPL tokens and Token-2022. Token-2022 mints with extensions that could
//! break the promise are refused (see `extension_allowed`).

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_2022::spl_token_2022::{
        extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions},
        state::Mint as MintState,
    },
    token_interface::{
        burn, close_account, transfer_checked, Burn, CloseAccount, Mint, TokenAccount,
        TokenInterface, TransferChecked,
    },
};

// The ID of the program keypair `anchor build` generated on the owner's Mac (kept in
// target/deploy/, which is gitignored). `anchor keys sync` writes it here.
declare_id!("3B7936bNMDnFP4no8B9hjqVWrpVwssgH473N3wruUkSD");

/// Shortest vesting period allowed. Kept at one minute so a live demo can show tokens unlocking;
/// a real launchpad would ask for weeks or months.
pub const MIN_LOCK_SECONDS: i64 = 60;
/// Longest vesting period allowed (about 10 years), so a typo can't lock tokens for a thousand years.
pub const MAX_LOCK_SECONDS: i64 = 10 * 365 * 24 * 60 * 60;

/// Share of an early withdrawal that is burned, in basis points (5,000 = 50%). Stored in each vow
/// so a later version could let creators pick it without changing what old vows mean.
pub const EARLY_EXIT_PENALTY_BPS: u16 = 5_000;

pub const VOW_SEED: &[u8] = b"vow";

#[program]
pub mod vow {
    use super::*;

    /// Vow `amount` tokens (in base units), unlocking evenly from now until `unlock_at` (unix
    /// seconds). `nonce` lets one wallet make several vows on the same token (0, 1, 2, ...).
    pub fn create_vow(
        ctx: Context<CreateVow>,
        nonce: u64,
        amount: u64,
        unlock_at: i64,
    ) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        validate_lock(now, unlock_at)?;
        require!(amount > 0, VowError::ZeroAmount);

        // Refuse mints whose extensions could undo the lock (e.g. a permanent delegate can move
        // tokens out of any account, our vault included).
        check_mint_extensions(&ctx.accounts.mint.to_account_info())?;

        let mint = &ctx.accounts.mint;
        let supply = mint.supply;
        // Is the locker the token's mint authority? That's the closest thing to "the creator" a
        // generic token has; the badge shows it so a stranger's vow can't pose as the creator's.
        let locker_is_mint_authority =
            Option::<Pubkey>::from(mint.mint_authority) == Some(ctx.accounts.locker.key());

        transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                TransferChecked {
                    from: ctx.accounts.locker_tokens.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                    authority: ctx.accounts.locker.to_account_info(),
                },
            ),
            amount,
            mint.decimals,
        )?;

        let vow = &mut ctx.accounts.vow;
        vow.mint = mint.key();
        vow.locker = ctx.accounts.locker.key();
        vow.nonce = nonce;
        vow.amount = amount;
        vow.supply_at_lock = supply;
        vow.created_at = now;
        vow.unlock_at = unlock_at;
        vow.released_at = 0;
        vow.withdrawn = 0;
        vow.burned = 0;
        vow.penalty_bps = EARLY_EXIT_PENALTY_BPS;
        vow.locker_is_mint_authority = locker_is_mint_authority;
        vow.bump = ctx.bumps.vow;

        emit!(VowCreated {
            vow: vow.key(),
            mint: vow.mint,
            locker: vow.locker,
            amount,
            share_bps: share_bps(amount, supply),
            unlock_at,
            penalty_bps: EARLY_EXIT_PENALTY_BPS,
            locker_is_mint_authority,
        });
        Ok(())
    }

    /// Take `amount` tokens out of the vault. The unlocked part goes to the locker whole; the
    /// early part (beyond what has unlocked) is half burned. Fails if the burn would be more than
    /// `max_burn`, so a caller only burns what they agreed to.
    pub fn withdraw(ctx: Context<Withdraw>, amount: u64, max_burn: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let vow = &ctx.accounts.vow;
        let split = plan_withdraw(vow, now, amount)?;
        require!(split.burn <= max_burn, VowError::BurnAboveLimit);

        // The vow account owns the vault, so the program signs for it with the vow's seeds.
        let mint_key = vow.mint;
        let locker_key = vow.locker;
        let nonce_bytes = vow.nonce.to_le_bytes();
        let seeds: &[&[u8]] = &[
            VOW_SEED,
            mint_key.as_ref(),
            locker_key.as_ref(),
            &nonce_bytes,
            &[vow.bump],
        ];
        let signer = &[seeds];

        // On the last withdrawal, also hand back anything someone sent to the vault on top of the
        // vow (it was never promised, so it isn't burned), so the vault can be closed empty.
        let last = split.remaining_after == 0;
        let still_promised = vow.amount - vow.withdrawn - vow.burned;
        let extra = if last { ctx.accounts.vault.amount.saturating_sub(still_promised) } else { 0 };

        if split.burn > 0 {
            burn(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.key(),
                    Burn {
                        mint: ctx.accounts.mint.to_account_info(),
                        from: ctx.accounts.vault.to_account_info(),
                        authority: ctx.accounts.vow.to_account_info(),
                    },
                    signer,
                ),
                split.burn,
            )?;
        }
        let to_locker = split.to_locker + extra;
        if to_locker > 0 {
            transfer_checked(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.key(),
                    TransferChecked {
                        from: ctx.accounts.vault.to_account_info(),
                        mint: ctx.accounts.mint.to_account_info(),
                        to: ctx.accounts.locker_tokens.to_account_info(),
                        authority: ctx.accounts.vow.to_account_info(),
                    },
                    signer,
                ),
                to_locker,
                ctx.accounts.mint.decimals,
            )?;
        }
        if last {
            // Close the empty vault and give its rent (a few thousandths of a SOL) to the locker.
            close_account(CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                CloseAccount {
                    account: ctx.accounts.vault.to_account_info(),
                    destination: ctx.accounts.locker.to_account_info(),
                    authority: ctx.accounts.vow.to_account_info(),
                },
                signer,
            ))?;
        }

        let vow = &mut ctx.accounts.vow;
        vow.withdrawn += split.to_locker;
        vow.burned += split.burn;
        if last {
            vow.released_at = now;
        }
        emit!(VowWithdrawn {
            vow: vow.key(),
            mint: vow.mint,
            locker: vow.locker,
            to_locker: split.to_locker,
            burned: split.burn,
            early: split.early,
            remaining: split.remaining_after,
            at: now,
        });
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(nonce: u64)]
pub struct CreateVow<'info> {
    #[account(mut)]
    pub locker: Signer<'info>,

    #[account(mint::token_program = token_program)]
    pub mint: InterfaceAccount<'info, Mint>,

    /// The locker's own token account the tokens come from.
    #[account(
        mut,
        token::mint = mint,
        token::authority = locker,
        token::token_program = token_program,
    )]
    pub locker_tokens: InterfaceAccount<'info, TokenAccount>,

    /// The vow record. Its address is derived from (mint, locker, nonce), so anyone can find it.
    #[account(
        init,
        payer = locker,
        space = 8 + Vow::INIT_SPACE,
        seeds = [VOW_SEED, mint.key().as_ref(), locker.key().as_ref(), &nonce.to_le_bytes()],
        bump,
    )]
    pub vow: Account<'info, Vow>,

    /// Where the locked tokens sit: the vow's own associated token account.
    #[account(
        init,
        payer = locker,
        associated_token::mint = mint,
        associated_token::authority = vow,
        associated_token::token_program = token_program,
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    /// Only the wallet that made the vow can withdraw from it.
    #[account(mut)]
    pub locker: Signer<'info>,

    /// Writable because burning lowers the mint's supply.
    #[account(mut, address = vow.mint, mint::token_program = token_program)]
    pub mint: InterfaceAccount<'info, Mint>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = locker,
        token::token_program = token_program,
    )]
    pub locker_tokens: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        has_one = locker @ VowError::NotLocker,
        has_one = mint,
        seeds = [VOW_SEED, vow.mint.as_ref(), vow.locker.as_ref(), &vow.nonce.to_le_bytes()],
        bump = vow.bump,
    )]
    pub vow: Account<'info, Vow>,

    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = vow,
        associated_token::token_program = token_program,
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// One vow. Field order matters for lookups: `mint` sits right after Anchor's 8-byte account
/// discriminator (offset 8) and `locker` after it (offset 40), so the badge page can ask an RPC
/// node for "every vow on this token" with a single byte filter.
#[account]
#[derive(InitSpace)]
pub struct Vow {
    pub mint: Pubkey,
    pub locker: Pubkey,
    pub nonce: u64,
    /// Tokens locked, in base units.
    pub amount: u64,
    /// The token's total supply when the vow was made (for "share of supply").
    pub supply_at_lock: u64,
    /// Vesting runs evenly from `created_at` to `unlock_at`.
    pub created_at: i64,
    pub unlock_at: i64,
    /// 0 while tokens remain in the vault; the time of the last withdrawal afterwards. To tell
    /// whether a vow is finished, compare `withdrawn + burned` with `amount`.
    pub released_at: i64,
    /// Tokens sent back to the locker so far.
    pub withdrawn: u64,
    /// Tokens burned so far as the early-exit penalty.
    pub burned: u64,
    /// Share of early withdrawals burned, in basis points.
    pub penalty_bps: u16,
    /// Was the locker the token's mint authority when they made the vow?
    pub locker_is_mint_authority: bool,
    pub bump: u8,
}

#[event]
pub struct VowCreated {
    pub vow: Pubkey,
    pub mint: Pubkey,
    pub locker: Pubkey,
    pub amount: u64,
    pub share_bps: u16,
    pub unlock_at: i64,
    pub penalty_bps: u16,
    pub locker_is_mint_authority: bool,
}

#[event]
pub struct VowWithdrawn {
    pub vow: Pubkey,
    pub mint: Pubkey,
    pub locker: Pubkey,
    /// Tokens sent to the locker (not counting any extra someone sent to the vault).
    pub to_locker: u64,
    pub burned: u64,
    /// How much of this withdrawal was beyond what had unlocked.
    pub early: u64,
    /// Tokens still promised and in the vault afterwards.
    pub remaining: u64,
    pub at: i64,
}

#[error_code]
pub enum VowError {
    #[msg("Full unlock must be at least one minute from now")]
    LockTooShort,
    #[msg("Full unlock must be within 10 years")]
    LockTooLong,
    #[msg("Amount must be more than zero")]
    ZeroAmount,
    #[msg("Everything in this vow has already been taken out")]
    AlreadyReleased,
    #[msg("Only the wallet that made the vow can withdraw from it")]
    NotLocker,
    #[msg("This token has a Token-2022 extension that could break the lock")]
    UnsafeMintExtension,
    #[msg("That is more than is left in this vow")]
    MoreThanLeft,
    #[msg("This withdrawal would burn more than you agreed to")]
    BurnAboveLimit,
}

// ---- Rules as plain functions, so they can be unit-tested without a blockchain ----

pub fn validate_lock(now: i64, unlock_at: i64) -> Result<()> {
    let length = unlock_at.saturating_sub(now);
    require!(length >= MIN_LOCK_SECONDS, VowError::LockTooShort);
    require!(length <= MAX_LOCK_SECONDS, VowError::LockTooLong);
    Ok(())
}

/// Tokens unlocked by `now`: none at `created_at`, all at `unlock_at`, evenly in between
/// (rounded down, so the locker never gets ahead of the schedule).
pub fn vested(amount: u64, created_at: i64, unlock_at: i64, now: i64) -> u64 {
    if now >= unlock_at {
        return amount;
    }
    if now <= created_at {
        return 0;
    }
    let elapsed = (now - created_at) as u128;
    let length = (unlock_at - created_at) as u128;
    ((amount as u128) * elapsed / length) as u64
}

/// How one withdrawal splits.
#[derive(Debug, PartialEq, Eq)]
pub struct Split {
    /// Sent to the locker: the unlocked part plus the unburned half of the early part.
    pub to_locker: u64,
    /// Burned: the penalty share of the early part, rounded up.
    pub burn: u64,
    /// The part of the request beyond what had unlocked.
    pub early: u64,
    /// Tokens left in the vow after this withdrawal.
    pub remaining_after: u64,
}

/// The withdrawal rule. Unlocked tokens not yet taken come out first and whole. The rest of the
/// request is early, and `penalty_bps` of it is burned. Early withdrawals use up future unlocks:
/// what counts as "taken" includes what was burned, so nothing new is free until the schedule
/// catches up with everything taken so far.
pub fn plan_withdraw(vow: &Vow, now: i64, amount: u64) -> Result<Split> {
    // "All out" is judged by the counts, not by `released_at`, which would be 0 if the clock
    // ever read 0 (it does in tests).
    let taken = vow.withdrawn + vow.burned;
    let left = vow.amount - taken;
    require!(left > 0, VowError::AlreadyReleased);
    require!(amount > 0, VowError::ZeroAmount);
    require!(amount <= left, VowError::MoreThanLeft);

    let free = vested(vow.amount, vow.created_at, vow.unlock_at, now).saturating_sub(taken);
    let unlocked_part = amount.min(free);
    let early = amount - unlocked_part;
    let burn = ((early as u128 * vow.penalty_bps as u128 + 9_999) / 10_000) as u64;
    Ok(Split {
        to_locker: amount - burn,
        burn,
        early,
        remaining_after: left - amount,
    })
}

/// Share of supply in basis points (1 bp = 0.01%), rounded down; 0 if supply is 0.
pub fn share_bps(amount: u64, supply: u64) -> u16 {
    if supply == 0 {
        return 0;
    }
    let bps = (amount as u128) * 10_000 / (supply as u128);
    bps.min(10_000) as u16
}

/// Token-2022 mint extensions we accept. Everything else is refused, because it could undo the
/// lock (PermanentDelegate can move tokens out of any account), block a withdrawal
/// (TransferHook, DefaultAccountState frozen, Pausable), or make the vault hold less than was
/// promised (TransferFeeConfig). These are descriptive only: name, symbol, image, groups.
pub fn extension_allowed(ext: ExtensionType) -> bool {
    matches!(
        ext,
        ExtensionType::MetadataPointer
            | ExtensionType::TokenMetadata
            | ExtensionType::GroupPointer
            | ExtensionType::GroupMemberPointer
            | ExtensionType::TokenGroup
            | ExtensionType::TokenGroupMember
            | ExtensionType::MintCloseAuthority
    )
}

fn check_mint_extensions(mint: &AccountInfo) -> Result<()> {
    let data = mint.try_borrow_data()?;
    let state = StateWithExtensions::<MintState>::unpack(&data)?;
    for ext in state.get_extension_types()? {
        require!(extension_allowed(ext), VowError::UnsafeMintExtension);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn lock_must_be_at_least_a_minute() {
        assert!(validate_lock(NOW, NOW + 59).is_err());
        assert!(validate_lock(NOW, NOW).is_err());
        assert!(validate_lock(NOW, NOW - 100).is_err());
        assert!(validate_lock(NOW, NOW + 60).is_ok());
    }

    #[test]
    fn lock_must_be_within_ten_years() {
        assert!(validate_lock(NOW, NOW + MAX_LOCK_SECONDS).is_ok());
        assert!(validate_lock(NOW, NOW + MAX_LOCK_SECONDS + 1).is_err());
        assert!(validate_lock(NOW, i64::MAX).is_err());
    }

    fn vow(amount: u64) -> Vow {
        Vow {
            mint: Pubkey::default(),
            locker: Pubkey::default(),
            nonce: 0,
            amount,
            supply_at_lock: amount * 10,
            created_at: NOW,
            unlock_at: NOW + 1_000,
            released_at: 0,
            withdrawn: 0,
            burned: 0,
            penalty_bps: EARLY_EXIT_PENALTY_BPS,
            locker_is_mint_authority: true,
            bump: 255,
        }
    }

    fn split(to_locker: u64, burn: u64, early: u64, remaining_after: u64) -> Split {
        Split { to_locker, burn, early, remaining_after }
    }

    #[test]
    fn vesting_is_even_between_the_dates() {
        assert_eq!(vested(1_000, NOW, NOW + 1_000, NOW - 5), 0);
        assert_eq!(vested(1_000, NOW, NOW + 1_000, NOW), 0);
        assert_eq!(vested(1_000, NOW, NOW + 1_000, NOW + 250), 250);
        assert_eq!(vested(1_000, NOW, NOW + 1_000, NOW + 999), 999);
        assert_eq!(vested(1_000, NOW, NOW + 1_000, NOW + 5_000), 1_000);
        assert_eq!(vested(10, NOW, NOW + 3, NOW + 1), 3); // rounded down
        assert_eq!(vested(u64::MAX, NOW, NOW + MAX_LOCK_SECONDS, NOW + 1), u64::MAX / MAX_LOCK_SECONDS as u64);
    }

    #[test]
    fn unlocked_tokens_come_out_whole() {
        let v = vow(1_000);
        assert_eq!(plan_withdraw(&v, NOW + 300, 300).unwrap(), split(300, 0, 0, 700));
        assert_eq!(plan_withdraw(&v, NOW + 300, 100).unwrap(), split(100, 0, 0, 900));
    }

    #[test]
    fn early_part_is_half_burned() {
        let v = vow(1_000);
        // 300 unlocked, asking for 500: 200 early, 100 burned, 400 to the locker.
        assert_eq!(plan_withdraw(&v, NOW + 300, 500).unwrap(), split(400, 100, 200, 500));
        // Everything at the very start: all early, half burned.
        assert_eq!(plan_withdraw(&v, NOW, 1_000).unwrap(), split(500, 500, 1_000, 0));
        // An odd early amount rounds the burn up.
        assert_eq!(plan_withdraw(&v, NOW, 3).unwrap(), split(1, 2, 3, 997));
    }

    #[test]
    fn early_exit_uses_up_future_unlocks() {
        let mut v = vow(1_000);
        // At 30%, the locker took 500 (300 free + 200 early, 100 of it burned): 500 counts as
        // taken, as if they had waited until 50%.
        v.withdrawn = 400;
        v.burned = 100;
        // At 50% nothing new is free yet, so 100 more is all early.
        assert_eq!(plan_withdraw(&v, NOW + 500, 100).unwrap(), split(50, 50, 100, 400));
        // At 70%, 200 is free again.
        assert_eq!(plan_withdraw(&v, NOW + 700, 200).unwrap(), split(200, 0, 0, 300));
        // At the end, everything left is free.
        assert_eq!(plan_withdraw(&v, NOW + 1_000, 500).unwrap(), split(500, 0, 0, 0));
    }

    #[test]
    fn cannot_take_more_than_is_left_or_nothing() {
        let mut v = vow(1_000);
        assert!(plan_withdraw(&v, NOW + 2_000, 1_001).is_err());
        assert!(plan_withdraw(&v, NOW + 2_000, 0).is_err());
        v.withdrawn = 1_000;
        v.released_at = NOW + 2_000;
        assert!(plan_withdraw(&v, NOW + 3_000, 1).is_err());
    }

    #[test]
    fn share_of_supply() {
        assert_eq!(share_bps(100, 1_000), 1_000); // 10%
        assert_eq!(share_bps(1, 3), 3_333); // 33.33%, rounded down
        assert_eq!(share_bps(5, 0), 0);
        assert_eq!(share_bps(u64::MAX, u64::MAX), 10_000);
        assert_eq!(share_bps(u64::MAX, 1), 10_000); // can't show more than 100%
    }

    #[test]
    fn dangerous_extensions_refused() {
        assert!(!extension_allowed(ExtensionType::PermanentDelegate));
        assert!(!extension_allowed(ExtensionType::TransferHook));
        assert!(!extension_allowed(ExtensionType::TransferFeeConfig));
        assert!(!extension_allowed(ExtensionType::DefaultAccountState));
        assert!(!extension_allowed(ExtensionType::NonTransferable));
        assert!(extension_allowed(ExtensionType::MetadataPointer));
        assert!(extension_allowed(ExtensionType::TokenMetadata));
    }
}
