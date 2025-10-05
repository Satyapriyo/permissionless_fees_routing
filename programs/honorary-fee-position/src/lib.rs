use anchor_lang::prelude::*;
use anchor_spl::token::{self, TokenAccount, Token, Transfer};
//use anchor_spl::associated_token::AssociatedToken;
//use std::convert::TryInto;


declare_id!("Y6S8ztXqBsRsj9husmE2PmLm3cLqbfwbmf1o1KNFsNk");


/*
 - Honorary Fee Position program
    - initialize_honorary_position
    - crank_distribute (paginated, idempotent, 24h gate)
    - local-testing feature includes cp_amm_stub & streamflow_stub to run tests offline
*/

const DAY_SECONDS: i64 = 86_400;
const PAGE_RECORD_CAP: usize = 50;

#[program]
pub mod honorary_fee_position {
    use super::*;

    /// Initialize policy & progress PDAs and treasury ATA.
    /// Note: This instruction does not itself create a DAMM position via cp-amm CPI.
    pub fn initialize_honorary_position(
        ctx: Context<InitializeHonoraryPosition>,
        _bump_owner: u8,
        y0: u128,
        investor_fee_share_bps: u16,
        daily_cap: Option<u64>,
        min_payout: u64,
        dust_threshold: u64,
    ) -> Result<()> {
        // Basic sanity checks
        require!(investor_fee_share_bps <= 10_000, ErrorCode::InvalidBps);
        require!(ctx.accounts.pool_quote_mint.key() != ctx.accounts.pool_base_mint.key(), ErrorCode::PoolTokenOrderMismatch);

        // Set Policy
        let policy = &mut ctx.accounts.policy;
        policy.vault = ctx.accounts.vault.key();
        policy.y0 = y0;
        policy.investor_fee_share_bps = investor_fee_share_bps;
        policy.daily_cap = daily_cap;
        policy.min_payout = min_payout;
        policy.dust_threshold = dust_threshold;

        // Initialize progress
        let progress = &mut ctx.accounts.progress;
        progress.vault = ctx.accounts.vault.key();
        progress.day_start_ts = 0;
        progress.last_distribution_ts = 0;
        progress.cumulative_distributed_today = 0;
        progress.carry_lamports = 0;
        progress.cursor = 0;
        progress.treasury_snapshot = ctx.accounts.program_quote_treasury.amount;
        progress.page_records = Vec::new();

        emit!(HonoraryPositionInitialized {
            pool: ctx.accounts.pool.key(),
            position: ctx.accounts.honorary_position.key(),
            quote_mint: ctx.accounts.pool_quote_mint.key(),
        });

        Ok(())
    }

