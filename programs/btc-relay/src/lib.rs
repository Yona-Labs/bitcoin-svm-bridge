use anchor_lang::prelude::*;
use instructions::*;
use events::*;
use errors::*;
use structs::*;
use bitcoin::Transaction;
use bitcoin::consensus::Decodable;
use bitcoin::hex::FromHex;

mod arrayutils;
mod utils;
mod instructions;
mod events;
mod errors;
mod structs;
mod state;

declare_id!("De2dsY5K3DXBDNzKUjE6KguVP5JUhveKNpMVRmRkazff");

#[program]
pub mod btc_relay {
    use crate::state::DepositState;
    use super::*;

    //Initializes the program with the initial blockheader,
    // this can be any past blockheader with high enough confirmations to be sure it doesn't get re-orged.
    pub fn initialize(
        ctx: Context<Initialize>,
        data: BlockHeader,
        block_height: u32,
        chain_work: [u8; 32],
        last_diff_adjustment: u32,
        prev_block_timestamps: [u32; 10]
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
        
            prev_block_timestamps
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

    //Submit new main chain blockheaders
    pub fn submit_block_headers(ctx: Context<SubmitBlockHeaders>, data: Vec<BlockHeader>, commited_header: CommittedBlockHeader) -> Result<()> {
        require!(
            !data.is_empty(),
            RelayErrorCode::NoHeaders
        );
        
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
            //Prev block hash matches
            require!(
                last_block_hash == header.reversed_prev_blockhash,
                RelayErrorCode::PrevBlock
            );

            block_height+=1;

            last_block_hash = utils::verify_header(
                header,
                &mut last_commited_header,
                &ctx.remaining_accounts[block_cnt],
                &ctx.accounts.signer,
                ctx.program_id
            )?;
            
            //Compute commit hash
            block_commit_hash = last_commited_header.get_commit_hash()?;

            //Store and emit
            main_state.store_block_commitment(block_height, block_commit_hash);
            emit!(StoreHeader {
                block_hash: last_block_hash,
                commit_hash: block_commit_hash,
                header: last_commited_header
            });
        }

        //Update globals
        main_state.last_diff_adjustment = last_commited_header.last_diff_adjustment;
        main_state.block_height = block_height;
        main_state.chain_work = last_commited_header.chain_work;
        main_state.tip_commit_hash = block_commit_hash;
        main_state.tip_block_hash = last_block_hash;

        Ok(())
    }

    //Submit new headers forking the chain at some point in the past,
    // only allows submission of up to 7 blockheaders, due to Solana tx size limitation
    pub fn submit_short_fork_headers(ctx: Context<SubmitShortForkHeaders>, data: Vec<BlockHeader>, commited_header: CommittedBlockHeader) -> Result<()> {
        require!(
            !data.is_empty(),
            RelayErrorCode::NoHeaders
        );
        
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

            block_height+=1;

            last_block_hash = utils::verify_header(
                header,
                &mut last_commited_header,
                &ctx.remaining_accounts[block_cnt],
                &ctx.accounts.signer,
                ctx.program_id
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

        //Verify if fork chain's work exceeded main chain's work
        require!(
            arrayutils::gt_arr(last_commited_header.chain_work, main_state.chain_work),
            RelayErrorCode::ForkTooShort
        );

        //Update globals
        main_state.last_diff_adjustment = last_commited_header.last_diff_adjustment;
        main_state.block_height = block_height;
        main_state.chain_work = last_commited_header.chain_work;
        main_state.tip_commit_hash = block_commit_hash;
        main_state.tip_block_hash = last_block_hash;
        main_state.fork_counter = fork_id+1;

        Ok(())
    }

    //Submit new headers forking the chain at some point in the past,
    // this stores the new fork's blockheaders in an intermediary fork PDA,
    // allowing forks of >7 blocks, as soon as the fork chain's work exceeds
    // the main chain's work, the main chain is overwritten and fork PDA closed
    pub fn submit_fork_headers(ctx: Context<SubmitForkHeaders>, data: Vec<BlockHeader>, commited_header: CommittedBlockHeader, fork_id: u64, init: bool) -> Result<()> {
        require!(
            !data.is_empty(),
            RelayErrorCode::NoHeaders
        );
        
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
                init == (fork_state.initialized==0),
                RelayErrorCode::ErrInit
            );

            let main_state = &mut ctx.accounts.main_state.load_mut()?;

            let commit_hash = commited_header.get_commit_hash()?;

            let mut block_height = commited_header.blockheight;

            if fork_state.initialized==0 {
                //Has to use new fork_id from the fork_counter
                require!(
                    main_state.fork_counter == fork_id,
                    RelayErrorCode::InvalidForkId
                );

                main_state.fork_counter = fork_id+1;

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

                block_height+=1;

                last_block_hash = utils::verify_header(header, &mut last_commited_header, &ctx.remaining_accounts[block_cnt], &ctx.accounts.signer, ctx.program_id)?;
                
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
                    main_state.store_block_commitment(start_height+1+i, fork_state.block_commitments[i as usize]);
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
            ctx.accounts.fork_state.close(ctx.accounts.signer.to_account_info())?;
            msg!("Account closed");
        }

        Ok(())
    }

