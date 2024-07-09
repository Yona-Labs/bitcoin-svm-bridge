use anchor_lang::prelude::*;
use bitcoin::consensus::Decodable;
use bitcoin::Transaction;

use errors::*;
use events::*;
use instructions::*;
use structs::*;

mod arrayutils;
mod errors;
pub mod events;
mod instructions;
pub mod state;
pub mod structs;
pub mod utils;

declare_id!("HY7CpkPjh5FgcU22RmEpqD6T5ZNWSgmwiTDYE5yHRfmf");

#[program]
pub mod btc_relay {
    use super::*;
    use crate::utils::{bridge_deposit_script, BITCOIN_DEPOSIT_PUBKEY};
    use bitcoin::address::Address;
    use bitcoin::hashes::hash160::Hash as Hash160;
    use bitcoin::hashes::Hash;
    use bitcoin::hex::FromHex;
    use bitcoin::Network;

    // Initializes the program with the initial block header,
    // this can be any past block header with high enough confirmations to be sure it doesn't get re-orged.
    pub fn initialize(
        ctx: Context<Initialize>,
        data: BlockHeader,
        block_height: u32,
        chain_work: [u8; 32],
        last_diff_adjustment: u32,
        prev_block_timestamps: [u32; 10],
    ) -> Result<()> {
        let main_state = &mut ctx.accounts.main_state.load_init()?;

        main_state.last_diff_adjustment = last_diff_adjustment;
        main_state.block_height = block_height;
        main_state.chain_work = chain_work;

        main_state.fork_counter = 0;

        let commited_header = CommittedBlockHeader {
            chain_work,

            header: data,

            last_diff_adjustment,
            blockheight: block_height,

            prev_block_timestamps,
        };

        let hash_result = commited_header.get_commit_hash()?;
        let block_hash = data.get_block_hash()?;

        main_state.block_commitments[0] = hash_result;

        main_state.start_height = block_height;
        main_state.total_blocks = 1;

        main_state.tip_block_hash = block_hash;
        main_state.tip_commit_hash = hash_result;

        emit!(StoreHeader {
            block_hash,
            commit_hash: hash_result,
            header: commited_header
        });

        Ok(())
    }

    // Submit new main chain block headers
    pub fn submit_block_headers(
        ctx: Context<SubmitBlockHeaders>,
        data: Vec<BlockHeader>,
        commited_header: CommittedBlockHeader,
    ) -> Result<()> {
        require!(!data.is_empty(), RelayErrorCode::NoHeaders);

        require!(
            data.len() == ctx.remaining_accounts.len(),
            RelayErrorCode::InvalidRemainingAccounts
        );

        //Verify commited header was indeed committed
        let commit_hash = commited_header.get_commit_hash()?;
        let main_state = &mut ctx.accounts.main_state.load_mut()?;
        let main_state_tip = main_state.get_commitment(main_state.block_height);
        require!(
            commit_hash == main_state_tip,
            RelayErrorCode::PrevBlockCommitment
        );

        let mut last_commited_header = commited_header;
        let mut last_block_hash: [u8; 32] = commited_header.header.get_block_hash()?;
        let mut block_height = main_state.block_height;
        let mut block_commit_hash: [u8; 32] = [0; 32];

        for (block_cnt, header) in data.iter().enumerate() {
            // Prev block hash matches
            require!(
                last_block_hash == header.reversed_prev_blockhash,
                RelayErrorCode::PrevBlock
            );

            block_height += 1;

            last_block_hash = utils::verify_header(
                header,
                &mut last_commited_header,
                &ctx.remaining_accounts[block_cnt],
                &ctx.accounts.signer,
                ctx.program_id,
            )?;

            // Compute commit hash
            block_commit_hash = last_commited_header.get_commit_hash()?;

            // Store and emit
            main_state.store_block_commitment(block_height, block_commit_hash);
            emit!(StoreHeader {
                block_hash: last_block_hash,
                commit_hash: block_commit_hash,
                header: last_commited_header
            });
        }

        // Update globals
        main_state.last_diff_adjustment = last_commited_header.last_diff_adjustment;
        main_state.block_height = block_height;
        main_state.chain_work = last_commited_header.chain_work;
        main_state.tip_commit_hash = block_commit_hash;
        main_state.tip_block_hash = last_block_hash;

        Ok(())
    }

