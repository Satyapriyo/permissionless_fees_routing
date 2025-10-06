# DAMM v2 Honorary Quote-Only Fee Position Module

A standalone, Anchor-compatible Solana program that creates and manages an "honorary" DAMM v2 LP position owned by a program PDA. This position accrues fees exclusively in the quote mint and provides a permissionless 24-hour distribution crank that distributes fees pro-rata to investors based on their locked amounts.

## 🎯 Overview

This module implements a fee distribution system for token launches where:
- **Honorary Position**: A DAMM v2 LP position owned by the program that only accrues quote token fees
- **24h Distribution Crank**: Permissionless mechanism that distributes fees once per day
- **Pro-rata Distribution**: Fees distributed based on investors' still-locked token amounts from Streamflow
- **Creator Remainder**: Unclaimed fees and dust go to the project creator

## 📋 Features

- ✅ **Quote-only fee accrual** with deterministic base fee rejection
- ✅ **Program-owned PDA** position management  
- ✅ **24-hour gated distributions** with pagination support
- ✅ **Pro-rata investor payouts** based on locked amounts
- ✅ **Daily caps and dust handling** with carry-over
- ✅ **Idempotent pagination** for reliable execution
- ✅ **Comprehensive events** for tracking and monitoring

## 🚀 Setup Instructions

### Prerequisites

- Solana CLI tools (v1.16+)
- Anchor framework (v0.28+)
- Node.js (v16+) and npm/yarn
- Rust (latest stable)

### Installation

```bash
# Clone the repository
git clone https://github.com/Satyapriyo/permissionless_fees_routing.git
cd permissionless_fees_routing

# Install dependencies
npm install

# Build the program
anchor build

# Start local validator in new terminal
solana-test-validator

# Deploy to localnet (for testing)
anchor deploy 
```
After deploying update the programId that you just got to
```
programs/honorary-fee-position/src/lib.rs

Anchor.toml
```


### Running Tests

```bash
# Start local validator in new terminal if not running
solana-test-validator

# Deploy first if not deployed
anchor deploy

# Stop the local validator of other terminal and run comprehensive test suite
anchor test 


```

## 🏗️ Integration Guide

### 1. Initialize Honorary Position

```typescript
import * as anchor from "@coral-xyz/anchor";
import { PublicKey, Keypair } from "@solana/web3.js";

const vault = Keypair.generate();
const quoteMint = new PublicKey("your_quote_mint_address");
const baseMint = new PublicKey("your_base_mint_address");

// Derive PDAs
const [policyPda] = PublicKey.findProgramAddressSync(
  [Buffer.from("policy"), vault.publicKey.toBuffer()],
  program.programId
);

const [progressPda] = PublicKey.findProgramAddressSync(
  [Buffer.from("progress"), vault.publicKey.toBuffer()],
  program.programId
);

const [investorFeeOwnerPda, investorFeeOwnerBump] = PublicKey.findProgramAddressSync(
  [
    Buffer.from("vault"),
    vault.publicKey.toBuffer(),
    Buffer.from("investor_fee_pos_owner"),
  ],
  program.programId
);

// Initialize the honorary position
await program.methods
  .initializeHonoraryPosition(
    investorFeeOwnerBump,
    new anchor.BN(1000000),     // Y0 - total allocation at TGE
    5000,                       // 50% investor fee share (in BPS)
    new anchor.BN(100000),      // Daily cap (optional)
    new anchor.BN(1000),        // Minimum payout threshold
    new anchor.BN(100)          // Dust threshold
  )
  .accounts({
    initializer: payer.publicKey,
    vault: vault.publicKey,
    policy: policyPda,
    progress: progressPda,
    investorFeePosOwnerPda: investorFeeOwnerPda,
    honoraryPosition: honoraryPositionPubkey,
    programQuoteTreasury: treasuryATA,
    pool: poolPubkey,
    poolQuoteMint: quoteMint,
    poolBaseMint: baseMint,
    cpAmmProgram: cpAmmProgramId,
    tokenProgram: TOKEN_PROGRAM_ID,
    systemProgram: SystemProgram.programId,
    rent: SYSVAR_RENT_PUBKEY,
    associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
  })
  .rpc();
```