    /// Permissionless crank — paginated distribution.
    /// remaining_accounts: pairs [stream_acc_0, investor_ata_0, stream_acc_1, investor_ata_1, ...]
    pub fn crank_distribute<'info>(
        ctx:  Context<'_, '_, 'info, 'info, CrankDistribute<'info>>,
        investor_fee_pos_owner_bump: u8,
        page_index: u64,
        is_last_page: bool,
    ) -> Result<()> {
        let now_ts = Clock::get()?.unix_timestamp;

        let policy = &ctx.accounts.policy;
        let progress = &mut ctx.accounts.progress;

        // Day gating & init if page_index == 0
        if page_index == 0 {
            if progress.last_distribution_ts != 0 {
                require!(now_ts >= progress.last_distribution_ts + DAY_SECONDS, ErrorCode::DayWindowNotElapsed);
            }
            progress.day_start_ts = now_ts;
            progress.cumulative_distributed_today = 0;
            progress.cursor = 0;
            progress.treasury_snapshot = ctx.accounts.program_quote_treasury.amount;
            progress.page_records.clear();
        } else {
            // subsequent pages must be within same day
            require!(now_ts < progress.day_start_ts + DAY_SECONDS, ErrorCode::DayWindowNotElapsed);
            // allow retry of current page or advancing by one
            require!(page_index == progress.cursor || page_index == progress.cursor + 1, ErrorCode::InvalidCursor);
        }

        // 1) Claim fees (either via local stub or integration CPI)
        #[cfg(feature = "local-testing")]
        let (claimed_quote, claimed_base): (u64, u64) = {
            let claim = crate::cp_amm_stub::claim_fees_stub()?;
            (claim.quote_fees_collected, claim.base_fees_collected)
        };

        #[cfg(not(feature = "local-testing"))]
        {
            // TODO: replace with real cp-amm CPI claim and parse result
            return Err(ErrorCode::CpAmmIntegrationMissing.into());
        }

        // 2) Enforce quote-only (if any base fees observed -> fail deterministically)
        #[cfg(feature = "local-testing")]
        if claimed_base != 0 {
            return Err(ErrorCode::BaseFeesObserved.into());
        }

        // 3) Compute actual newly-claimed by comparing treasury snapshot (account might have funds put directly)
        let treasury_balance = ctx.accounts.program_quote_treasury.amount;
        let prev_snapshot = progress.treasury_snapshot;
        // claimed_by_claim = claimed_quote (from stub/CPI). We'll compute effective increase for safety:
        let effective_claimed = treasury_balance.saturating_sub(prev_snapshot);
        
        #[cfg(feature = "local-testing")]
        let effective_claimed_use = std::cmp::max(claimed_quote, effective_claimed);
        
        #[cfg(not(feature = "local-testing"))]
        let effective_claimed_use = effective_claimed; // When not testing, use treasury difference

        // 4) Read investor locked amounts from remaining_accounts
        // Expect pairs: [stream_acc, investor_ata]...
        let mut iter = ctx.remaining_accounts.iter();
        let mut inputs: Vec<InvestorInput> = Vec::new();
        while let Some(stream_acc) = iter.next() {
            if let Some(ata_acc) = iter.next() {
                #[cfg(feature = "local-testing")]
                let locked = crate::streamflow_stub::read_locked_stub(stream_acc)?;

                #[cfg(not(feature = "local-testing"))]
                {
                    // TODO: replace with Streamflow CPI/deserializing stream account
                    return Err(ErrorCode::StreamflowIntegrationMissing.into());
                }

                #[cfg(feature = "local-testing")]
                inputs.push(InvestorInput {
                    stream_pubkey: stream_acc.key(),
                    investor_ata: ata_acc.key(),
                    locked_amount: locked,
                });
            } else {
                return Err(ErrorCode::MissingInvestorAccount.into());
            }
        }
        // 5) locked_total and f_locked_bps
        let locked_total_u128: u128 = inputs.iter().map(|i| i.locked_amount as u128).sum();
        let y0 = policy.y0;
        let f_locked_bps: u64 = if y0 == 0 {
            0
        } else {
            ((locked_total_u128.saturating_mul(10_000_u128) / (y0 as u128)) as u64).min(10_000)
        };
        let eligible_bps = std::cmp::min(policy.investor_fee_share_bps as u64, f_locked_bps);

        // 6) Compute investor_fee_quote
        let mut investor_fee_quote: u64 = ((effective_claimed_use as u128).saturating_mul(eligible_bps as u128) / 10_000_u128) as u64;

        // 7) Apply daily cap
        if let Some(cap) = policy.daily_cap {
            let remaining_cap = cap.saturating_sub(progress.cumulative_distributed_today);
            if investor_fee_quote > remaining_cap {
                investor_fee_quote = remaining_cap;
            }
        }

        // If locked_total == 0, investor_fee_quote must be zero
        if locked_total_u128 == 0 {
            investor_fee_quote = 0;
        }

        // 8) Compute per-investor payouts (floor math)
        let mut payouts: Vec<(Pubkey, u64)> = Vec::with_capacity(inputs.len());
        let mut page_total_payout: u64 = 0;
        let mut page_dust: u64 = 0;

        if locked_total_u128 > 0 && investor_fee_quote > 0 {
            for inv in inputs.iter() {
                let numerator = (investor_fee_quote as u128).saturating_mul(inv.locked_amount as u128);
                let payout = (numerator / locked_total_u128) as u64;
                if payout < policy.min_payout {
                    page_dust = page_dust.saturating_add(payout);
                    payouts.push((inv.investor_ata, 0));
                } else {
                    payouts.push((inv.investor_ata, payout));
                    page_total_payout = page_total_payout.saturating_add(payout);
                }
            }
            // rounding leftover -> dust
            let rounding_leftover = investor_fee_quote.saturating_sub(page_total_payout).saturating_sub(page_dust);
            page_dust = page_dust.saturating_add(rounding_leftover);
        }

        // 9) Idempotency: check progress.page_records
        if progress.page_records.iter().any(|r| r.page_index == page_index) {
            // If already processed, emit event & return success (idempotent)
            emit!(InvestorPayoutPage {
                day_ts: progress.day_start_ts,
                page_index,
                page_total_payout,
                distributed_to_investors: page_total_payout,
                carry_after_page: progress.carry_lamports
            });
            // advance cursor if necessary
            if page_index > progress.cursor {
                progress.cursor = page_index;
            }
            return Ok(());
        }

        // 10) Transfer payouts from program_quote_treasury to investors
        // Must sign with investor_fee_pos_owner PDA
        let seeds: &[&[u8]] = &[
            b"vault",
            ctx.accounts.vault.key.as_ref(),
            b"investor_fee_pos_owner",
            &[investor_fee_pos_owner_bump],
        ];
        let signer_seeds = &[seeds];

        // Ensure treasury has enough
        require!(ctx.accounts.program_quote_treasury.amount >= page_total_payout, ErrorCode::InsufficientTreasuryBalance);

        for (dest_pubkey, amount) in payouts.iter() {
            if *amount == 0 {
                continue;
            }
            // find AccountInfo for dest in remaining_accounts
            let maybe_dest_info = find_account_info_by_pubkey(&ctx.remaining_accounts, dest_pubkey);
            let dest_info = maybe_dest_info.ok_or(ErrorCode::MissingInvestorAta)?;
            
            let cpi_accounts = Transfer {
                from: ctx.accounts.program_quote_treasury.to_account_info(),
                to: dest_info.to_account_info(),
                authority: ctx.accounts.investor_fee_pos_owner_pda.to_account_info(),
            };
            
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
            
            token::transfer(cpi_ctx, *amount)?;
        }
        // 11) Update progress: cumulative, carry, records, cursor
        progress.cumulative_distributed_today = progress.cumulative_distributed_today.saturating_add(page_total_payout);
        progress.carry_lamports = progress.carry_lamports.saturating_add(page_dust);
        progress.page_records.push(PageRecord { page_index, distributed: page_total_payout, timestamp: now_ts });
        if page_index > progress.cursor {
            progress.cursor = page_index;
        }

        emit!(QuoteFeesClaimed {
            pool: ctx.accounts.pool.key(),
            position: ctx.accounts.honorary_position.key(),
            claimed_quote: effective_claimed_use,
        });

        emit!(InvestorPayoutPage {
            day_ts: progress.day_start_ts,
            page_index,
            page_total_payout,
            distributed_to_investors: page_total_payout,
            carry_after_page: progress.carry_lamports
        });

        // 12) If last page: route remainder (and carry) to creator and finalize day
        if is_last_page {
            let total_claimed_today = ctx.accounts.program_quote_treasury.amount.saturating_sub(progress.treasury_snapshot);
            let total_distributed = progress.cumulative_distributed_today;
            let mut remainder = total_claimed_today.saturating_sub(total_distributed);
            if progress.carry_lamports > 0 {
                remainder = remainder.saturating_add(progress.carry_lamports);
                progress.carry_lamports = 0;
            }
            if remainder > 0 {
                require!(ctx.accounts.program_quote_treasury.amount >= remainder, ErrorCode::InsufficientTreasuryBalance);
                let cpi_accounts = Transfer {
                    from: ctx.accounts.program_quote_treasury.to_account_info(),
                    to: ctx.accounts.creator_quote_ata.to_account_info(),
                    authority: ctx.accounts.investor_fee_pos_owner_pda.to_account_info(),
                };
                token::transfer(
                    CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, signer_seeds),
                    remainder,
                )?;
            }

            progress.last_distribution_ts = now_ts;
            progress.treasury_snapshot = ctx.accounts.program_quote_treasury.amount;
            emit!(CreatorPayoutDayClosed { day_ts: progress.day_start_ts, creator_payout: remainder });
        }

        Ok(())
    }
}