    //Used to close the fork PDA
    pub fn close_fork_account(_ctx: Context<CloseForkAccount>, _fork_id: u64) -> Result<()> {
        Ok(())
    }

    //Verifies transaction block inclusion proof, requiring certain amount of confirmations
    //Can be called as a CPI or a standalone instruction, that gets executed
    // before the instructions that depend on transaction verification
    pub fn verify_transaction(ctx: Context<VerifyTransaction>, tx_bytes: Vec<u8>, confirmations: u32, tx_index: u32, reversed_merkle_proof: Vec<[u8; 32]>, commited_header: CommittedBlockHeader) -> Result<()> {
        #[cfg(feature = "mocked")]
        {
            return Ok(());
        }

        #[cfg(not(feature = "mocked"))]
        {
            let block_height = commited_header.blockheight;

            let bitcoin_tx = Transaction::consensus_decode(&mut tx_bytes.as_slice()).unwrap();

            let lamports = ctx.accounts.main_state.to_account_info().lamports;
            msg!("Got lamports {}", **lamports.borrow());
            msg!("Bitcoin tx output amount {}", bitcoin_tx.output[0].value);

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

            let computed_merkle = utils::compute_merkle(bitcoin_tx.compute_txid().as_ref(), tx_index, &reversed_merkle_proof);

            require!(
                computed_merkle == commited_header.header.merkle_root,
                RelayErrorCode::MerkleRoot
            );

            /*
            let big_tx_hex = "020000000001141a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40900000017160014b8b72c5943b2cfd9de7d92f48e9fcad01d055de7feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40f000000171600140c532e8594f510da8b0d387f563934861575e844feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40700000017160014d07cba9645f2814af5ed30aef767234ddda9ac13feffffff3115e2b995e8a8f6ff045671e932f151b8df53c5f13ba7dcf03beb5cfaa1eec00000000000feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40f000000171600140c532e8594f510da8b0d387f563934861575e844feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40900000017160014b8b72c5943b2cfd9de7d92f48e9fcad01d055de7feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd405000000171600142aa38dc9e745450410da5bc9350ee8d6d12e0c70feffffff147d565149cd432db0af1a6fe7540e20f2878897127358605046b2e63702e2220100000000feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40100000017160014a748dee29baef670e3264341b2e7aebdfdb3a644feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40e00000017160014995cd732c74a3446f1ad35bafbd8d6f79828ef04feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40300000017160014004bebeb0b79c94754f5458a3ed8a293370df7fbfeffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40e00000017160014995cd732c74a3446f1ad35bafbd8d6f79828ef04feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40a00000017160014e3a8924beabbfcf3a3350368567a5c972dbbf2e7feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b4040000001716001476e4db8a1d5c4c238775df63899f71dfda0197f4feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b4060000001716001437453e98e510a863dd06be9f96d0292ba24cdcf3feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b405000000171600142aa38dc9e745450410da5bc9350ee8d6d12e0c70feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40100000017160014a748dee29baef670e3264341b2e7aebdfdb3a644feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40800000017160014ae471e13840e2b5dcdec9c4a9b54ba11fe7b665cfeffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40b0000001716001412d0bf7a471fde32b6ab8bea44c08806b6b9d178feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40300000017160014004bebeb0b79c94754f5458a3ed8a293370df7fbfeffffff0283a90f0000000000160014b32309634f6145184542e93869b693178a2b3068c01a1f15000000001976a9140035fb9766613a5d60c9cb7e68b7031b93f9c0ce88ac02473044022078d264c3710e473478b440d8bce655972d71dac199040df71a667f4616fc3d3e02203266223a3202045e40f317c717a81767979d7e569acabb9d6f80dbdcc3f61c50012103ad7a479ae2c1a65e5a81284dae6e9192cbee296377d5df5ed33ceb4d3845568702473044022050fed0de3d27a061542f9e85f32e178b6cab49a40f5411236cc471343a30fe0f02205a962c3bfe55b46022a0c5c594c13d8c7b58162f78dfeb9c9daa6ee3fa42c8330121030f0ce1edbf552656ea2ff791d88f84df9d68023fcf000d6a7ee8ecd5dc1e3ed202473044022012e7f7ad4d6ea9cdce4b91a84f02dd5141d60e968acc250c9ebf5f974549c7b102201c8687048907f27bbd0e48a98c320b1629de70bfbcdebefffa8c5cfbf76c9082012103d79e1063b8b5bd169ac5b2791b57e19f43160c7e553baa0b243cb666d30c19f40247304402207d9665fca9d76ed7737f309dda016d767cdd2c6aadb9bff20e2d6c9b5534b6130220225293e7650f9b8fe3c761d20ae226aa24e92f124d677141d3b24c4610fb2006012102100a0ba10ca19d9bfac2c933d473adc78043d4bfeae96ded14b3fc19466f351a02473044022074cbe0c5b4efb62d9f4b917bd528541ec3a6bf0f8d21ba2e789c34368241d99002201e5cbfb855698ec5ace5672193ce78c239ff33cc2cbf6b546af9e5531132e7cf0121030f0ce1edbf552656ea2ff791d88f84df9d68023fcf000d6a7ee8ecd5dc1e3ed202473044022034b92d6d96db9347e61e8a915ab3a37a295af6f08b3a2de0c4192828dc4804d6022067efdd8c5b8197f2b890bb46894e71f3088b64f731bd8dd3ae08b4f1918a4ade012103ad7a479ae2c1a65e5a81284dae6e9192cbee296377d5df5ed33ceb4d3845568702473044022041bd9d222072584b908b065311fa4fb84f58adbd6a47b911f11549b029e4363c022043c8bb3e485959c3531f078e460aad3c66c515d13e16303446733beea2e93a4c012102b9faff46c4afbb0c7303f47ef5d711bd8da4663bc92fe8c7295e0352cecb657502473044022057ced8a248383ca77d0e3c14616100e7343dc3f12f825188730ae5b3ed0c9c5e0220676d06cbd7bc52a2feef4cb5cbac942c8b61832438ca8b29a59beb407250dfaf01210273402ba5ce7143bdd1b4b1063378cae9cda29dcd820f339cf38b15549cb4007e024730440220078968f47a2dec93b1a4920f9a81177d8ce61a723a7fb183675bae23e750e5da02202271ded859d28a9d58bcada4566ed55b9cc590143af131917633de4668ab7cdb012102e8dedb110d0812d3a769c6f0539b3bc2164d748d386433407d503c4d6f2c44770247304402205882941c239cbfd0ba7619198894340a243803396913637f45ef2ff4a6a96791022041aa555dfc56b43f2d27ccf8691370b316f0195835e5d542e95640d8a04a34e401210393c79d23ae89f461f039b895dfd6f365ef5cc89f1a3ef030382f80c2cbd84caa024730440220272a9d37449a4f417657c0e1e5d3a87d1f02478f1fc1ddce9ab7013231ccee050220252fd6e3bdfd3f5499df7f064cdc5c1406b097d300aa734c31c97e017a341b1d0121025389a45a00c6d57e6a61258e95d6f7413f2c23274f7f82a7164ec040f7bf73ab024730440220700e5220c44e0f68931f046b52aa2d1273d86b50b7b86f2b847b98785dd0aa0102207aa0a3657e7cb1169a64f0efc15f10296f335cdadef99b62856de1b990c71c1c01210393c79d23ae89f461f039b895dfd6f365ef5cc89f1a3ef030382f80c2cbd84caa0247304402202f1e547a5a43776c855d0c37ec4cdc812cd381069337e6e843f49f9b4b9c5b8902207a565110b4af1f67300d41cf2f499a8be5424af394cba8c0cc7b373e650eeced012102e5aea5c0d5acb437f8f2bc6e8a293e56fe7f021c3b63aa01e5d681d8660af2730247304402205e2bfddb2e3b1eb6e43718e6aaa9d823ba47e295c5d381ba20817ecf37a2a2e9022068c9cc98005df5bc8f13385feed2350af854aec4b0de94ae1abaaddefe1d5fe00121030c9daf8f58ccd1733de05574964eaf2810c5d6e2375dee0f49603151cf589e1d0247304402205eb0fab222b1f5d3adf54784fda4a555c5c8acc4151908f194bd1a4fb2f6ba540220370be1435d09b6c1d94297a6128fff677336877ad30994befbacdab941d6c35701210241202dcd1c9a451c4a212f88b10c2bdf2afab7bfbd14b76c2a9078c932dfffeb02473044022003d2cb7fe423c634caeec0d3d00085351041f83f33a77596d5c104283bae70aa022031affaf6564c24d946cec27419bfb5182dcab6be1e26fcf2aa334eb7d6027160012102b9faff46c4afbb0c7303f47ef5d711bd8da4663bc92fe8c7295e0352cecb6575024730440220156f5c439dbc5ec403fcde44e27804906051d717466cf057026d4fd86be5522802204c97b2c98e06d6ea14572c91d5da9f89c4bf7d37df07badcc60b527b27e047e1012102e8dedb110d0812d3a769c6f0539b3bc2164d748d386433407d503c4d6f2c4477024730440220533759a56d6f141f00ecc3f889f5c839c340fbc015cdec5cc7595082c00e702802200e1829c99288c487cfb5af72e909e0f8d30d65c027207d1b9f59df7a728887dc012103a261fa4d379512d74d8eb14a7f797f694bbbda5ff3dcc05a08a55be7dd2b2e000247304402203fa51d8f3dd4cd19162760384036f75a15c56e432cb73a3c7de29f0a0359d48e0220679b4fa29004ebf1f04f924f7af859abc98a89a5476a57c7ad8cc9a6cdd1a437012102d78e2a75a028716ab8d9eb64833f546241fbe0ce54051fd534436135414d12c60247304402200fb88079388afdc469bc428bb7cff08cd3bf1ad9d2a0504eba6d6d23256959ef02204182acf8104ed927a6a96a8384c60a35ae45a10d04a25a3741830316014404e20121025389a45a00c6d57e6a61258e95d6f7413f2c23274f7f82a7164ec040f7bf73abadba0b00";
            let tx_bytes = Vec::<u8>::from_hex(big_tx_hex).unwrap();

            let big_tx = Transaction::consensus_decode(&mut tx_bytes.as_slice()).unwrap();
            msg!("Big tx output {}", big_tx.output[0].value);
            msg!("Big tx ID {:02x}", big_tx.compute_txid());

            match Transaction::consensus_decode(&mut tx_bytes.as_slice()) {
                Ok(tx) => msg!("Big tx ID {:02x}", tx.compute_txid()),
                Err(e) => msg!("Error {:?}", e),
            };
            */

            Ok(())
        }
    }

    //Verifies blockheight of the main chain
    //Supports many operators
    // 0 - blockheight has to be < value
    // 1 - blockheight has to be <= value
    // 2 - blockheight has to be > value
    // 3 - blockheight has to be >= value
    // 4 - blockheight has to be == value
    //This can be called a standalone instruction, that gets executed
    // before the instructions that depend on bitcoin relay having a specific blockheight
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
                    _ => false
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
                    _ => false
                },
                RelayErrorCode::InvalidBlockheight
            );

            Ok(())
        }
    }

    #[derive(Accounts)]
    pub struct Deposit<'info> {
        /// The user account initiating the deposit.
        #[account(mut)]
        pub user: Signer<'info>,
        /// The program's account to receive the deposit. This should be a derived PDA (Program Derived Address).
        #[account(init, seeds = [b"solana_deposit".as_ref()], bump, payer = user, space = 10240)]
        pub deposit_account: AccountLoader<'info, DepositState>,
        pub system_program: Program<'info, System>,
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

        // Transfer half of it back immediately
        **ctx.accounts.deposit_account.as_ref().try_borrow_mut_lamports()? -= amount / 2;
        **ctx.accounts.user.try_borrow_mut_lamports()? += amount / 2;

        Ok(())
    }
}
