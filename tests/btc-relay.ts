import * as anchor from "@project-serum/anchor";
import { Program } from "@project-serum/anchor";
import { BtcRelay } from "../target/types/btc_relay";
const { SystemProgram, ComputeBudgetProgram } = anchor.web3;
import { BN } from "bn.js";
import { randomBytes, createHash } from "crypto";
import TransactionFactory from "@project-serum/anchor/dist/cjs/program/namespace/transaction";

const commitment: anchor.web3.Commitment = "confirmed";

const mainStateSeed = "state";
const headerSeed = "header";
const PRUNING_FACTOR = 250;
const accountSize = 8+4+4+4+32+8+4+(PRUNING_FACTOR*32);

function dblSha256(data: Buffer) {  
  const hash1 = createHash("sha256").update(data).digest();
  return createHash("sha256").update(hash1).digest();
}

const provider = anchor.AnchorProvider.local();
const program = anchor.workspace.BtcRelay as Program<BtcRelay>;

function programPaidBy(payer: anchor.web3.Keypair): anchor.Program {
  const newProvider = new anchor.AnchorProvider(provider.connection, new anchor.Wallet(payer), {});
  return new anchor.Program(program.idl as anchor.Idl, program.programId, newProvider)
}

const commitedHeader = {
  "chainWork": [
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      60,
      39,
      152,
      233,
      4,
      108,
      127,
      40,
      200,
      233,
      11,
      98
  ],
  "header": {
      "version": 541065220,
      "reversedPrevBlockhash": [
          139,
          67,
          181,
          184,
          213,
          211,
          105,
          125,
          12,
          246,
          47,
          248,
          73,
          161,
          241,
          44,
          181,
          146,
          16,
          138,
          254,
          221,
          7,
          0,
          0,
          0,
          0,
          0,
          0,
          0,
          0,
          0
      ],
      "merkleRoot": [
          249,
          15,
          28,
          171,
          45,
          98,
          137,
          134,
          153,
          170,
          220,
          149,
          126,
          248,
          16,
          219,
          250,
          39,
          227,
          139,
          56,
          10,
          175,
          63,
          31,
          24,
          40,
          28,
          243,
          155,
          125,
          33
      ],
      "timestamp": 1671837609,
      "nbits": 386397584,
      "nonce": 3268247420
  },
  "lastDiffAdjustment": 1671463076,
  "blockheight": 768686,
  "prevBlockTimestamps": [
      1671463076,
      1671463076,
      1671463076,
      1671463076,
      1671463076,
      1671463076,
      1671463076,
      1671463076,
      1671463076,
      1671463076
  ]
};

const signer = anchor.web3.Keypair.generate();

const rawHeader = Buffer.from("040040208b43b5b8d5d3697d0cf62ff849a1f12cb592108afedd07000000000000000000f90f1cab2d62898699aadc957ef810dbfa27e38b380aaf3f1f18281cf39b7d21a937a66390f507177c7fcdc2", "hex");

const blockHash = dblSha256(rawHeader);

console.log("Blockhash: ", blockHash.toString("hex"));

const blockTopicKey = anchor.web3.PublicKey.findProgramAddressSync(
  [Buffer.from(anchor.utils.bytes.utf8.encode(headerSeed)), blockHash],
  program.programId
)[0];

const header = {
  version: rawHeader.readUInt32LE(),
  reversedPrevBlockhash: [...rawHeader.slice(4, 36)],
  merkleRoot: [...rawHeader.slice(36, 68)],
  timestamp: rawHeader.readUInt32LE(68),
  nbits: rawHeader.readUInt32LE(72),
  nonce: rawHeader.readUInt32LE(76)
};

const rawHeader2 = Buffer.from("00006020779dcf4332b66c507cd5e88038d4ca7911aaac6d14e506000000000000000000cb87f699dd722bf616a5e47b4041f3843b40e5fdc9b95440d7d3d1b230a93ad57f3ba66390f5071787a84b20", "hex");

const blockHash2 = dblSha256(rawHeader2);

console.log("Blockhash: ", blockHash2.toString("hex"));

const blockTopicKey2 = anchor.web3.PublicKey.findProgramAddressSync(
  [Buffer.from(anchor.utils.bytes.utf8.encode(headerSeed)), blockHash2],
  program.programId
)[0];

const header2 = {
  version: rawHeader2.readUInt32LE(),
  reversedPrevBlockhash: [...rawHeader2.slice(4, 36)],
  merkleRoot: [...rawHeader2.slice(36, 68)],
  timestamp: rawHeader2.readUInt32LE(68),
  nbits: rawHeader2.readUInt32LE(72),
  nonce: rawHeader2.readUInt32LE(76)
};

