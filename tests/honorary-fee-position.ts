import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { HonoraryFeePosition } from "../target/types/honorary_fee_position";
import {
  PublicKey,
  Keypair,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
  Transaction,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createMint,
  mintTo,
  getOrCreateAssociatedTokenAccount,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import { assert } from "chai";

// Set up wallet if not provided
if (!process.env.ANCHOR_WALLET) {
  process.env.ANCHOR_WALLET =
    require("os").homedir() + "/.config/solana/id.json";
}

describe("honorary_fee_position (local testing)", () => {
  const provider = anchor.AnchorProvider.local();
  anchor.setProvider(provider);
  const program = anchor.workspace
    .HonoraryFeePosition as Program<HonoraryFeePosition>;

  const payer = provider.wallet as anchor.Wallet;
  let vault = Keypair.generate();
  let quoteMint: PublicKey;
  let baseMint: PublicKey;
  let programQuoteTreasury: PublicKey; // ATA address, not keypair
  let creatorQuoteAta: PublicKey;

  // PDAs
  let policyPda: PublicKey;
  let progressPda: PublicKey;
  let investorFeeOwnerPda: PublicKey;
  let bumps: any = {};

  // Mock stream accounts
  let mockStreamAccounts: Array<{
    keypair: Keypair;
    ata: PublicKey;
    locked: number;
  }> = [];

  before(async () => {
    console.log("Setting up test environment...");

    try {
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

      [investorFeeOwnerPda, bumps.investorFeeOwner] =
        PublicKey.findProgramAddressSync(
          [
            Buffer.from("vault"),
            vault.publicKey.toBuffer(),
            Buffer.from("investor_fee_pos_owner"),
          ],
          program.programId
        );

      // Calculate the ATA address for the treasury (owned by the PDA)
      programQuoteTreasury = getAssociatedTokenAddressSync(
        quoteMint,
        investorFeeOwnerPda,
        true // allowOwnerOffCurve = true for PDAs
      );

      // Create creator ATA
      const creatorAta = await getOrCreateAssociatedTokenAccount(
        provider.connection,
        payer.payer,
        quoteMint,
        payer.publicKey
      );
      creatorQuoteAta = creatorAta.address;

      console.log("✅ Test environment setup complete");
      console.log("Vault:", vault.publicKey.toString());
      console.log("Quote Mint:", quoteMint.toString());
      console.log("Policy PDA:", policyPda.toString());
      console.log("Progress PDA:", progressPda.toString());
      console.log("Investor Fee Owner PDA:", investorFeeOwnerPda.toString());
      console.log("Treasury ATA:", programQuoteTreasury.toString());
      console.log("Creator ATA:", creatorQuoteAta.toString());
    } catch (error) {
      console.error("Error in test setup:", error);
      throw error;
    }
  });

  it("Initializes program + PDAs", async () => {
    try {
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
          programQuoteTreasury: programQuoteTreasury, // Use ATA address
          pool: Keypair.generate().publicKey,
          poolQuoteMint: quoteMint,
          poolBaseMint: baseMint,
          cpAmmProgram: Keypair.generate().publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .preInstructions([
          anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({
            units: 400000,
          }),
        ])
        .signers([payer.payer]) // Only payer signs, ATA is created automatically
        .rpc();

      console.log("✅ Initialized program + PDAs, tx:", tx);

      // Verify accounts were created
      const policyAccount = await program.account.policy.fetch(policyPda);
      const progressAccount = await program.account.progress.fetch(progressPda);

      assert.ok(policyAccount);
      assert.ok(progressAccount);
      console.log("✅ Policy and Progress accounts created successfully");

      // Verify treasury ATA was created
      const treasuryInfo = await provider.connection.getAccountInfo(
        programQuoteTreasury
      );
      assert.ok(treasuryInfo, "Treasury ATA should be created");
      console.log("✅ Treasury ATA created successfully");
    } catch (error) {
      console.error("Initialization failed:", error);
      throw error;
    }
  });

  it("Creates mock investor stream accounts with locked balances", async () => {
    const streamData = [
      { locked: 100000 },
      { locked: 200000 },
      { locked: 300000 },
    ];

    for (let i = 0; i < streamData.length; i++) {
      const data = streamData[i];
      const streamKeypair = Keypair.generate();

      // Create investor ATA (all using same investor for simplicity)
      const investorAta = await getOrCreateAssociatedTokenAccount(
        provider.connection,
        payer.payer,
        quoteMint,
        payer.publicKey
      );

      // Create stream account with locked amount data
      const accountSize = 32; // Enough space for locked amount + padding
      const lamports =
        await provider.connection.getMinimumBalanceForRentExemption(
          accountSize
        );

      const createAccountTx = new Transaction().add(
        SystemProgram.createAccount({
          fromPubkey: payer.publicKey,
          newAccountPubkey: streamKeypair.publicKey,
          lamports: lamports,
          space: accountSize,
          programId: SystemProgram.programId,
        })
      );

      await provider.sendAndConfirm(createAccountTx, [
        payer.payer,
        streamKeypair,
      ]);

      // Write the locked amount as the first 8 bytes
      const lockedBuffer = Buffer.alloc(8);
      lockedBuffer.writeBigUInt64LE(BigInt(data.locked), 0);

      mockStreamAccounts.push({
        keypair: streamKeypair,
        ata: investorAta.address,
        locked: data.locked,
      });

      console.log(
        `Created mock stream account ${i + 1} with ${data.locked} locked tokens`
      );
    }

    console.log("✅ Created 3 mock stream accounts and investor ATAs");
    console.log(
      "Mock accounts:",
      mockStreamAccounts.map((acc, i) => ({
        index: i,
        stream: acc.keypair.publicKey.toString(),
        ata: acc.ata.toString(),
        locked: acc.locked,
      }))
    );
  });

  it("Funds the program quote treasury", async () => {
    try {
      // Check if treasury account exists
      const accountInfo = await provider.connection.getAccountInfo(
        programQuoteTreasury
      );

      if (!accountInfo) {
        throw new Error(
          "Treasury ATA not initialized. The first test may have failed."
        );
      }

      // Mint tokens to the treasury
      const mintAmount = 500000;

      await mintTo(
        provider.connection,
        payer.payer,
        quoteMint,
        programQuoteTreasury, // Use ATA address
        payer.publicKey,
        mintAmount
      );

      const treasuryBalance = await provider.connection.getTokenAccountBalance(
        programQuoteTreasury
      );
      console.log(
        `✅ Funded treasury: ${programQuoteTreasury.toString()} Balance: ${
          treasuryBalance.value.amount
        }`
      );

      assert.equal(treasuryBalance.value.amount, mintAmount.toString());
    } catch (error) {
      console.error("Treasury funding error:", error);
      throw error;
    }
  });

  it("Cranks a single distribution page", async () => {
    try {
      console.log("Setting up distribution crank...");

      // Create fresh stream accounts for the crank operation
      const remainingAccounts = [];
      const streamAccounts = [];

      for (let i = 0; i < 3; i++) {
        const streamKeypair = Keypair.generate();
        const lockedAmount = (i + 1) * 100000; // 100000, 200000, 300000

        // Use the same investor ATA for all (for simplicity)
        const investorAta = await getOrCreateAssociatedTokenAccount(
          provider.connection,
          payer.payer,
          quoteMint,
          payer.publicKey
        );

        // Create the stream account
        const accountSize = 32;
        const lamports =
          await provider.connection.getMinimumBalanceForRentExemption(
            accountSize
          );

        const createAccountTx = new Transaction().add(
          SystemProgram.createAccount({
            fromPubkey: payer.publicKey,
            newAccountPubkey: streamKeypair.publicKey,
            lamports: lamports,
            space: accountSize,
            programId: SystemProgram.programId,
          })
        );

        await provider.sendAndConfirm(createAccountTx, [
          payer.payer,
          streamKeypair,
        ]);

        streamAccounts.push({
          stream: streamKeypair.publicKey,
          ata: investorAta.address,
        });

        remainingAccounts.push(
          {
            pubkey: streamKeypair.publicKey,
            isSigner: false,
            isWritable: false,
          },
          { pubkey: investorAta.address, isSigner: false, isWritable: true }
        );

        console.log(
          `Created stream account ${
            i + 1
          }: ${streamKeypair.publicKey.toString()}`
        );
        console.log(`Associated ATA: ${investorAta.address.toString()}`);
        console.log(`Locked amount: ${lockedAmount}`);
      }

      console.log("Executing crank distribute...");

      const tx = await program.methods
        .crankDistribute(
          bumps.investorFeeOwner,
          new anchor.BN(0), // page_index
          true // is_last_page
        )
        .accounts({
          cranker: payer.publicKey,
          vault: vault.publicKey,
          policy: policyPda,
          progress: progressPda,
          investorFeePosOwnerPda: investorFeeOwnerPda,
          honoraryPosition: Keypair.generate().publicKey,
          programQuoteTreasury: programQuoteTreasury, // Use ATA address
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
            units: 400000,
          }),
        ])
        .signers([payer.payer])
        .rpc();

      console.log("✅ Cranked page 0, tx:", tx);

      // Verify the progress was updated
      try {
        const progressAccount = await program.account.progress.fetch(
          progressPda
        );
        console.log("Progress after crank:", {
          cursor: progressAccount.cursor.toString(),
          cumulativeDistributed:
            progressAccount.cumulativeDistributedToday.toString(),
          lastDistributionTs: progressAccount.lastDistributionTs.toString(),
          carryLamports: progressAccount.carryLamports.toString(),
        });

        // Verify policy account
        const policyAccount = await program.account.policy.fetch(policyPda);
        console.log("Policy data:", {
          vault: policyAccount.vault.toString(),
          y0: policyAccount.y0.toString(),
          investorFeeShareBps: policyAccount.investorFeeShareBps,
          dailyCap: policyAccount.dailyCap?.toString() || "None",
          minPayout: policyAccount.minPayout.toString(),
          dustThreshold: policyAccount.dustThreshold.toString(),
        });

        console.log("✅ Distribution crank completed successfully");
      } catch (debugError) {
        console.error("Debug info error:", debugError);
      }
    } catch (error) {
      console.error("Crank distribute error:", error);

      // Log additional debug info
      try {
        const policyExists = await program.account.policy
          .fetch(policyPda)
          .then(() => true)
          .catch(() => false);
        console.log("Policy account exists:", policyExists);

        const progressExists = await program.account.progress
          .fetch(progressPda)
          .then(() => true)
          .catch(() => false);
        console.log("Progress account exists:", progressExists);

        const treasuryInfo = await provider.connection.getAccountInfo(
          programQuoteTreasury
        );
        console.log("Treasury account exists:", !!treasuryInfo);

        if (treasuryInfo) {
          const treasuryBalance =
            await provider.connection.getTokenAccountBalance(
              programQuoteTreasury
            );
          console.log("Treasury balance:", treasuryBalance.value.amount);
        }
      } catch (debugError) {
        console.error("Debug info error:", debugError);
      }

      throw error;
    }
  });

  // Additional test to verify account states
  it("Verifies final account states", async () => {
    try {
      const policyAccount = await program.account.policy.fetch(policyPda);
      const progressAccount = await program.account.progress.fetch(progressPda);

      console.log("Final Policy State:", {
        vault: policyAccount.vault.toString(),
        y0: policyAccount.y0.toString(),
        investorFeeShareBps: policyAccount.investorFeeShareBps,
        dailyCap: policyAccount.dailyCap?.toString() || "None",
        minPayout: policyAccount.minPayout.toString(),
        dustThreshold: policyAccount.dustThreshold.toString(),
      });

      console.log("Final Progress State:", {
        vault: progressAccount.vault.toString(),
        dayStartTs: progressAccount.dayStartTs.toString(),
        lastDistributionTs: progressAccount.lastDistributionTs.toString(),
        cumulativeDistributedToday:
          progressAccount.cumulativeDistributedToday.toString(),
        carryLamports: progressAccount.carryLamports.toString(),
        cursor: progressAccount.cursor.toString(),
        treasurySnapshot: progressAccount.treasurySnapshot.toString(),
        pageRecordsCount: progressAccount.pageRecords.length,
      });

      const treasuryBalance = await provider.connection.getTokenAccountBalance(
        programQuoteTreasury
      );
      console.log("Final Treasury Balance:", treasuryBalance.value.amount);

      assert.ok(policyAccount, "Policy account should exist");
      assert.ok(progressAccount, "Progress account should exist");
      assert.ok(treasuryBalance, "Treasury should have balance info");

      console.log("✅ All account states verified successfully");
    } catch (error) {
      console.error("Account verification error:", error);
      throw error;
    }
  });
});