### 2. Run Distribution Crank

```typescript
// Prepare investor accounts (pairs of stream + ATA)
const remainingAccounts = [];
for (const investor of investors) {
  remainingAccounts.push(
    { pubkey: investor.streamPubkey, isSigner: false, isWritable: false },
    { pubkey: investor.quoteMintATA, isSigner: false, isWritable: true }
  );
}

// Execute crank (page 0 starts new day)
await program.methods
  .crankDistribute(
    investorFeeOwnerBump,
    new anchor.BN(0),  // page_index (0 = first page)
    false              // is_last_page
  )
  .accounts({
    cranker: payer.publicKey,
    vault: vault.publicKey,
    policy: policyPda,
    progress: progressPda,
    investorFeePosOwnerPda: investorFeeOwnerPda,
    honoraryPosition: honoraryPositionPubkey,
    programQuoteTreasury: treasuryATA,
    creatorQuoteAta: creatorQuoteATA,
    pool: poolPubkey,
    poolQuoteMint: quoteMint,
    poolBaseMint: baseMint,
    cpAmmProgram: cpAmmProgramId,
    tokenProgram: TOKEN_PROGRAM_ID,
  })
  .remainingAccounts(remainingAccounts)
  .rpc();
```

## 📊 Account Tables

### Core PDAs

| Account | Seeds | Space | Purpose |
|---------|-------|-------|---------|
| `Policy` | `["policy", vault]` | 75 bytes | Stores distribution parameters |
| `Progress` | `["progress", vault]` | ~1KB | Tracks daily distribution state |
| `InvestorFeeOwnerPda` | `["vault", vault, "investor_fee_pos_owner"]` | 0 bytes | PDA authority for transfers |

### Account Structures

#### Policy Account
```rust
pub struct Policy {
    pub vault: Pubkey,                    // 32 bytes
    pub y0: u128,                        // 16 bytes - Total TGE allocation
    pub investor_fee_share_bps: u16,     // 2 bytes - Max investor share (BPS)
    pub daily_cap: Option<u64>,          // 9 bytes - Optional daily limit
    pub min_payout: u64,                 // 8 bytes - Minimum payout threshold
    pub dust_threshold: u64,             // 8 bytes - Dust accumulation threshold
}
```

#### Progress Account
```rust
pub struct Progress {
    pub vault: Pubkey,                        // 32 bytes
    pub day_start_ts: i64,                   // 8 bytes - Day start timestamp
    pub last_distribution_ts: i64,           // 8 bytes - Last distribution time
    pub cumulative_distributed_today: u64,   // 8 bytes - Total distributed today
    pub carry_lamports: u64,                 // 8 bytes - Carried dust amount
    pub cursor: u64,                         // 8 bytes - Pagination cursor
    pub treasury_snapshot: u64,              // 8 bytes - Treasury balance snapshot
    pub page_records: Vec<PageRecord>,       // Variable - Page execution history
}

pub struct PageRecord {
    pub page_index: u32,        // 4 bytes
    pub distributed: u64,       // 8 bytes  
    pub timestamp: i64,         // 8 bytes
}
```

### Required Accounts

#### InitializeHonoraryPosition
| Account | Type | Constraint | Description |
|---------|------|------------|-------------|
| `initializer` | `Signer` | `mut` | Transaction payer |
| `vault` | `UncheckedAccount` | - | Vault identifier |
| `policy` | `Account<Policy>` | `init` | Policy PDA |
| `progress` | `Account<Progress>` | `init` | Progress PDA |
| `investor_fee_pos_owner_pda` | `UncheckedAccount` | PDA | Transfer authority |
| `program_quote_treasury` | `Account<TokenAccount>` | `init_if_needed` | Fee treasury ATA |
| `pool_quote_mint` | `Account<Mint>` | - | Quote token mint |
| `pool_base_mint` | `Account<Mint>` | - | Base token mint |

