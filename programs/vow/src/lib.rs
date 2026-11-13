//! # Vow: a code-enforced fair-launch promise
//!
//! A token creator locks part of their token's supply in a vault this program controls, until a
//! date they choose. Nobody can take the tokens out early: not the creator, not us. There is no
//! admin key, no upgrade path in the instructions, and no "emergency withdraw".
//!
//! Two instructions:
//! - `create_vow`: move `amount` tokens from the locker's wallet into a vault owned by the vow,
//!   and write the vow record (who, which token, how much, share of supply, unlock time).
//! - `release_vow`: after the unlock time, send the tokens back to the locker and close the vault.
//!   Before the unlock time it fails with `VowStillLocked`. The vow record stays on chain
//!   afterwards (marked released), so the history of kept promises can always be read.
//!
//! Works with classic SPL tokens and Token-2022 (pump.fun's newer launches use Token-2022).
//! Token-2022 mints with extensions that could break the promise are refused (see
//! `extension_allowed`).

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_2022::spl_token_2022::{
        extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions},
        state::Mint as MintState,
    },
    token_interface::{
        close_account, transfer_checked, CloseAccount, Mint, TokenAccount, TokenInterface,
        TransferChecked,
    },
};

// Placeholder program ID. `anchor keys sync` replaces it with the ID of the program keypair that
// `anchor build` generates (kept in target/deploy/, which is gitignored).
declare_id!("3B7936bNMDnFP4no8B9hjqVWrpVwssgH473N3wruUkSD");

/// Shortest lock allowed. Kept at one minute so a live demo can show both the refusal and the
/// release; a real launchpad would ask for days or months.
pub const MIN_LOCK_SECONDS: i64 = 60;
/// Longest lock allowed (about 10 years), so a typo can't lock tokens for a thousand years.
pub const MAX_LOCK_SECONDS: i64 = 10 * 365 * 24 * 60 * 60;

pub const VOW_SEED: &[u8] = b"vow";

#[program]
pub mod vow {
    use super::*;

    /// Lock `amount` tokens (in base units) until `unlock_at` (unix seconds).
    /// `nonce` lets one wallet make several vows on the same token (0, 1, 2, ...).
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
        vow.locker_is_mint_authority = locker_is_mint_authority;
        vow.bump = ctx.bumps.vow;

        emit!(VowCreated {
            vow: vow.key(),
            mint: vow.mint,
            locker: vow.locker,
            amount,
            share_bps: share_bps(amount, supply),
            unlock_at,
            locker_is_mint_authority,
        });
        Ok(())
    }

    /// Return the locked tokens to the locker. Fails before `unlock_at`.
    pub fn release_vow(ctx: Context<ReleaseVow>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let vow = &ctx.accounts.vow;
        check_release(now, vow.unlock_at, vow.released_at)?;

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

        // Send back whatever the vault holds (equal to `amount` unless someone donated extra).
        let balance = ctx.accounts.vault.amount;
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
            balance,
            ctx.accounts.mint.decimals,
        )?;

        // Close the empty vault and give its rent (a few thousandths of a SOL) back to the locker.
        close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            CloseAccount {
                account: ctx.accounts.vault.to_account_info(),
                destination: ctx.accounts.locker.to_account_info(),
                authority: ctx.accounts.vow.to_account_info(),
            },
            signer,
        ))?;

        let vow = &mut ctx.accounts.vow;
        vow.released_at = now;
        emit!(VowReleased {
            vow: vow.key(),
            mint: vow.mint,
            locker: vow.locker,
            amount: balance,
            released_at: now,
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
pub struct ReleaseVow<'info> {
    /// Only the wallet that made the vow can release it.
    #[account(mut)]
    pub locker: Signer<'info>,

    #[account(address = vow.mint, mint::token_program = token_program)]
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
    pub created_at: i64,
    pub unlock_at: i64,
    /// 0 while locked; the release time afterwards.
    pub released_at: i64,
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
    pub locker_is_mint_authority: bool,
}

#[event]
pub struct VowReleased {
    pub vow: Pubkey,
    pub mint: Pubkey,
    pub locker: Pubkey,
    pub amount: u64,
    pub released_at: i64,
}

#[error_code]
pub enum VowError {
    #[msg("Unlock time must be at least one minute from now")]
    LockTooShort,
    #[msg("Unlock time must be within 10 years")]
    LockTooLong,
    #[msg("Amount must be more than zero")]
    ZeroAmount,
    #[msg("This vow is still locked")]
    VowStillLocked,
    #[msg("This vow was already released")]
    AlreadyReleased,
    #[msg("Only the wallet that made the vow can release it")]
    NotLocker,
    #[msg("This token has a Token-2022 extension that could break the lock")]
    UnsafeMintExtension,
}

// ---- Rules as plain functions, so they can be unit-tested without a blockchain ----

pub fn validate_lock(now: i64, unlock_at: i64) -> Result<()> {
    let length = unlock_at.saturating_sub(now);
    require!(length >= MIN_LOCK_SECONDS, VowError::LockTooShort);
    require!(length <= MAX_LOCK_SECONDS, VowError::LockTooLong);
    Ok(())
}

pub fn check_release(now: i64, unlock_at: i64, released_at: i64) -> Result<()> {
    require!(released_at == 0, VowError::AlreadyReleased);
    require!(now >= unlock_at, VowError::VowStillLocked);
    Ok(())
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
/// lock (PermanentDelegate can move tokens out of any account), block the release
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

    #[test]
    fn release_refused_before_unlock() {
        assert!(check_release(NOW, NOW + 1, 0).is_err());
        assert!(check_release(NOW, NOW, 0).is_ok());
        assert!(check_release(NOW + 10, NOW, 0).is_ok());
    }

    #[test]
    fn release_only_once() {
        assert!(check_release(NOW + 10, NOW, NOW + 5).is_err());
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