    // Submit new headers forking the chain at some point in the past,
    // only allows submission of up to 7 blockheaders, due to Solana tx size limitation
    pub fn submit_short_fork_headers(
        ctx: Context<SubmitShortForkHeaders>,
        data: Vec<BlockHeader>,
        commited_header: CommittedBlockHeader,
    ) -> Result<()> {
        require!(!data.is_empty(), RelayErrorCode::NoHeaders);

        require!(
            data.len() == ctx.remaining_accounts.len(),
            RelayErrorCode::InvalidRemainingAccounts
        );

        //Verify commited header was indeed committed
        let commit_hash = commited_header.get_commit_hash()?;

        let main_state = &mut ctx.accounts.main_state.load_mut()?;

        require!(
            commit_hash == main_state.get_commitment(commited_header.blockheight),
            RelayErrorCode::PrevBlockCommitment
        );

        let fork_id = main_state.fork_counter;
        let mut last_commited_header = commited_header;
        let mut last_block_hash: [u8; 32] = commited_header.header.get_block_hash()?;
        let mut block_height = commited_header.blockheight;

        let mut block_commit_hash: [u8; 32] = [0; 32];

        for (block_cnt, header) in data.iter().enumerate() {
            //Prev block hash matches
            require!(
                last_block_hash == header.reversed_prev_blockhash,
                RelayErrorCode::PrevBlock
            );

            block_height += 1;

            last_block_hash = utils::verify_header(
                header,
                &mut last_commited_header,
                &ctx.remaining_accounts[block_cnt],
                &ctx.accounts.signer,
                ctx.program_id,
            )?;

            //Compute commit hash
            block_commit_hash = last_commited_header.get_commit_hash()?;

            //Store and emit
            main_state.store_block_commitment(block_height, block_commit_hash);
            emit!(StoreFork {
                fork_id,
                block_hash: last_block_hash,
                commit_hash: block_commit_hash,
                header: last_commited_header
            });
        }

        // Verify if fork chain's work exceeded main chain's work
        require!(
            arrayutils::gt_arr(last_commited_header.chain_work, main_state.chain_work),
            RelayErrorCode::ForkTooShort
        );

        // Update globals
        main_state.last_diff_adjustment = last_commited_header.last_diff_adjustment;
        main_state.block_height = block_height;
        main_state.chain_work = last_commited_header.chain_work;
        main_state.tip_commit_hash = block_commit_hash;
        main_state.tip_block_hash = last_block_hash;
        main_state.fork_counter = fork_id + 1;

        Ok(())
    }

