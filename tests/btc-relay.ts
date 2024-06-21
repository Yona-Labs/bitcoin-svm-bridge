import * as anchor from "@project-serum/anchor";
import {Program} from "@project-serum/anchor";
import {BtcRelay} from "../target/types/btc_relay";
import {createHash} from "crypto";

const {SystemProgram, ComputeBudgetProgram} = anchor.web3;

const commitment: anchor.web3.Commitment = "confirmed";

const mainStateSeed = "state";
const headerSeed = "header";
const PRUNING_FACTOR = 250;
const accountSize = 8 + 4 + 4 + 4 + 32 + 8 + 4 + (PRUNING_FACTOR * 32);

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
        const signature = await provider.connection.requestAirdrop(signer.publicKey, 100000000000);
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
            new anchor.BN(10000000000), {
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

        // console.log(fetched);

        const transaction = await provider.connection.getTransaction(fetched[fetched.length - 1].signature, {
            commitment: "confirmed"
        });

        // console.log("TX: ", transaction);

        let commitedHeader;
        if (transaction.meta.err == null) {
            const events = eventParser.parseLogs(transaction.meta.logMessages);
            for (let event of events) {
                // console.log("Event: ", JSON.stringify(event, null, 4));
                commitedHeader = event.data.header;
            }
        }

        // raw bytes of a67f13443942e0ee8697b8e4cb72b023c927f8b045402a70da2218da2ff4c252 bitcoin mainnet tx
        // https://www.blockchain.com/explorer/transactions/btc/a67f13443942e0ee8697b8e4cb72b023c927f8b045402a70da2218da2ff4c252
        const txBytes = "02000000000102a38425992fc058588b427fcae9221c68866659831c66795373ed1a81718f925b0e00000000fdffffff929dcafdada394d1b1570310ff115b26ce05b8404a95c2c9a27f1f59cb2c72b50000000000fdffffff014d7f720000000000160014f8e21f7f39f0abde360903492aab1858af4cee2a024730440220699a30ce9337e11d4208dfdee834ecce0fdb43796efb999db73c259f9f620aa80220190d732165b5bf2086bd0eb2808b91b3616f38c153f10c2097b30b6ea581720e0121025ca59290b01ed8d8f01cef4be862c8564ffb994bdca331315f47c14fd04e58f30247304402207c40800f50b06e3aff527904ca496c5bf4190c0ae81cf682ee1833a581de70c8022012e61f0e52fd5d7af94334e31fba20014272ef1b6d73ce4991eed4f5d8267a150121032397da1f5ece45a8e84eedef6aa83b472a12a405cb7fa4a07628b2a2d165a5d5adba0b00";
        const merkleProof = ["b5722ccb591f7fa2c9c2954a40b805ce265b11ff100357b1d194a3adfdca9d92", "0465507dfb8c4c9e24f57f7aa9aa90ea084872639ec87ffe102d5ccf77b33a76", "fb56b2adbc130f2bcf10e11f0b8a1fec6e8b4b6d0129cc094d222650dce984dc", "c1a1db7717ee11aaad8e231ff8105ae14be93876a22bbcba6e8b605bc9fa2530", "1e6ca60faac3a45a0e461f26d4d8b038279ebb195cd294c8bc65122745beee85", "76358016750f17dec7fbe0771c4abf07388734f09a79a2f670eebaf8a67a8abc", "35b7e6b41d1064f51deeeeb7a1c769f109bb0746565f993148fd517bc46b9b8c", "a628c02d83eaa32d54efbcfbcf1458a5c73d6924602b9b186731e48e1db8d19b", "37ffa85bd1d8d3e30e8006905115b7fa87f6399276627473975334a0ec53606f", "e17c0fd9dc3e9fd61f0036f8dd17df78cc7732533cf153fe332238442a58c2f6", "21fb731ff8fd7a5b7472f111e240417c646455022d59a71a9a4ba2d37022a9e8", "d265a5b211f0232e84660f97354799a64b7174139531688fde6b182aed83ff8f"];
        const position = 7;
        const blockHeight = 768686;

        const [depositAccount, bump] = await anchor.web3.PublicKey.findProgramAddress(
            [Buffer.from("solana_deposit")],
            program.programId
        );

        const ix = await programPaidBy(signer).methods
            .verifySmallTx(
                Buffer.from(txBytes, "hex"),
                1,
                position,
                merkleProof.map(e => Buffer.from(e, "hex").reverse()),
                commitedHeader
            )
            .accounts({
                signer: signer.publicKey,
                mainState: mainStateKey,
                depositAccount,
            })
            .signers([signer])
            .instruction();

        // console.log("IX: ", ix);

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

    it("Submit big tx", async () => {
        await new Promise(resolve => setTimeout(resolve, 2000));

        const coder = new anchor.BorshCoder(program.idl);
        const eventParser = new anchor.EventParser(program.programId, coder);

        const fetched = await provider.connection.getSignaturesForAddress(blockTopicKey, {
            limit: 1000
        }, "confirmed");

        // console.log(fetched);

        const transaction = await provider.connection.getTransaction(fetched[fetched.length - 1].signature, {
            commitment: "confirmed"
        });

        // console.log("TX: ", transaction);

        let commitedHeader;
        if (transaction.meta.err == null) {
            const events = eventParser.parseLogs(transaction.meta.logMessages);
            for (let event of events) {
                // console.log("Event: ", JSON.stringify(event, null, 4));
                commitedHeader = event.data.header;
            }
        }

        // raw bytes of ab0d1da870784e72ba9f51810db7cc8b3edd3466e72e7083e9a42f1a05cbac8c bitcoin mainnet tx
        // https://www.blockchain.com/explorer/transactions/btc/ab0d1da870784e72ba9f51810db7cc8b3edd3466e72e7083e9a42f1a05cbac8c
        const bigTxHex = "020000000001141a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40900000017160014b8b72c5943b2cfd9de7d92f48e9fcad01d055de7feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40f000000171600140c532e8594f510da8b0d387f563934861575e844feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40700000017160014d07cba9645f2814af5ed30aef767234ddda9ac13feffffff3115e2b995e8a8f6ff045671e932f151b8df53c5f13ba7dcf03beb5cfaa1eec00000000000feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40f000000171600140c532e8594f510da8b0d387f563934861575e844feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40900000017160014b8b72c5943b2cfd9de7d92f48e9fcad01d055de7feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd405000000171600142aa38dc9e745450410da5bc9350ee8d6d12e0c70feffffff147d565149cd432db0af1a6fe7540e20f2878897127358605046b2e63702e2220100000000feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40100000017160014a748dee29baef670e3264341b2e7aebdfdb3a644feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40e00000017160014995cd732c74a3446f1ad35bafbd8d6f79828ef04feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40300000017160014004bebeb0b79c94754f5458a3ed8a293370df7fbfeffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40e00000017160014995cd732c74a3446f1ad35bafbd8d6f79828ef04feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40a00000017160014e3a8924beabbfcf3a3350368567a5c972dbbf2e7feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b4040000001716001476e4db8a1d5c4c238775df63899f71dfda0197f4feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b4060000001716001437453e98e510a863dd06be9f96d0292ba24cdcf3feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b405000000171600142aa38dc9e745450410da5bc9350ee8d6d12e0c70feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40100000017160014a748dee29baef670e3264341b2e7aebdfdb3a644feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40800000017160014ae471e13840e2b5dcdec9c4a9b54ba11fe7b665cfeffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40b0000001716001412d0bf7a471fde32b6ab8bea44c08806b6b9d178feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40300000017160014004bebeb0b79c94754f5458a3ed8a293370df7fbfeffffff0283a90f0000000000160014b32309634f6145184542e93869b693178a2b3068c01a1f15000000001976a9140035fb9766613a5d60c9cb7e68b7031b93f9c0ce88ac02473044022078d264c3710e473478b440d8bce655972d71dac199040df71a667f4616fc3d3e02203266223a3202045e40f317c717a81767979d7e569acabb9d6f80dbdcc3f61c50012103ad7a479ae2c1a65e5a81284dae6e9192cbee296377d5df5ed33ceb4d3845568702473044022050fed0de3d27a061542f9e85f32e178b6cab49a40f5411236cc471343a30fe0f02205a962c3bfe55b46022a0c5c594c13d8c7b58162f78dfeb9c9daa6ee3fa42c8330121030f0ce1edbf552656ea2ff791d88f84df9d68023fcf000d6a7ee8ecd5dc1e3ed202473044022012e7f7ad4d6ea9cdce4b91a84f02dd5141d60e968acc250c9ebf5f974549c7b102201c8687048907f27bbd0e48a98c320b1629de70bfbcdebefffa8c5cfbf76c9082012103d79e1063b8b5bd169ac5b2791b57e19f43160c7e553baa0b243cb666d30c19f40247304402207d9665fca9d76ed7737f309dda016d767cdd2c6aadb9bff20e2d6c9b5534b6130220225293e7650f9b8fe3c761d20ae226aa24e92f124d677141d3b24c4610fb2006012102100a0ba10ca19d9bfac2c933d473adc78043d4bfeae96ded14b3fc19466f351a02473044022074cbe0c5b4efb62d9f4b917bd528541ec3a6bf0f8d21ba2e789c34368241d99002201e5cbfb855698ec5ace5672193ce78c239ff33cc2cbf6b546af9e5531132e7cf0121030f0ce1edbf552656ea2ff791d88f84df9d68023fcf000d6a7ee8ecd5dc1e3ed202473044022034b92d6d96db9347e61e8a915ab3a37a295af6f08b3a2de0c4192828dc4804d6022067efdd8c5b8197f2b890bb46894e71f3088b64f731bd8dd3ae08b4f1918a4ade012103ad7a479ae2c1a65e5a81284dae6e9192cbee296377d5df5ed33ceb4d3845568702473044022041bd9d222072584b908b065311fa4fb84f58adbd6a47b911f11549b029e4363c022043c8bb3e485959c3531f078e460aad3c66c515d13e16303446733beea2e93a4c012102b9faff46c4afbb0c7303f47ef5d711bd8da4663bc92fe8c7295e0352cecb657502473044022057ced8a248383ca77d0e3c14616100e7343dc3f12f825188730ae5b3ed0c9c5e0220676d06cbd7bc52a2feef4cb5cbac942c8b61832438ca8b29a59beb407250dfaf01210273402ba5ce7143bdd1b4b1063378cae9cda29dcd820f339cf38b15549cb4007e024730440220078968f47a2dec93b1a4920f9a81177d8ce61a723a7fb183675bae23e750e5da02202271ded859d28a9d58bcada4566ed55b9cc590143af131917633de4668ab7cdb012102e8dedb110d0812d3a769c6f0539b3bc2164d748d386433407d503c4d6f2c44770247304402205882941c239cbfd0ba7619198894340a243803396913637f45ef2ff4a6a96791022041aa555dfc56b43f2d27ccf8691370b316f0195835e5d542e95640d8a04a34e401210393c79d23ae89f461f039b895dfd6f365ef5cc89f1a3ef030382f80c2cbd84caa024730440220272a9d37449a4f417657c0e1e5d3a87d1f02478f1fc1ddce9ab7013231ccee050220252fd6e3bdfd3f5499df7f064cdc5c1406b097d300aa734c31c97e017a341b1d0121025389a45a00c6d57e6a61258e95d6f7413f2c23274f7f82a7164ec040f7bf73ab024730440220700e5220c44e0f68931f046b52aa2d1273d86b50b7b86f2b847b98785dd0aa0102207aa0a3657e7cb1169a64f0efc15f10296f335cdadef99b62856de1b990c71c1c01210393c79d23ae89f461f039b895dfd6f365ef5cc89f1a3ef030382f80c2cbd84caa0247304402202f1e547a5a43776c855d0c37ec4cdc812cd381069337e6e843f49f9b4b9c5b8902207a565110b4af1f67300d41cf2f499a8be5424af394cba8c0cc7b373e650eeced012102e5aea5c0d5acb437f8f2bc6e8a293e56fe7f021c3b63aa01e5d681d8660af2730247304402205e2bfddb2e3b1eb6e43718e6aaa9d823ba47e295c5d381ba20817ecf37a2a2e9022068c9cc98005df5bc8f13385feed2350af854aec4b0de94ae1abaaddefe1d5fe00121030c9daf8f58ccd1733de05574964eaf2810c5d6e2375dee0f49603151cf589e1d0247304402205eb0fab222b1f5d3adf54784fda4a555c5c8acc4151908f194bd1a4fb2f6ba540220370be1435d09b6c1d94297a6128fff677336877ad30994befbacdab941d6c35701210241202dcd1c9a451c4a212f88b10c2bdf2afab7bfbd14b76c2a9078c932dfffeb02473044022003d2cb7fe423c634caeec0d3d00085351041f83f33a77596d5c104283bae70aa022031affaf6564c24d946cec27419bfb5182dcab6be1e26fcf2aa334eb7d6027160012102b9faff46c4afbb0c7303f47ef5d711bd8da4663bc92fe8c7295e0352cecb6575024730440220156f5c439dbc5ec403fcde44e27804906051d717466cf057026d4fd86be5522802204c97b2c98e06d6ea14572c91d5da9f89c4bf7d37df07badcc60b527b27e047e1012102e8dedb110d0812d3a769c6f0539b3bc2164d748d386433407d503c4d6f2c4477024730440220533759a56d6f141f00ecc3f889f5c839c340fbc015cdec5cc7595082c00e702802200e1829c99288c487cfb5af72e909e0f8d30d65c027207d1b9f59df7a728887dc012103a261fa4d379512d74d8eb14a7f797f694bbbda5ff3dcc05a08a55be7dd2b2e000247304402203fa51d8f3dd4cd19162760384036f75a15c56e432cb73a3c7de29f0a0359d48e0220679b4fa29004ebf1f04f924f7af859abc98a89a5476a57c7ad8cc9a6cdd1a437012102d78e2a75a028716ab8d9eb64833f546241fbe0ce54051fd534436135414d12c60247304402200fb88079388afdc469bc428bb7cff08cd3bf1ad9d2a0504eba6d6d23256959ef02204182acf8104ed927a6a96a8384c60a35ae45a10d04a25a3741830316014404e20121025389a45a00c6d57e6a61258e95d6f7413f2c23274f7f82a7164ec040f7bf73abadba0b00";

        // ab0d1da870784e72ba9f51810db7cc8b3edd3466e72e7083e9a42f1a05cbac8c tx merkle proof
        // {"jsonrpc":"2.0","result":{"block_height":768686,"merkle":["6e166b147ce38f1f0a87a127a24ecd26d41bded66bcb2b593bde6e689272a260","6fe80557cadc2e1d63f97f65a9fcc6e74432495b431215619d2fe8a66800efa2","d441810cda3fd857a7f68de1236ede49cf10cd7fa0f00a413eeb898c0e536d2b","600d8fe6623382994da2ea73c966c700c6671919dc6360784af5af4509cedeba","dce7b1f2c2a73f74db640f801ad6d01a524f61f7707024d0180cb6a332444adf","76358016750f17dec7fbe0771c4abf07388734f09a79a2f670eebaf8a67a8abc","35b7e6b41d1064f51deeeeb7a1c769f109bb0746565f993148fd517bc46b9b8c","a628c02d83eaa32d54efbcfbcf1458a5c73d6924602b9b186731e48e1db8d19b","37ffa85bd1d8d3e30e8006905115b7fa87f6399276627473975334a0ec53606f","e17c0fd9dc3e9fd61f0036f8dd17df78cc7732533cf153fe332238442a58c2f6","21fb731ff8fd7a5b7472f111e240417c646455022d59a71a9a4ba2d37022a9e8","d265a5b211f0232e84660f97354799a64b7174139531688fde6b182aed83ff8f"],"pos":25},"id":2}
        const merkleProof = ["6e166b147ce38f1f0a87a127a24ecd26d41bded66bcb2b593bde6e689272a260", "6fe80557cadc2e1d63f97f65a9fcc6e74432495b431215619d2fe8a66800efa2", "d441810cda3fd857a7f68de1236ede49cf10cd7fa0f00a413eeb898c0e536d2b", "600d8fe6623382994da2ea73c966c700c6671919dc6360784af5af4509cedeba", "dce7b1f2c2a73f74db640f801ad6d01a524f61f7707024d0180cb6a332444adf", "76358016750f17dec7fbe0771c4abf07388734f09a79a2f670eebaf8a67a8abc", "35b7e6b41d1064f51deeeeb7a1c769f109bb0746565f993148fd517bc46b9b8c", "a628c02d83eaa32d54efbcfbcf1458a5c73d6924602b9b186731e48e1db8d19b", "37ffa85bd1d8d3e30e8006905115b7fa87f6399276627473975334a0ec53606f", "e17c0fd9dc3e9fd61f0036f8dd17df78cc7732533cf153fe332238442a58c2f6", "21fb731ff8fd7a5b7472f111e240417c646455022d59a71a9a4ba2d37022a9e8", "d265a5b211f0232e84660f97354799a64b7174139531688fde6b182aed83ff8f"];

        const txId = "ab0d1da870784e72ba9f51810db7cc8b3edd3466e72e7083e9a42f1a05cbac8c";

        const txIdBytes = Buffer.from(txId, "hex").reverse();
        const txBytes = Buffer.from(bigTxHex, "hex");
        const txPos = new anchor.BN(25);

        const [txAccount, bump] = anchor.web3.PublicKey.findProgramAddressSync(
            [txIdBytes],
            program.programId
        );

        const ix = await programPaidBy(signer).methods
            .initBigTxVerify(
                txIdBytes,
                new anchor.BN(txBytes.length),
                1,
                txPos,
                merkleProof.map(e => Buffer.from(e, "hex").reverse()),
                commitedHeader
            )
            .accounts({
                signer: signer.publicKey,
                mainState: mainStateKey,
                txAccount,
                systemProgram: SystemProgram.programId
            })
            .signers([signer])
            .instruction();

        // console.log("IX: ", ix);

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

        const chunkSize = 800;
        for (let i = 0; i < txBytes.length; i += chunkSize) {
            const chunk = txBytes.subarray(i, i + chunkSize);

            const ix = await programPaidBy(signer).methods
                .storeTxBytes(
                    txIdBytes,
                    chunk
                )
                .accounts({
                    signer: signer.publicKey,
                    txAccount
                })
                .signers([signer])
                .instruction();

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
        }

        const balanceBeforeFinalize = await provider.connection.getBalance(signer.publicKey);
        console.log(`balanceBeforeFinalize ${balanceBeforeFinalize}`);

        const [depositAccount, depositBump] = await anchor.web3.PublicKey.findProgramAddress(
            [Buffer.from("solana_deposit")],
            program.programId
        );

        const finalizeIx = await programPaidBy(signer).methods
            .finalizeTxProcessing(
                txIdBytes
            )
            .accounts({
                signer: signer.publicKey,
                txAccount,
                depositAccount
            })
            .signers([signer])
            .instruction();

        // to increase CU limit
        // .add(ComputeBudgetProgram.setComputeUnitLimit({
        //    units: 1_400_000, // Set the desired number of compute units
        //  }))
        const finalizeTx = new anchor.web3.Transaction().add(ComputeBudgetProgram.setComputeUnitLimit({
            units: 300_000, // Set the desired number of compute units
        })).add(finalizeIx);

        // to catch error and log more details about it
        // .catch(e => console.error(e));
        const finalizeResult = await provider.sendAndConfirm(finalizeTx, [signer], {
            skipPreflight: false
        });

        const latestBlockhash = await provider.connection.getLatestBlockhash();
        await provider.connection.confirmTransaction(
            {
                signature: finalizeResult,
                ...latestBlockhash,
            },
            commitment
        );

        const balanceAfterFinalize = await provider.connection.getBalance(signer.publicKey);
        console.log(`balanceAfterFinalize ${balanceAfterFinalize}`);
    });

    return;

    it("submit next block header", async () => {
        const coder = new anchor.BorshCoder(program.idl);
        const eventParser = new anchor.EventParser(program.programId, coder);

        const fetched = await provider.connection.getSignaturesForAddress(blockTopicKey, {
            limit: 1000
        }, "confirmed");

        // console.log(fetched);

        const transaction = await provider.connection.getTransaction(fetched[fetched.length - 1].signature, {
            commitment: "confirmed"
        });

        // console.log("TX: ", transaction);

        let commitedHeader;
        if (transaction.meta.err == null) {
            const events = eventParser.parseLogs(transaction.meta.logMessages);
            for (let event of events) {
                // console.log("Event: ", JSON.stringify(event, null, 4));
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

    it("get state", async () => {
        const fetched = await program.account.mainState.fetch(mainStateKey);

        fetched.blockCommitments = null;
        // console.log(fetched);
    })

    it("get past log", async () => {
        const coder = new anchor.BorshCoder(program.idl);
        const eventParser = new anchor.EventParser(program.programId, coder);

        const fetched = await provider.connection.getSignaturesForAddress(blockTopicKey, {
            limit: 1000
        }, "confirmed");

        // console.log(fetched);

        const transaction = await provider.connection.getTransaction(fetched[fetched.length - 1].signature, {
            commitment: "confirmed"
        });

        // console.log("TX: ", transaction);

        if (transaction.meta.err == null) {
            const events = eventParser.parseLogs(transaction.meta.logMessages);
            for (let event of events) {
                // console.log("Event: ", JSON.stringify(event, null, 4));
            }
        }
    });
});