/// ---------------------------------------------------------------------------
/// Accounts / Types
/// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(bump_owner: u8)]
pub struct InitializeHonoraryPosition<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    /// CHECK: Vault identifying this raise
    pub vault: UncheckedAccount<'info>,

    /// Policy PDA
    #[account(init, payer = initializer, space = 8 + Policy::INIT_SPACE, seeds = [b"policy", vault.key().as_ref()], bump)]
    pub policy: Account<'info, Policy>,

    /// Progress PDA
    #[account(init, payer = initializer, space = 8 + Progress::INIT_SPACE, seeds = [b"progress", vault.key().as_ref()], bump)]
    pub progress: Account<'info, Progress>,

    /// Owner PDA (will sign transfers)
    /// In many cases this is a system account type; we keep unchecked for flexibility
    /// CHECK: Seeds: [b"vault", vault, b"investor_fee_pos_owner"]
    #[account(seeds = [b"vault", vault.key().as_ref(), b"investor_fee_pos_owner"], bump)]
    pub investor_fee_pos_owner_pda: UncheckedAccount<'info>,

    /// CHECK: honorary DAMM position placeholder
    pub honorary_position: UncheckedAccount<'info>,

    ///  Program quote treasury ATA (owned by the investor_fee_pos_owner_pda)
    #[account(init, payer = initializer, token::mint = pool_quote_mint, token::authority = investor_fee_pos_owner_pda)]
    pub program_quote_treasury: Account<'info, TokenAccount>,

    /// CHECK: Pool & mints (used for validation)
    pub pool: UncheckedAccount<'info>,
    pub pool_quote_mint: Account<'info, anchor_spl::token::Mint>,
    pub pool_base_mint: Account<'info, anchor_spl::token::Mint>,

    /// CHECK: cp-amm program (unchecked for now)
    pub cp_amm_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
