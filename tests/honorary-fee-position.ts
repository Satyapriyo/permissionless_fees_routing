import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { HonoraryFeePosition } from "../target/types/honorary_fee_position";
import { 
  PublicKey, 
  Keypair, 
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
  Transaction
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  mintTo,
  getOrCreateAssociatedTokenAccount,
  createInitializeAccountInstruction,
  getAccountLenForMint,
  ACCOUNT_SIZE
} from "@solana/spl-token";
import { assert } from "chai";

// Set up wallet if not provided
if (!process.env.ANCHOR_WALLET) {
  process.env.ANCHOR_WALLET = require('os').homedir() + '/.config/solana/id.json';
}

describe("honorary_fee_position (local testing)", () => {
  const provider = anchor.AnchorProvider.local();
  anchor.setProvider(provider);
  const program = anchor.workspace.HonoraryFeePosition as Program<HonoraryFeePosition>;

  const payer = provider.wallet as anchor.Wallet;
  let vault = Keypair.generate();
  let quoteMint: PublicKey;
  let baseMint: PublicKey;
  let programTreasuryKeypair: Keypair;
  let creatorQuoteAta: PublicKey;
  
  // PDAs
  let policyPda: PublicKey;
  let progressPda: PublicKey;
  let investorFeeOwnerPda: PublicKey;
  let bumps: any = {};

  before(async () => {
    // Create mints
    quoteMint = await createMint(
      provider.connection,
      payer.payer,
      payer.publicKey,
      null,
      6
    );

    baseMint = await createMint(
      provider.connection,
      payer.payer,
      payer.publicKey,
      null,
      6
    );

    // Derive PDAs
    [policyPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("policy"), vault.publicKey.toBuffer()],
      program.programId
    );

    [progressPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("progress"), vault.publicKey.toBuffer()],
      program.programId
    );

    [investorFeeOwnerPda, bumps.investorFeeOwner] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("vault"),
        vault.publicKey.toBuffer(),
        Buffer.from("investor_fee_pos_owner")
      ],
      program.programId
    );

    // Create program treasury keypair
    programTreasuryKeypair = Keypair.generate();

    // Create creator ATA
    const creatorAta = await getOrCreateAssociatedTokenAccount(
      provider.connection,
      payer.payer,
      quoteMint,
      payer.publicKey
    );
    creatorQuoteAta = creatorAta.address;
  });

  it("Initializes program + PDAs", async () => {
    // Use the exact account names from the generated types
    const tx = await program.methods
      .initializeHonoraryPosition(
        bumps.investorFeeOwner,
        new anchor.BN(1000000), // y0
        5000, // 50% investor fee share
        new anchor.BN(1000000), // daily cap
        new anchor.BN(1000), // min payout
        new anchor.BN(100) // dust threshold
      )
      .accounts({
        initializer: payer.publicKey,
        vault: vault.publicKey,
        policy: policyPda,
        progress: progressPda,
        investorFeePosOwnerPda: investorFeeOwnerPda,
        honoraryPosition: Keypair.generate().publicKey,
        programQuoteTreasury: programTreasuryKeypair.publicKey,
        pool: Keypair.generate().publicKey,
        poolQuoteMint: quoteMint,
        poolBaseMint: baseMint,
        cpAmmProgram: Keypair.generate().publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .preInstructions([
        // Add compute budget to handle larger accounts
        anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({
          units: 300000, // Increase compute units
        }),
      ])
      .signers([payer.payer, programTreasuryKeypair])
      .rpc();

    console.log("✅ Initialized program + PDAs, tx:", tx);

    // Verify accounts were created
    const policyAccount = await program.account.policy.fetch(policyPda);
    const progressAccount = await program.account.progress.fetch(progressPda);
    
    assert.ok(policyAccount);
    assert.ok(progressAccount);
    console.log("✅ Policy and Progress accounts created successfully");
  });

  it("Creates mock investor stream accounts with locked balances", async () => {
    const streams = [
      { keypair: Keypair.generate(), locked: 10000 },
      { keypair: Keypair.generate(), locked: 20000 },
      { keypair: Keypair.generate(), locked: 30000 }
    ];

    for (let stream of streams) {
      const createAccountTx = await provider.connection.requestAirdrop(
        stream.keypair.publicKey,
        anchor.web3.LAMPORTS_PER_SOL * 0.01
      );
      await provider.connection.confirmTransaction(createAccountTx);
    }

    console.log("✅ Created 3 mock stream accounts and investor ATAs");
  });

  it("Funds the program quote treasury", async () => {
    // First verify the treasury account exists and get its info
    const accountInfo = await provider.connection.getAccountInfo(programTreasuryKeypair.publicKey);
    
    if (!accountInfo) {
      throw new Error("Treasury account not initialized. The first test may have failed.");
    }

    // Mint tokens to the treasury
    const mintAmount = 500000;
    
    await mintTo(
      provider.connection,
      payer.payer,
      quoteMint,
      programTreasuryKeypair.publicKey,
      payer.publicKey,
      mintAmount
    );

    const treasuryBalance = await provider.connection.getTokenAccountBalance(programTreasuryKeypair.publicKey);
    console.log(`✅ Funded treasury: ${programTreasuryKeypair.publicKey.toString()} Balance: ${treasuryBalance.value.amount}`);
    
    assert.equal(treasuryBalance.value.amount, mintAmount.toString());
  });

  it("Cranks a single distribution page", async () => {
    const mockStreams = [];
    const remainingAccounts = [];

    for (let i = 0; i < 3; i++) {
      const streamKeypair = Keypair.generate();
      
      const investorAta = await getOrCreateAssociatedTokenAccount(
        provider.connection,
        payer.payer,
        quoteMint,
        payer.publicKey
      );

      mockStreams.push({
        stream: streamKeypair.publicKey,
        ata: investorAta.address
      });

      remainingAccounts.push(
        { pubkey: streamKeypair.publicKey, isSigner: false, isWritable: false },
        { pubkey: investorAta.address, isSigner: false, isWritable: true }
      );
    }

    const tx = await program.methods
      .crankDistribute(
        bumps.investorFeeOwner,
        new anchor.BN(0),
        true
      )
      .accounts({
        cranker: payer.publicKey,
        vault: vault.publicKey,
        policy: policyPda,
        progress: progressPda,
        investorFeePosOwnerPda: investorFeeOwnerPda,
        honoraryPosition: Keypair.generate().publicKey,
        programQuoteTreasury: programTreasuryKeypair.publicKey,
        creatorQuoteAta: creatorQuoteAta,
        pool: Keypair.generate().publicKey,
        poolQuoteMint: quoteMint,
        poolBaseMint: baseMint,
        cpAmmProgram: Keypair.generate().publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts(remainingAccounts)
      .preInstructions([
        anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({
          units: 300000,
        }),
      ])
      .signers([payer.payer])
      .rpc();

    console.log("✅ Cranked page 0, tx:", tx);
  });
});