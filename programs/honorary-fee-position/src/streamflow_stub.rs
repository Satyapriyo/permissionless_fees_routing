use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct StreamLockInfo {
    pub locked_amount: u64,
}

#[program]
pub mod streamflow_stub {
    use super::*;

    pub fn get_locked_amount(_ctx: Context<GetLockInfoStub>) -> Result<StreamLockInfo> {
        // Return hardcoded value for testing
        Ok(StreamLockInfo { locked_amount: 100_000 })
    }
}

#[derive(Accounts)]
pub struct GetLockInfoStub<'info> {
    /// Streamflow stream account
    pub stream: AccountInfo<'info>,
}