#[instruction()]
pub struct CrankDistribute<'info> {
    /// Permissionless caller
    pub cranker: Signer<'info>,

    /// CHECK: Vault
    pub vault: UncheckedAccount<'info>,

    #[account(mut, seeds = [b"policy", vault.key().as_ref()], bump)]
    pub policy: Account<'info, Policy>,

    #[account(mut, seeds = [b"progress", vault.key().as_ref()], bump)]
    pub progress: Account<'info, Progress>,

    /// CHECK: PDA authority (signing for transfers)
    #[account(seeds = [b"vault", vault.key().as_ref(), b"investor_fee_pos_owner"], bump)]
    pub investor_fee_pos_owner_pda: UncheckedAccount<'info>,
 
    /// CHECK: bump (passed as account to make deriving signer seeds easy in client)
    pub honorary_position: UncheckedAccount<'info>,
    // pub investor_fee_pos_owner_bump: u8,

    /// Treasury & creator ATA
    #[account(mut)]
    pub program_quote_treasury: Account<'info, TokenAccount>,

    #[account(mut)]
    pub creator_quote_ata: Account<'info, TokenAccount>,

    /// CHECK: Pool & mints
    pub pool: UncheckedAccount<'info>,
    pub pool_quote_mint: Account<'info, anchor_spl::token::Mint>,
    pub pool_base_mint: Account<'info, anchor_spl::token::Mint>,

    /// CHECK: cp-amm program (for CPI)
    pub cp_amm_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
}

/// Policy account
#[account]
pub struct Policy {
    pub vault: Pubkey,
    pub y0: u128,
    pub investor_fee_share_bps: u16,
    pub daily_cap: Option<u64>,
    pub min_payout: u64,
    pub dust_threshold: u64,
}
impl Policy {
    pub const INIT_SPACE: usize = 32 + 16 + 2 + 9 + 8 + 8 ; // generous
}

