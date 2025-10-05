use anchor_lang::prelude::*;

/// Simulated output from a fee claim on DAMM v2.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct DammClaimQuoteFees {
    pub quote_fees_collected: u64,
    pub base_fees_collected: u64,
}

#[program]
pub mod cp_amm_stub {
    use super::*;

    pub fn claim_fees(_ctx: Context<ClaimFeesStub>) -> Result<DammClaimQuoteFees> {
        // Simulate deterministic quote-only fee claim
        Ok(DammClaimQuoteFees {
            quote_fees_collected: 100_000,
            base_fees_collected: 0,
        })
    }
}

#[derive(Accounts)]
pub struct ClaimFeesStub<'info> {
    /// Placeholder account for the AMM pool
    #[account(mut)]
    pub pool: Signer<'info>,
    /// Treasury account receiving fees
    #[account(mut)]
    pub quote_treasury: AccountInfo<'info>,
}
