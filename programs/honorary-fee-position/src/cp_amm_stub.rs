use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

/// Simulated output from a fee claim on DAMM v2.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct DammClaimQuoteFees {
    pub quote_fees_collected: u64,
    pub base_fees_collected: u64,
}

#[program]
pub mod cp_amm_stub {
    use super::*;

    /// Standard quote-only fee claim (for normal testing)
    pub fn claim_fees(ctx: Context<ClaimFeesStub>) -> Result<DammClaimQuoteFees> {
        // Simulate deterministic quote-only fee claim
        let fees_to_transfer = 100_000u64;
        
        // Actually transfer tokens to make the test realistic
        if fees_to_transfer > 0 {
            let cpi_accounts = Transfer {
                from: ctx.accounts.pool_quote_vault.to_account_info(),
                to: ctx.accounts.quote_treasury.to_account_info(),
                authority: ctx.accounts.pool.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            
            // Transfer the fees
            token::transfer(cpi_ctx, fees_to_transfer)?;
            
            msg!("CP-AMM Stub: Transferred {} quote fees to treasury", fees_to_transfer);
        }

        // Emit event for tracking
        emit!(QuoteFeesCollected {
            amount: fees_to_transfer,
            base_amount: 0,
            treasury: ctx.accounts.quote_treasury.key(),
        });
        
        Ok(DammClaimQuoteFees {
            quote_fees_collected: fees_to_transfer,
            base_fees_collected: 0, // Always 0 to ensure quote-only
        })
    }

    /// Claim fees with base amount (for testing base fee detection)
    pub fn claim_fees_with_base(ctx: Context<ClaimFeesStub>) -> Result<DammClaimQuoteFees> {
        let quote_fees = 50_000u64;
        let base_fees = 25_000u64; // This should trigger failure in main program
        
        msg!("CP-AMM Stub: Simulating MIXED fees - quote: {}, base: {}", quote_fees, base_fees);
        
        emit!(QuoteFeesCollected {
            amount: quote_fees,
            base_amount: base_fees,
            treasury: ctx.accounts.quote_treasury.key(),
        });
        
        Ok(DammClaimQuoteFees {
            quote_fees_collected: quote_fees,
            base_fees_collected: base_fees, // Non-zero base fees
        })
    }

    /// Large fee claim (for testing caps)
    pub fn claim_large_fees(ctx: Context<ClaimFeesStub>) -> Result<DammClaimQuoteFees> {
        let large_fees = 1_000_000u64; // Large amount to test daily caps
        
        // Transfer if possible
        if large_fees > 0 {
            let cpi_accounts = Transfer {
                from: ctx.accounts.pool_quote_vault.to_account_info(),
                to: ctx.accounts.quote_treasury.to_account_info(),
                authority: ctx.accounts.pool.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            
            token::transfer(cpi_ctx, large_fees)?;
            msg!("CP-AMM Stub: Transferred {} LARGE quote fees", large_fees);
        }

        emit!(QuoteFeesCollected {
            amount: large_fees,
            base_amount: 0,
            treasury: ctx.accounts.quote_treasury.key(),
        });
        
        Ok(DammClaimQuoteFees {
            quote_fees_collected: large_fees,
            base_fees_collected: 0,
        })
    }

    /// Small fee claim (for testing dust thresholds)
    pub fn claim_small_fees(ctx: Context<ClaimFeesStub>) -> Result<DammClaimQuoteFees> {
        let small_fees = 10u64; // Very small amount to test dust handling
        
        msg!("CP-AMM Stub: Simulating SMALL fees: {}", small_fees);
        
        emit!(QuoteFeesCollected {
            amount: small_fees,
            base_amount: 0,
            treasury: ctx.accounts.quote_treasury.key(),
        });
        
        Ok(DammClaimQuoteFees {
            quote_fees_collected: small_fees,
            base_fees_collected: 0,
        })
    }

    /// No fees available
    pub fn claim_no_fees(_ctx: Context<ClaimFeesStub>) -> Result<DammClaimQuoteFees> {
        msg!("CP-AMM Stub: No fees available to claim");
        
        Ok(DammClaimQuoteFees {
            quote_fees_collected: 0,
            base_fees_collected: 0,
        })
    }
}

#[derive(Accounts)]
pub struct ClaimFeesStub<'info> {
    /// Pool authority (signer for fee transfers)
    #[account(mut)]
    pub pool: Signer<'info>,
    
    /// Pool's quote token vault (source of fees)
    #[account(mut)]
    pub pool_quote_vault: Account<'info, TokenAccount>,
    
    /// Treasury account receiving fees (destination)
    #[account(mut)]
    pub quote_treasury: Account<'info, TokenAccount>,
    
    /// Token program for transfers
    pub token_program: Program<'info, Token>,
}

/// Event emitted when fees are collected
#[event]
pub struct QuoteFeesCollected {
    pub amount: u64,
    pub base_amount: u64,
    pub treasury: Pubkey,
}

/// Error codes for the stub
#[error_code]
pub enum CpAmmStubError {
    #[msg("Insufficient fees in pool vault")]
    InsufficientFees,
    #[msg("Invalid treasury account")]
    InvalidTreasury,
}