#### CrankDistribute
| Account | Type | Constraint | Description |
|---------|------|------------|-------------|
| `cranker` | `Signer` | - | Permissionless caller |
| `policy` | `Account<Policy>` | `mut` | Policy PDA |
| `progress` | `Account<Progress>` | `mut` | Progress PDA |
| `program_quote_treasury` | `Account<TokenAccount>` | `mut` | Source of fee distributions |
| `creator_quote_ata` | `Account<TokenAccount>` | `mut` | Creator's quote token account |

### Remaining Accounts Format

The `remaining_accounts` must be provided as pairs in this exact order:
```
[stream_account_0, investor_ata_0, stream_account_1, investor_ata_1, ...]
```

**Each pair represents:**
- `stream_account`: Streamflow account containing locked amount data (readable)
- `investor_ata`: Investor's Associated Token Account for quote mint (writable)

## ⚙️ Configuration Parameters

### Policy Settings

| Parameter | Type | Range | Description |
|-----------|------|-------|-------------|
| `y0` | `u128` | > 0 | Total investor allocation minted at TGE |
| `investor_fee_share_bps` | `u16` | 0-10,000 | Maximum investor share in basis points |
| `daily_cap` | `Option<u64>` | Optional | Maximum tokens distributed per day |
| `min_payout` | `u64` | ≥ 0 | Minimum payout per investor (dust threshold) |
| `dust_threshold` | `u64` | ≥ 0 | Additional dust accumulation threshold |

### Distribution Formula

The distribution follows this mathematical model:

```rust
// 1. Calculate locked percentage
locked_total = sum(locked_i for all investors)
f_locked_bps = min(10000, (locked_total * 10000) / Y0)

// 2. Determine eligible investor share
eligible_bps = min(investor_fee_share_bps, f_locked_bps)

// 3. Calculate total investor portion
investor_fee_quote = (claimed_quote * eligible_bps) / 10000

// 4. Apply daily cap
if daily_cap.is_some() {
    remaining_cap = daily_cap - cumulative_distributed_today
    investor_fee_quote = min(investor_fee_quote, remaining_cap)
}

// 5. Pro-rata distribution with floor math
for each investor_i {
    weight_i = locked_i / locked_total
    payout_i = floor(investor_fee_quote * weight_i)
    
    if payout_i < min_payout {
        payout_i = 0  // Becomes dust
        carry_lamports += payout_i
    }
}

// 6. Creator gets remainder
creator_payout = claimed_quote - total_investor_payouts
```

## 🚨 Error Codes Reference

| Code | Error | Description | Resolution |
|------|-------|-------------|------------|
| 6000 | `QuoteOnlyNotGuaranteed` | Cannot guarantee quote-only accrual | Check pool configuration |
| 6001 | `BaseFeesObserved` | Base fees detected during claim | Verify pool token order |
| 6002 | `DayWindowNotElapsed` | 24h window not elapsed | Wait for next distribution window |
| 6003 | `NoLockedFunds` | No locked funds to distribute | Check investor lock status |
| 6004 | `DailyCapReached` | Daily cap exceeded | Increase cap or wait for next day |
| 6005 | `InvalidCursor` | Invalid pagination cursor | Use correct page sequence |
| 6006 | `MissingInvestorStreamflow` | Streamflow account missing/invalid | Provide valid stream accounts |
| 6007 | `MissingInvestorAta` | Investor ATA missing | Create investor token accounts |
| 6008 | `InsufficientTreasuryBalance` | Not enough treasury balance | Ensure adequate treasury funds |
| 6009 | `PoolTokenOrderMismatch` | Quote/base mint order wrong | Verify token mint addresses |
| 6010 | `CpAmmIntegrationMissing` | CP-AMM integration not wired | Implement CP-AMM CPI calls |
| 6011 | `StreamflowIntegrationMissing` | Streamflow integration not wired | Implement Streamflow integration |
| 6012 | `InvalidBps` | BPS value out of range (0-10,000) | Use valid basis points |
| 6013 | `MissingInvestorAccount` | Missing investor account pair | Provide complete account pairs |