describe("btc-relay", () => {
  // Configure the client to use the local cluster.
  anchor.setProvider(provider);

  const seed = [Buffer.from(anchor.utils.bytes.utf8.encode(mainStateSeed))]
  const [mainStateKey, nonce] = anchor.web3.PublicKey.findProgramAddressSync(
    seed,
    program.programId
  );

  it("Is initialized!", async () => {
    // Add your test here.
    const signature = await provider.connection.requestAirdrop(signer.publicKey, 10000000000);
    const latestBlockhash = await provider.connection.getLatestBlockhash();
    await provider.connection.confirmTransaction(
      {
        signature,
        ...latestBlockhash,
      },
      commitment
    );

    const tx = await programPaidBy(signer).methods
      .initialize(
        header,
        0xbbaae,
        [...Buffer.from("3c2798e9046c7f28c8e90b62".padStart(64, "0"), "hex")],
        0x63a080a4, 
        [
          0x63a080a4,
          0x63a080a4,
          0x63a080a4,
          0x63a080a4,
          0x63a080a4,
          0x63a080a4,
          0x63a080a4,
          0x63a080a4,
          0x63a080a4,
          0x63a080a4
        ]
      )
      .accounts({
        signer: signer.publicKey,
        headerTopic: blockTopicKey,
        mainState: mainStateKey,
        systemProgram: SystemProgram.programId
      })
      .signers([signer])
      .transaction();

    const result = await provider.sendAndConfirm(tx, [signer], {
      skipPreflight: false
    });

    console.log("Your transaction signature", result);

      const [depositAccount, bump] = await anchor.web3.PublicKey.findProgramAddress(
          [Buffer.from("solana_deposit")],
          program.programId
      );

      console.log(bump);

      const programBalance = await provider.connection.getBalance(depositAccount);
      console.log(`Program balance ${programBalance}`);

      const depositTx = await program.rpc.deposit(
          new anchor.BN(1000000000), {
              accounts: {
                  user: signer.publicKey,
                  depositAccount,
                  systemProgram: SystemProgram.programId
              },
              signers: [signer],
          });

      console.log("Your transaction signature", depositTx);

      const latestBlockhashDep = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction(
          {
              signature: depositTx,
              ...latestBlockhashDep,
          },
          commitment
      );
      const programBalanceAfter = await provider.connection.getBalance(depositAccount);
      console.log(`Program balance after ${programBalanceAfter}`);
  });

  it("Verify tx!", async () => {
    
    await new Promise(resolve => setTimeout(resolve, 2000));
    
    const coder = new anchor.BorshCoder(program.idl);
    const eventParser = new anchor.EventParser(program.programId, coder);

    const fetched = await provider.connection.getSignaturesForAddress(blockTopicKey, {
      limit: 1000
    }, "confirmed");

    console.log(fetched);

    const transaction = await provider.connection.getTransaction(fetched[fetched.length-1].signature, {
        commitment: "confirmed"
    });

    console.log("TX: ", transaction);

    let commitedHeader;
    if(transaction.meta.err==null) {
        const events = eventParser.parseLogs(transaction.meta.logMessages);
        for(let event of events) {
          console.log("Event: ", JSON.stringify(event, null, 4));
          commitedHeader = event.data.header;
        }
    }

    // raw bytes of a67f13443942e0ee8697b8e4cb72b023c927f8b045402a70da2218da2ff4c252 bitcoin mainnet tx
    // https://www.blockchain.com/explorer/transactions/btc/a67f13443942e0ee8697b8e4cb72b023c927f8b045402a70da2218da2ff4c252
    const txBytes = "02000000000102a38425992fc058588b427fcae9221c68866659831c66795373ed1a81718f925b0e00000000fdffffff929dcafdada394d1b1570310ff115b26ce05b8404a95c2c9a27f1f59cb2c72b50000000000fdffffff014d7f720000000000160014f8e21f7f39f0abde360903492aab1858af4cee2a024730440220699a30ce9337e11d4208dfdee834ecce0fdb43796efb999db73c259f9f620aa80220190d732165b5bf2086bd0eb2808b91b3616f38c153f10c2097b30b6ea581720e0121025ca59290b01ed8d8f01cef4be862c8564ffb994bdca331315f47c14fd04e58f30247304402207c40800f50b06e3aff527904ca496c5bf4190c0ae81cf682ee1833a581de70c8022012e61f0e52fd5d7af94334e31fba20014272ef1b6d73ce4991eed4f5d8267a150121032397da1f5ece45a8e84eedef6aa83b472a12a405cb7fa4a07628b2a2d165a5d5adba0b00";
    const merkleProof = ["b5722ccb591f7fa2c9c2954a40b805ce265b11ff100357b1d194a3adfdca9d92","0465507dfb8c4c9e24f57f7aa9aa90ea084872639ec87ffe102d5ccf77b33a76","fb56b2adbc130f2bcf10e11f0b8a1fec6e8b4b6d0129cc094d222650dce984dc","c1a1db7717ee11aaad8e231ff8105ae14be93876a22bbcba6e8b605bc9fa2530","1e6ca60faac3a45a0e461f26d4d8b038279ebb195cd294c8bc65122745beee85","76358016750f17dec7fbe0771c4abf07388734f09a79a2f670eebaf8a67a8abc","35b7e6b41d1064f51deeeeb7a1c769f109bb0746565f993148fd517bc46b9b8c","a628c02d83eaa32d54efbcfbcf1458a5c73d6924602b9b186731e48e1db8d19b","37ffa85bd1d8d3e30e8006905115b7fa87f6399276627473975334a0ec53606f","e17c0fd9dc3e9fd61f0036f8dd17df78cc7732533cf153fe332238442a58c2f6","21fb731ff8fd7a5b7472f111e240417c646455022d59a71a9a4ba2d37022a9e8","d265a5b211f0232e84660f97354799a64b7174139531688fde6b182aed83ff8f"];
    const position = 7;
    const blockHeight = 768686;

    const ix = await programPaidBy(signer).methods
      .verifyTransaction(
        Buffer.from(txBytes, "hex"),
        1,
        position,
        merkleProof.map(e => Buffer.from(e, "hex").reverse()),
        commitedHeader
      )
      .accounts({
        signer: signer.publicKey,
        mainState: mainStateKey
      })
      .signers([signer])
      .instruction();

    console.log("IX: ", ix);

    // to increase CU limit
    // .add(ComputeBudgetProgram.setComputeUnitLimit({
    //    units: 1_400_000, // Set the desired number of compute units
    //  }))
    const tx = new anchor.web3.Transaction().add(ix);

    // to catch error and log more details about it
    // .catch(e => console.error(e));
    const result = await provider.sendAndConfirm(tx, [signer], {
      skipPreflight: false
    });

  });

  return;

  it("submit next blockheader", async () => {
    const coder = new anchor.BorshCoder(program.idl);
    const eventParser = new anchor.EventParser(program.programId, coder);

    const fetched = await provider.connection.getSignaturesForAddress(blockTopicKey, {
      limit: 1000
    }, "confirmed");

    console.log(fetched);

    const transaction = await provider.connection.getTransaction(fetched[fetched.length-1].signature, {
        commitment: "confirmed"
    });

    console.log("TX: ", transaction);

    let commitedHeader;
    if(transaction.meta.err==null) {
        const events = eventParser.parseLogs(transaction.meta.logMessages);
        for(let event of events) {
          console.log("Event: ", JSON.stringify(event, null, 4));
          commitedHeader = event.data.header;
        }
    }
    
    const tx = await programPaidBy(signer).methods
      .submitBlockHeaders(
        [header2],
        commitedHeader
      )
      .accounts({
        signer: signer.publicKey,
        mainState: mainStateKey,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts([{
        pubkey: blockTopicKey2,
        isSigner: false,
        isWritable: false
      }])
      .signers([signer])
      .transaction();

    const result = await provider.sendAndConfirm(tx, [signer], {
      skipPreflight: true
    });

    console.log("Your transaction signature", result);
  });

  it("get state", async() => {
    const fetched = await program.account.mainState.fetch(mainStateKey);

    fetched.blockCommitments = null;
    console.log(fetched);
  })
  
  it("get past log", async() => {
    const coder = new anchor.BorshCoder(program.idl);
    const eventParser = new anchor.EventParser(program.programId, coder);

    const fetched = await provider.connection.getSignaturesForAddress(blockTopicKey, {
      limit: 1000
    }, "confirmed");

    console.log(fetched);

    const transaction = await provider.connection.getTransaction(fetched[fetched.length-1].signature, {
        commitment: "confirmed"
    });

    console.log("TX: ", transaction);

    if(transaction.meta.err==null) {
        const events = eventParser.parseLogs(transaction.meta.logMessages);
        for(let event of events) {
          console.log("Event: ", JSON.stringify(event, null, 4));
        }
    }
  });
});