/// Progress account (tracks day & pages)
#[account]
pub struct Progress {
    pub vault: Pubkey,
    pub day_start_ts: i64,
    pub last_distribution_ts: i64,
    pub cumulative_distributed_today: u64,
    pub carry_lamports: u64,
    pub cursor: u64,
    pub treasury_snapshot: u64,
    pub page_records: Vec<PageRecord>,
}
impl Progress {
    pub const INIT_SPACE: usize = 32 + 8 + 8 + 8 + 8 + 8 + 8 + (4 + PAGE_RECORD_CAP * PageRecord::SIZE);
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PageRecord {
    pub page_index: u32,
    pub distributed: u64,
    pub timestamp: i32,
}
impl PageRecord {
    pub const SIZE: usize = 4 + 8 + 4;
}

#[derive(Clone)]
struct InvestorInput {
    pub stream_pubkey: Pubkey,
    pub investor_ata: Pubkey,
    pub locked_amount: u64,
}

/// ---------------------------------------------------------------------------
/// Events
/// ---------------------------------------------------------------------------

#[event]
pub struct HonoraryPositionInitialized {
    pub pool: Pubkey,
    pub position: Pubkey,
    pub quote_mint: Pubkey,
}

#[event]
pub struct QuoteFeesClaimed {
    pub pool: Pubkey,
    pub position: Pubkey,
    pub claimed_quote: u64,
}

#[event]
pub struct InvestorPayoutPage {
    pub day_ts: i64,
    pub page_index: u64,
    pub page_total_payout: u64,
    pub distributed_to_investors: u64,
    pub carry_after_page: u64,
}

#[event]
pub struct CreatorPayoutDayClosed {
    pub day_ts: i64,
    pub creator_payout: u64,
}

/// ---------------------------------------------------------------------------
/// Helpers
/// ---------------------------------------------------------------------------

fn find_account_info_by_pubkey<'info>(
    accounts: &'info [AccountInfo<'info>], 
    key: &Pubkey
) -> Option<&'info AccountInfo<'info>> {
    accounts.iter().find(|a| a.key == key)
}

/// ---------------------------------------------------------------------------
/// Errors
/// ---------------------------------------------------------------------------

#[error_code]
pub enum ErrorCode {
    #[msg("Cannot guarantee quote-only accrual under current pool config.")]
    QuoteOnlyNotGuaranteed = 6000,

    #[msg("Base-fee detected during claim; aborting.")]
    BaseFeesObserved = 6001,

    #[msg("24h window not elapsed for a new distribution day.")]
    DayWindowNotElapsed = 6002,

    #[msg("No locked funds to distribute.")]
    NoLockedFunds = 6003,

    #[msg("Daily cap reached or insufficient remaining cap.")]
    DailyCapReached = 6004,

    #[msg("Invalid pagination cursor.")]
    InvalidCursor = 6005,

    #[msg("Missing or invalid Streamflow account.")]
    MissingInvestorStreamflow = 6006,

    #[msg("Investor ATA missing.")]
    MissingInvestorAta = 6007,

    #[msg("Insufficient treasury balance to perform transfers.")]
    InsufficientTreasuryBalance = 6008,

    #[msg("Pool token order mismatch (quote/base).")]
    PoolTokenOrderMismatch = 6009,

    #[msg("Invalid cp-amm program id or CPI (integration not wired).")]
    CpAmmIntegrationMissing = 6010,

    #[msg("Invalid Streamflow integration (not wired).")]
    StreamflowIntegrationMissing = 6011,

    #[msg("Invalid BPS value.")]
    InvalidBps = 6012,

    #[msg("Missing investor remaining account pair (stream, ata).")]
    MissingInvestorAccount = 6013,
}

/// ---------------------------------------------------------------------------
/// Local Testing Stubs (only compiled when feature = "local-testing")
/// ---------------------------------------------------------------------------

#[cfg(feature = "local-testing")]
pub mod cp_amm_stub {
    use super::*;

    #[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
    pub struct ClaimResult {
        pub quote_fees_collected: u64,
        pub base_fees_collected: u64,
    }

    /// Simple deterministic stub for tests: returns quote-only fees
    pub fn claim_fees_stub() -> Result<ClaimResult> {
        Ok(ClaimResult {
            quote_fees_collected: 100_000, // default simulated claim
            base_fees_collected: 0,
        })
    }
}

#[cfg(feature = "local-testing")]
pub mod streamflow_stub {
    use super::*;

    /// Read locked from a mock stream account
    /// For tests: the account data first 8 bytes is u64 locked amount LE
    pub fn read_locked_stub(acc: &AccountInfo) -> Result<u64> {
        let data = acc.try_borrow_data()?;
        if data.len() < 8 {
            return err!(ErrorCode::MissingInvestorStreamflow);
        }
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&data[0..8]);
        Ok(u64::from_le_bytes(arr))
    }
}