## 📅 Day & Pagination Semantics

### Day Initialization
- **Trigger**: First crank with `page_index = 0`
- **Gate**: Requires `now >= last_distribution_ts + 86400` seconds
- **Actions**: 
  - Sets `day_start_ts = now`
  - Resets `cumulative_distributed_today = 0`
  - Clears `page_records`
  - Takes treasury snapshot

### Pagination Flow
```
Day 1: [Page 0] -> [Page 1] -> ... -> [Page N (last=true)]
         ↓           ↓                    ↓
      Initialize   Continue         Finalize + Creator
       new day    distribution        Payout
```

### Page Execution Rules
1. **Page 0**: Must wait 24h since last distribution
2. **Pages 1+**: Must be within same day (`now < day_start_ts + 86400`)
3. **Cursor**: Can retry current page or advance by 1
4. **Last Page**: Routes remainder to creator and sets `last_distribution_ts`

### Idempotency Guarantees
- **Page Records**: Prevent double-execution of same page
- **Cursor Tracking**: Ensures proper sequence
- **State Recovery**: Safe to restart mid-day after failures

## 🎉 Events Documentation

### HonoraryPositionInitialized
```rust
pub struct HonoraryPositionInitialized {
    pub pool: Pubkey,           // Pool address
    pub position: Pubkey,       // Position account
    pub quote_mint: Pubkey,     // Quote token mint
}
```

### QuoteFeesClaimed
```rust
pub struct QuoteFeesClaimed {
    pub pool: Pubkey,           // Pool address
    pub position: Pubkey,       // Position account
    pub claimed_quote: u64,     // Amount of quote fees claimed
}
```

### InvestorPayoutPage
```rust
pub struct InvestorPayoutPage {
    pub day_ts: i64,                    // Day start timestamp
    pub page_index: u64,                // Page number
    pub page_total_payout: u64,         // Total paid this page
    pub distributed_to_investors: u64,  // Amount to investors
    pub carry_after_page: u64,          // Accumulated dust
}
```

### CreatorPayoutDayClosed
```rust  
pub struct CreatorPayoutDayClosed {
    pub day_ts: i64,            // Day start timestamp
    pub creator_payout: u64,    // Amount paid to creator
}
```

## 🧪 Testing Scenarios

The comprehensive test suite covers all critical scenarios:

### Basic Flow Tests
- ✅ **Initialization**: Creates PDAs, treasury ATA, validates parameters
- ✅ **Treasury Funding**: Mints tokens to program treasury
- ✅ **Single Page Distribution**: Basic fee claiming and distribution

### Edge Case Coverage  
- ✅ **All Investors Unlocked**: Routes 100% to creator when `locked_total = 0`
- ✅ **Partial Locks**: Verifies proportional distribution with different locked amounts
- ✅ **Dust Handling**: Tests dust accumulation below `min_payout` threshold
- ✅ **Daily Cap**: Verifies cap enforcement and remainder handling
- ✅ **Base Fee Detection**: Fails deterministically when base fees observed
- ✅ **Idempotency**: Retrying pages doesn't double-pay
- ✅ **Account States**: Verifies final policy and progress states

### Test Execution
```bash
# Run all tests(No need to start local validator this starts one automatically)
anchor test 

```

### Test Data Validation
The tests validate:
- Mathematical correctness of pro-rata distribution
- Proper weight calculation: `weight_i = locked_i / locked_total`
- Floor math implementation
- Dust carry-over mechanics
- Daily cap enforcement
- Creator remainder calculation

## 🔗 Integration Requirements

### For Production Deployment

**You must provide:**