    // Submit new headers forking the chain at some point in the past,
    // this stores the new fork's blockheaders in an intermediary fork PDA,
    // allowing forks of >7 blocks, as soon as the fork chain's work exceeds
    // the main chain's work, the main chain is overwritten and fork PDA closed
    pub fn submit_fork_headers(
        ctx: Context<SubmitForkHeaders>,
        data: Vec<BlockHeader>,
        commited_header: CommittedBlockHeader,
        fork_id: u64,
        init: bool,
    ) -> Result<()> {
        require!(!data.is_empty(), RelayErrorCode::NoHeaders);

        require!(
            data.len() == ctx.remaining_accounts.len(),
            RelayErrorCode::InvalidRemainingAccounts
        );

        let mut close = false;

        {
            let load_res = if init {
                ctx.accounts.fork_state.load_init()
            } else {
                ctx.accounts.fork_state.load_mut()
            };

            let fork_state = &mut load_res?;

            //Only yet uninitialized PDA can be initialized
            require!(
                init == (fork_state.initialized == 0),
                RelayErrorCode::ErrInit
            );

            let main_state = &mut ctx.accounts.main_state.load_mut()?;

            let commit_hash = commited_header.get_commit_hash()?;

            let mut block_height = commited_header.blockheight;

            if fork_state.initialized == 0 {
                //Has to use new fork_id from the fork_counter
                require!(
                    main_state.fork_counter == fork_id,
                    RelayErrorCode::InvalidForkId
                );

                main_state.fork_counter = fork_id + 1;

                //Verify commited header was indeed committed,
                // the latest common ancestor block, right before the fork occurred
                require!(
                    commit_hash == main_state.get_commitment(commited_header.blockheight),
                    RelayErrorCode::PrevBlockCommitment
                );

                fork_state.initialized = 1;
                fork_state.start_height = block_height;
            } else {
                //Verify commited header was indeed committed in the fork state
                require!(
                    commit_hash == fork_state.tip_commit_hash,
                    RelayErrorCode::PrevBlockCommitment
                );
            }

            let mut last_commited_header = commited_header;
            let mut last_block_hash: [u8; 32] = commited_header.header.get_block_hash()?;

            let mut block_commit_hash: [u8; 32] = [0; 32];

            for (block_cnt, header) in data.iter().enumerate() {
                //Prev block hash matches
                require!(
                    last_block_hash == header.reversed_prev_blockhash,
                    RelayErrorCode::PrevBlock
                );

                block_height += 1;

                last_block_hash = utils::verify_header(
                    header,
                    &mut last_commited_header,
                    &ctx.remaining_accounts[block_cnt],
                    &ctx.accounts.signer,
                    ctx.program_id,
                )?;

                //Compute commit hash
                block_commit_hash = last_commited_header.get_commit_hash()?;

                //Store and emit
                fork_state.store_block_commitment(block_commit_hash);
                emit!(StoreFork {
                    fork_id,
                    block_hash: last_block_hash,
                    commit_hash: block_commit_hash,
                    header: last_commited_header
                });
            }

            if arrayutils::gt_arr(last_commited_header.chain_work, main_state.chain_work) {
                //Successful fork, fork's work exceeded main chain's work

                msg!("Successful fork...");

                //Overwrite block commitments in main chain
                let start_height = fork_state.start_height;
                for i in 0..fork_state.length {
                    main_state.store_block_commitment(
                        start_height + 1 + i,
                        fork_state.block_commitments[i as usize],
                    );
                }

                msg!("Commitments stored...");

                //Update main state with fork's state
                main_state.last_diff_adjustment = last_commited_header.last_diff_adjustment;
                main_state.block_height = block_height;
                main_state.chain_work = last_commited_header.chain_work;
                main_state.tip_commit_hash = block_commit_hash;
                main_state.tip_block_hash = last_block_hash;

                msg!("Main state updated");

                //Close the fork PDA
                close = true;

                emit!(ChainReorg {
                    fork_id,
                    start_height,
                    tip_block_hash: last_block_hash,
                    tip_commit_hash: block_commit_hash
                });
            } else {
                //Fork still needs to be appended
                fork_state.tip_block_hash = last_block_hash;
                fork_state.tip_commit_hash = block_commit_hash;
            }
        }

        if close {
            ctx.accounts
                .fork_state
                .close(ctx.accounts.signer.to_account_info())?;
            msg!("Account closed");
        }

        Ok(())
    }

    // Used to close the fork PDA
    pub fn close_fork_account(_ctx: Context<CloseForkAccount>, _fork_id: u64) -> Result<()> {
        Ok(())
    }

    // Verifies transaction block inclusion proof, requiring certain amount of confirmations
    // Can be called as a CPI or a standalone instruction, that gets executed
    // before the instructions that depend on transaction verification
    pub fn verify_small_tx(
        ctx: Context<VerifyTransaction>,
        tx_bytes: Vec<u8>,
        confirmations: u32,
        tx_index: u32,
        reversed_merkle_proof: Vec<[u8; 32]>,
        commited_header: CommittedBlockHeader,
    ) -> Result<()> {
        #[cfg(feature = "mocked")]
        {
            return Ok(());
        }

        #[cfg(not(feature = "mocked"))]
        {
            let block_height = commited_header.blockheight;

            let main_state = ctx.accounts.main_state.load()?;

            require!(
                main_state.block_height - block_height + 1 >= confirmations,
                RelayErrorCode::BlockConfirmations
            );

            let commit_hash = commited_header.get_commit_hash()?;
            require!(
                commit_hash == main_state.get_commitment(block_height),
                RelayErrorCode::PrevBlockCommitment
            );

            let bitcoin_tx = Transaction::consensus_decode(&mut tx_bytes.as_slice()).unwrap();

            let bridge_pubkey: [u8; 33] = FromHex::from_hex(BITCOIN_DEPOSIT_PUBKEY).unwrap();
            let pubkey_hash = Hash160::hash(&bridge_pubkey);
            let expected_script = bridge_deposit_script(
                ctx.accounts.mint_receiver.key.to_bytes(),
                pubkey_hash.to_byte_array(),
            );

            let expected_address = Address::p2wsh(expected_script.as_script(), Network::Regtest);
            let actual_address = Address::from_script(
                bitcoin_tx.output[0].script_pubkey.as_script(),
                Network::Regtest,
            )
            .unwrap();

            require!(
                actual_address == expected_address,
                RelayErrorCode::InvalidDepositAddress
            );

            let computed_merkle = utils::compute_merkle(
                bitcoin_tx.compute_txid().as_ref(),
                tx_index,
                &reversed_merkle_proof,
            );

            require!(
                computed_merkle == commited_header.header.merkle_root,
                RelayErrorCode::MerkleRoot
            );

            let sol_amount = bitcoin_tx.output[0].value.to_sat() * 10;

            **ctx
                .accounts
                .deposit_account
                .as_ref()
                .try_borrow_mut_lamports()? -= sol_amount;
            **ctx.accounts.mint_receiver.try_borrow_mut_lamports()? += sol_amount;
            Ok(())
        }
    }