1. **Creator Configuration**
   - Creator wallet quote token ATA
   - Creator public key for remainder routing

2. **Investor Distribution Set** (paginated)
   - Array of Streamflow stream pubkeys per investor
   - Corresponding investor quote token ATAs
   - Expected pagination size (recommend 10-20 investors per page)

3. **CP-AMM Integration**
   - Pool program ID and pool account
   - Position account management
   - Fee claiming CPI implementation

4. **Policy Configuration**
   - `Y0`: Total investor allocation minted at TGE
   - `investor_fee_share_bps`: Maximum investor share (0-10,000 BPS)
   - `daily_cap`: Optional daily distribution limit
   - `min_payout`: Minimum individual payout threshold
   - `dust_threshold`: Dust accumulation threshold

### Required Code Replacements

Replace the `#[cfg(feature = "local-testing")]` stubs with production implementations:

#### 1. CP-AMM Integration
```rust
// Replace this stub:
#[cfg(feature = "local-testing")]
let (claimed_quote, claimed_base) = crate::cp_amm_stub::claim_fees_stub()?;

// With real CP-AMM CPI:
#[cfg(not(feature = "local-testing"))]
let claim_result = cp_amm::cpi::claim_fees(
    CpiContext::new(
        ctx.accounts.cp_amm_program.to_account_info(),
        cp_amm::cpi::accounts::ClaimFees {
            position: ctx.accounts.honorary_position.to_account_info(),
            // ... other CP-AMM accounts
        }
    )
)?;
```

#### 2. Streamflow Integration
```rust
// Replace this stub:
#[cfg(feature = "local-testing")]
let locked = crate::streamflow_stub::read_locked_stub(stream_acc)?;

// With real Streamflow account parsing:
#[cfg(not(feature = "local-testing"))]
let stream_data = StreamData::try_deserialize(&mut &stream_acc.data.borrow()[..])?;
let locked = stream_data.remaining_locked_amount;
```

## 🛡️ Security Considerations

### Access Control
- **Permissionless Cranking**: Anyone can call the crank (rate-limited to 24h)
- **PDA Ownership**: All critical operations signed by program PDAs
- **Treasury Control**: Only program PDA can authorize transfers from treasury

### Validation & Safety
- **Pool Token Order**: Validates quote mint ≠ base mint at initialization
- **Quote-only Enforcement**: Fails deterministically if base fees detected
- **Overflow Protection**: Uses saturating math operations throughout
- **Account Validation**: Verifies ATA ownership and mint relationships

### Economic Security
- **Daily Caps**: Prevents excessive distributions in single day
- **Minimum Payouts**: Prevents dust attacks via tiny distributions
- **Idempotency**: Prevents double-spending via page replay attacks
- **Remainder Protection**: Ensures creator receives unclaimed funds

### Integration Safety
- **Seed Determinism**: All PDAs use deterministic, collision-resistant seeds
- **Account Size Limits**: Progress account sized to prevent bloat
- **State Consistency**: Atomic updates prevent partial state corruption

## 📝 Development Notes

### Local Testing Features
The module includes comprehensive testing stubs enabled with `feature = "local-testing"`:

- **CP-AMM Stub**: Simulates different fee scenarios (normal, large, small, base fees)
- **Streamflow Stub**: Provides deterministic locked amounts for testing
- **Scenario Testing**: Different page indices trigger different stub behaviors

### Architecture Decisions
- **Pagination**: Supports large investor lists without transaction size limits
- **Idempotency**: Safe retry mechanism for production reliability  
- **Event Emission**: Comprehensive logging for monitoring and debugging
- **Modular Design**: Clean separation between core logic and integration stubs

## 📄 License

This project is implemented as part of Star Protocol's DAMM v2 Honorary Fee Position bounty.

---

**Program ID**: `Y6S8ztXqBsRsj9husmE2PmLm3cLqbfwbmf1o1KNFsNk`

**Built for Star Protocol's Fundraising Platform**