    // Verifies block height of the main chain
    // Supports many operators
    // 0 - block height has to be < value
    // 1 - block height has to be <= value
    // 2 - block height has to be > value
    // 3 - block height has to be >= value
    // 4 - block height has to be == value
    // This can be called a standalone instruction, that gets executed
    // before the instructions that depend on bitcoin relay having a specific block height
    pub fn block_height(ctx: Context<BlockHeight>, value: u32, operation: u32) -> Result<()> {
        #[cfg(feature = "mocked")]
        {
            require!(
                match operation {
                    0 => 845414 < value,
                    1 => 845414 <= value,
                    2 => 845414 > value,
                    3 => 845414 >= value,
                    4 => 845414 == value,
                    _ => false,
                },
                RelayErrorCode::InvalidBlockheight
            );

            return Ok(());
        }

        #[cfg(not(feature = "mocked"))]
        {
            let main_state = ctx.accounts.main_state.load()?;
            let block_height = main_state.block_height;

            require!(
                match operation {
                    0 => block_height < value,
                    1 => block_height <= value,
                    2 => block_height > value,
                    3 => block_height >= value,
                    4 => block_height == value,
                    _ => false,
                },
                RelayErrorCode::InvalidBlockheight
            );

            Ok(())
        }
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        // Transfer SOL from the user to the program's account
        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.user.key,
            &ctx.accounts.deposit_account.as_ref().key,
            amount,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.user.to_account_info(),
                ctx.accounts.deposit_account.to_account_info(),
            ],
        )?;

        Ok(())
    }

    pub fn init_big_tx_verify(
        ctx: Context<InitBigTxVerify>,
        tx_id: [u8; 32],
        tx_size: u64,
        confirmations: u32,
        tx_index: u32,
        reversed_merkle_proof: Vec<[u8; 32]>,
        commited_header: CommittedBlockHeader,
    ) -> Result<()> {
        let block_height = commited_header.blockheight;

        let main_state = ctx.accounts.main_state.load()?;

        require!(
            main_state.block_height - block_height + 1 >= confirmations,
            RelayErrorCode::BlockConfirmations
        );

        let commit_hash = commited_header.get_commit_hash()?;
        require!(
            commit_hash == main_state.get_commitment(block_height),
            RelayErrorCode::PrevBlockCommitment
        );

        let computed_merkle = utils::compute_merkle(&tx_id, tx_index, &reversed_merkle_proof);

        msg!("Computed merkle {:?}", computed_merkle);
        msg!("Header root {:?}", commited_header.header.merkle_root);

        require!(
            computed_merkle == commited_header.header.merkle_root,
            RelayErrorCode::MerkleRoot
        );

        Ok(())
    }

    pub fn store_tx_bytes(
        ctx: Context<StoreTxBytes>,
        tx_id: [u8; 32],
        bytes: Vec<u8>,
    ) -> Result<()> {
        ctx.accounts.tx_account.tx_bytes.extend(bytes);
        Ok(())
    }

    pub fn finalize_tx_processing(ctx: Context<FinalizeTx>, tx_id: [u8; 32]) -> Result<()> {
        let bitcoin_tx =
            Transaction::consensus_decode(&mut ctx.accounts.tx_account.tx_bytes.as_slice())
                .unwrap();

        assert_eq!(tx_id, bitcoin_tx.compute_txid().as_ref());

        let sol_amount = bitcoin_tx.output[0].value.to_sat() * 10;
        msg!("Sol amount to transfer {}", sol_amount);

        **ctx
            .accounts
            .deposit_account
            .as_ref()
            .try_borrow_mut_lamports()? -= sol_amount;
        **ctx.accounts.user.try_borrow_mut_lamports()? += sol_amount;
        Ok(())
    }
}
