import * as anchor from "@coral-xyz/anchor";
import {Program} from "@coral-xyz/anchor";
import {BtcRelay} from "../target/types/btc_relay";
import {createHash} from "crypto";

import * as chai from 'chai';
import chaiAsPromised = require('chai-as-promised');

chai.use(chaiAsPromised);

const {SystemProgram, ComputeBudgetProgram, LAMPORTS_PER_SOL} = anchor.web3;

const commitment: anchor.web3.Commitment = "confirmed";

const mainStateSeed = "state";
const headerSeed = "header";
const PRUNING_FACTOR = 250;
const accountSize = 8 + 4 + 4 + 4 + 32 + 8 + 4 + (PRUNING_FACTOR * 32);

function dblSha256(data: Buffer) {
    const hash1 = createHash("sha256").update(data).digest();
    return createHash("sha256").update(hash1).digest();
}

const provider = anchor.AnchorProvider.env();
const program = anchor.workspace.BtcRelay as Program<BtcRelay>;

const signer = anchor.web3.Keypair.generate();

const rawHeader = Buffer.from("0000002083d04469c82347e01488ee3acd643ef87f3849918bc3c7599815bee9485be667343dfd9da41f797fa67aebff13a002126157153fb0e1d601075ed502470b21a4e8c09466ffff7f2001000000", "hex");

const blockHash = dblSha256(rawHeader);

console.log("Initial Blockhash: ", blockHash.toString("hex"));

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

const mintReceiver = new anchor.web3.PublicKey("5Xy6zEA64yENXm9Zz5xDmTdB8t9cQpNaD3ZwNLBeiSc5");

let initCommittedHeader;

async function getCommitedHeaderFromTx(signature: string): Promise<any> {
    const latestBlockhashDep = await provider.connection.getLatestBlockhash();
    await provider.connection.confirmTransaction(
        {
            signature,
            ...latestBlockhashDep,
        },
        commitment
    );

    const coder = new anchor.BorshCoder(program.idl);
    const eventParser = new anchor.EventParser(program.programId, coder);

    const transaction = await provider.connection.getTransaction(signature, {
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
    return commitedHeader;
}

describe("btc-relay", () => {
    // Configure the client to use the local cluster.
    anchor.setProvider(provider);

    const seed = [Buffer.from(anchor.utils.bytes.utf8.encode(mainStateSeed))]
    const [mainStateKey, nonce] = anchor.web3.PublicKey.findProgramAddressSync(
        seed,
        program.programId
    );

    const [depositAccount] = anchor.web3.PublicKey.findProgramAddressSync(
        [Buffer.from("solana_deposit")],
        program.programId
    );

    it("Is initialized!", async () => {
        // Add your test here.
        const signature = await provider.connection.requestAirdrop(signer.publicKey, 1000 * LAMPORTS_PER_SOL);
        const latestBlockhash = await provider.connection.getLatestBlockhash();
        await provider.connection.confirmTransaction(
            {
                signature,
                ...latestBlockhash,
            },
            commitment
        );

        const tx = await program.methods
            .initialize(
                header,
                12999,
                Array(32).fill(0),
                1721024744,
                Array(10).fill(1721024744)
            )
            .accounts({
                signer: signer.publicKey,
                headerTopic: blockTopicKey,
                mainState: mainStateKey,
                systemProgram: SystemProgram.programId
            })
            .signers([signer])
            .transaction();

        const initResult = await provider.sendAndConfirm(tx, [signer], {
            skipPreflight: false
        }).catch(e => {
            console.error(e);
            throw e
        });

        console.log("Initialize transaction signature", initResult);

        initCommittedHeader = await getCommitedHeaderFromTx(initResult);

        const [depositAccount, bump] = await anchor.web3.PublicKey.findProgramAddress(
            [Buffer.from("solana_deposit")],
            program.programId
        );

        const programBalance = await provider.connection.getBalance(depositAccount);
        console.log(`Program balance ${programBalance}`);

        const depositTx = await program.rpc.deposit(
            new anchor.BN(900 * LAMPORTS_PER_SOL), {
                accounts: {
                    signer: signer.publicKey,
                    depositAccount,
                    systemProgram: SystemProgram.programId
                },
                signers: [signer],
            })
            .catch(e => {
                console.error(e);
                throw e
            });

        console.log("Deposit transaction signature", depositTx);

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

    it("Submit more blocks and verify small tx!", async () => {
        let headers = [
            {
                height: 13000,
                bytes: Buffer.from("000000206decbe778da43b24f4395aaeebc97c89094101e46c7a0c4d5fc9051ed3282e67127150aa9fde65fcef6b76a34fb1a42f723f22c5595210cdd42c7dd1fa78148024c19466ffff7f2001000000", "hex")
            },
            {
                height: 13001,
                bytes: Buffer.from("00000020db122d85f1ae9fc3986d6a4c2b21e5265c9eeba0e99130db66c20c39beb44c38c205b1a9cfa5bea4ead7eea69ee42bb45190c41a30c12fc1582b77066f815ffe60c19466ffff7f2000000000", "hex")
            },
            {
                height: 13002,
                bytes: Buffer.from("00000020a40ece6f8430488f6f6f1295da636bf81e474d2f3ca873b0e87a84e92587e925fe900e0bc36c95a92b967609ef0fb8a111bcb2fc438d0fbb5511d7de84937a4a9cc19466ffff7f2007000000", "hex")
            },
            {
                height: 13003,
                bytes: Buffer.from("00000020b10eb60be55e6724fa90b7c3b2d1eada2616adfd3d3e329d5f98ade7490d3579650fdc194fb23924b32d72ac1eb799074d1738eec4dba7d6a169705f34a7b406d8c19466ffff7f2001000000", "hex")
            },
            {
                height: 13004,
                bytes: Buffer.from("00000020b91d0a922e971c40fb58faa74ffa6df04b853ed5771f1bddc5d27e6764c068441836f5234ade5dfcd36473936bbe001613458115a29c910a96dab48b4657cfd914c29466ffff7f2000000000", "hex")
            },
            {
                height: 13005,
                bytes: Buffer.from("000000208db6323ab452b57b73a635103e263579045e5afc2562b26e681e963171c0d159d77c0ebd73ef1fce1d12e59642ae1fd4166870764357489b81c4382c24cddc3750c29466ffff7f2000000000", "hex")
            },
            {
                height: 13006,
                bytes: Buffer.from("00000020d0a1aac5c43906496dfde5faf26611eb6f7e0273358a3f95d65727a20b484a69ae515b5aa48c4c811ad6ed026fad3ee55972f354d2fe74141c720ad82f8329408cc29466ffff7f2002000000", "hex")
            },
        ];

        let currentCommited = initCommittedHeader;
        for (let nextHeader of headers) {
            const header = {
                version: nextHeader.bytes.readUInt32LE(),
                reversedPrevBlockhash: [...nextHeader.bytes.slice(4, 36)],
                merkleRoot: [...nextHeader.bytes.slice(36, 68)],
                timestamp: nextHeader.bytes.readUInt32LE(68),
                nbits: nextHeader.bytes.readUInt32LE(72),
                nonce: nextHeader.bytes.readUInt32LE(76)
            };

            const headerHash = dblSha256(nextHeader.bytes);
            const headerTopic = anchor.web3.PublicKey.findProgramAddressSync(
                [Buffer.from(anchor.utils.bytes.utf8.encode(headerSeed)), headerHash],
                program.programId
            )[0];

            const tx = await program.methods
                .submitBlockHeaders(
                    [header],
                    currentCommited
                )
                .accounts({
                    signer: signer.publicKey,
                    mainState: mainStateKey,
                    systemProgram: SystemProgram.programId,
                })
                .remainingAccounts([{
                    pubkey: headerTopic,
                    isSigner: false,
                    isWritable: false
                }])
                .signers([signer])
                .transaction();

            const result = await provider.sendAndConfirm(tx, [signer], {
                skipPreflight: true
            });

            currentCommited = await getCommitedHeaderFromTx(result);
        }

        // raw bytes of 7c04665a396c766c68306c04ea3700975777fc8c198f352c92c2ebe0acb48443 Yona bitcoin regtest tx
        // http://139.59.156.238:8094/regtest/tx/7c04665a396c766c68306c04ea3700975777fc8c198f352c92c2ebe0acb48443
        const txBytes = "02000000000103d592a7cbfd1d3a2a79fc7b47fbafbd98db92c577910eba87614317f34b7c48100100000000fdffffff336d22988206b10ee6d8e8ecc882712046cb65bc0ef10f6da2c505352a9be6e30000000000fdffffff185b7398b1043b1f6e0d7df98bc049ddb4b427b678037313602124d596a04bf30000000000fdffffff01241acc1d000000002200204a3a5f1583c04e6c45ada0e4724f3e394122ce97d36762d24cc9f6563faee4850247304402202e017d47b1a6a0d65629171be7275b31c1cb3365d9f697d721895aca10d6125d022023d1cf4b458304cedfbd52ce59d7a80398ee8531b7d0ed352016a62d0239a5a60121021f0eeaabee6b006aa78ac93b26a9268e0079acb5a0a0e70e1ce03dd10b012d6802473044022046ff88c6af23c8cc02cebffd50f49aaddbb90cbfec5390a6821b5eabff3017f502201932303780e41e71d6effe1e1c5f944fb4c01baff6aa400a9d78bf5e61158155012102cc27969207f94386d7ae02a3f0605bbb87cec34d436d3a30cf22d447c164c9da02473044022001f4bb68622415dcb654101a4e9c73007c1e672f2a0370375e1d0aa8d63b3f7c022054dd59718599c12243077efe8b5ac6802cfb05f930e7646ef7e7cb339e4331c8012102cc27969207f94386d7ae02a3f0605bbb87cec34d436d3a30cf22d447c164c9dacd320000";
        const merkleProof = ["035949c9b8b899cfa8e4d2de26010d72f9ae1e52af6acd6fe8d2409305782414"];
        const position = 1;

        const [depositAccount, bump] = await anchor.web3.PublicKey.findProgramAddress(
            [Buffer.from("solana_deposit")],
            program.programId
        );

        const txId = "7c04665a396c766c68306c04ea3700975777fc8c198f352c92c2ebe0acb48443";

        const txIdBytes = Buffer.from(txId, "hex").reverse();

        const [txAccount] = anchor.web3.PublicKey.findProgramAddressSync(
            [txIdBytes],
            program.programId
        );

        const receiverBalanceBefore = await provider.connection.getBalance(mintReceiver);

        const ix = await program.methods
            .verifySmallTx(
                txIdBytes,
                Buffer.from(txBytes, "hex"),
                1,
                position,
                merkleProof.map(e => Buffer.from(e, "hex").reverse()),
                currentCommited
            )
            .accounts({
                signer: signer.publicKey,
                mainState: mainStateKey,
                depositAccount,
                mintReceiver,
                txAccount,
                systemProgram: SystemProgram.programId,
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
        let signature = await provider.sendAndConfirm(tx, [signer], {
            skipPreflight: false
        }).catch(e => {
            console.log(e);
            throw e;
        });

        const latestBlockhash = await provider.connection.getLatestBlockhash();
        await provider.connection.confirmTransaction(
            {
                signature,
                ...latestBlockhash,
            },
            commitment
        );
        const receiverBalanceAfter = await provider.connection.getBalance(mintReceiver);
        const expectedBalance = receiverBalanceBefore + 4999153000;
        chai.expect(receiverBalanceAfter).eq(expectedBalance);

        // should not allow to verify the same transaction again
        await chai.expect(provider.sendAndConfirm(tx, [signer], {
            skipPreflight: false
        })).to.be.eventually.rejectedWith("already in use");
    });

    it("Submit big tx", async () => {
        // raw bytes of 155ad532984baae90e7d4e71fa0c74748c95b2f53742e9ca80946f835c64d7b1 Yona bitcoin regtest tx
        // http://139.59.156.238:8094/regtest/tx/155ad532984baae90e7d4e71fa0c74748c95b2f53742e9ca80946f835c64d7b1
        const bigTxHex = "0200000000012b78eb7e06c06598eb78cc088cf0d62e78284413d51bb1c47a3e59ba169c1697360000000000fdffffffc0bbde7b6582b52ee02c12d7bba8e6f72dd5a176a73d809c89cba221c84a7b560000000000fdffffffb62d61da5bcb18478ab789caf5a54e53f6df2d04e6ec6e9fd4b57e736b3fda270000000000fdffffff7cd6efba8bd1bbc2efa91cbd1432f1ca080ae435c1db3dd6ab3cd1ec97958a070000000000fdffffffb7d88b45dc153d86aedea03100810d87a15d70c9ffe7262e5dd19db7081694cb0000000000fdffffff95b50dc47098d25034b2a318569ff1a01984331e14a3afc66c31d1456c75a5190000000000fdffffff4a280355f85f71a734136bf2a82eec35d9760505397e7ccdedd2791a93bdc3a20000000000fdffffffd79fe7a9a74261f732c07b4087b1fa7619240bcc0173d72b8de0e49a0c8c344d0000000000fdffffff9fc9fd31d0b91b14853a7bb38d9a1c8469fc2f4d2b91ce927673ece72af52b360000000000fdffffff13924aee2014590611379bea05b8014db1b36d0ff57e4316776993090b0cd32e0000000000fdffffffae9d91bc47dd6fc4cd5de458cf44bb4ce8621b64884d7f7eea2b4bc579f40d3c0000000000fdffffff3020af577d7ad480876736f45d422e1a9de108a99adaed3649e600fe8ee29d6d0000000000fdffffff716cfb4113b7688528ec37f4852c8dbd3e630f972ff7d684681ad70947d425ab0000000000fdffffffa8d1830d29979120cb405fb94bfd6d1de1ada7090de3f0af39742863798ac1d60000000000fdffffff2da0b6acde8f437b732d92bfbc518328bb033d47eaf09325a6acaf08b88c5e7a0000000000fdffffff68b25e5b370a23953953384d10e2db58130020ff1e3fb54638406ff0c63b8b160000000000fdffffff6bd923ad1a2fdd30151fcca6deb1b8d024e954f20f4092edd0b55cd816dde8be0000000000fdffffff4b4aa6460efc44485c54832982899bac5c19359397cb06bc4234200e1f7a60cb0000000000fdffffff4ee272cbcf27a3d1f3dde1d5d2b032c6d9927091d5ae26c551c2ee0be2076bb00000000000fdffffff36982c0dfafe733324c0aac2f91336e119fee3995bd9773b60e25addf7ffddaf0000000000fdfffffff4b1f1f82cc2beb25ab8b6546cee0249fa5fa3cd4ad5f4d8c64007a7e27689700000000000fdfffffff27c57fda268f622d3603971d57010c5ab5e085e3b6bc466984203f5c5157ed60000000000fdffffffedee335d01f5192f733b710e7abe1a9d977b1c6c72e7fa98a36e020eb92fe0460000000000fdffffff67b234da63365024b01fc2863e1aeb9ca983418a0777eb12dc4ba5f39d19dfdf0000000000fdffffffe4f19c3ecd4488ecbf25b1033c7e9eb1e62d66c27df07fbc215b01497f59864d0000000000fdffffff8d74662196588c262eb029998034f2f66421e532cdcdafdbe33df1c9448ed2580000000000fdffffff163891a199ea582651d31f0d9d254a714de0991cc0b903eccad3d354ffbbe0750000000000fdffffffa3f3029947298312cf52c5b097965d56e4ed2a46d40323e4ec845843521e1db30000000000fdffffff40694097ad3ada4d76d1f6634cd958458b0c7dceb8cbd7522251e83636e093af0000000000fdffffff2ae1699713b4215d2bedb36e2a5fc7abc3ee4b361a411a9b88971d6d1d954f450000000000fdffffff731b2ef5f4209ad0e9339fac724b557369bc999c7f10450e125e647aa970c3d50000000000fdffffff263f0bd6669628ab159d4e919fca5866db3e8448a7613ec6a063db3ed8b73dc90000000000fdffffff1da34fc330fb28a4f4005db0c416686c485937316b95af0895eda353ba8cdb4b0000000000fdffffff89a6e40eb517a24d208d3f8df04df9f9781f64b74e42b8c9523bc5d31cc557060000000000fdffffff583f9fb2e923f7e42d3569faf096e20f6392b608d2db348bd1f8e6ce7889d7480000000000fdffffff1785fa1c5513d6296a80c7caeea88570c3a74ef65ad96370e23c13b5af4156400000000000fdffffff5093048ca5293832dc5a5697a6d6507c105e79cdc46966d1956cea8c9975a4d50000000000fdffffff024da51b63a980bc0c00941d92a9dedca56ed1cce80413014eded8ba5c12c9c40000000000fdffffff2ef34c5b99fc97a24d559bbcec74ac9f4c790705c9772572d2dd1a1b8b6b2a040000000000fdffffff8a3a3be3ed7a6e37d9b88bfa25a11ff16cb4701482df481b6ae28298fe1d284f0000000000fdffffffe47af6cd443982644ae3cf7aa97bbd5ab64d976e04b4af27811cfe3914726df80000000000fdffffffa9282e32b6c4ef6050e1de68ea9e10fcdbd4ddf6fa845f85c8519267cc5599240000000000fdffffff7d1f993505cca88ec6ed3ecce70705aab897b511d652783dc209da57712077d60000000000fdffffff0100e1f505000000002200204a3a5f1583c04e6c45ada0e4724f3e394122ce97d36762d24cc9f6563faee48502473044022058fa3114e6592c5df7b9ae97ff1e9d7958deeecd2c05149bb5ececda3461680b02204a2e6ed5a73d161a598f192475d84d9bf6f6b235dda5caa17bd04d5b52e80be2012103c4ccc78b02d1501306cdfa755028843a1990057d9a620c7ed595b0a2edd8d84c0247304402206f77e5c3f377204d8b74faf9ce0b5d86fe56854221b5f7f364347b0ba770e69e02201aa634d4732a5ba8b5eb30372a024b16ff239cde246b4deb46937a578834d789012103fb223c42db23742f814100edf0f10fd68dcf7c06c9be47fedbc869d36f7c0a4b0247304402206d83f54ab100281740caea2526c2918c076fd4c067a7efed1b884dbd0214141e02200897910c67db6307d1a7be4f0167567166e6e2dd4d1562043e35d3a72e96a172012102db05f48f0bc78b41d6cdcc5dc49bdb4602bca5ddac494227dc4c3be9eafb2f5f0247304402201904b392ca46ec064a2b4ea2f38e02179090b7514460fa5d8425eb2f331ddcbc02207a4ff3bfb6bb24e03e2fe41fe9ce48349b85a0d8d9808de4094d19039f4b7d3c012103c9a997d407838139f4299d7b409a946ce5a9caad8797d434cc5c8623537a0cc202473044022066998a5398e9f8a4a1a8b6b29eb35aaeb82760c0e94d4853bb700d66a2f1709b02207017eaf53330e0e0576c9caf08983360d4eaa23489b13634f93d46bcabb03805012102205a241b24b5817f589f51cece04778273af7aea70df6c977e73067a3c4a7072024730440220591c850057681d9cba1daad507dd96f988a1d7092bebbbb5fc1198df97045fed02207b72a8971621b9f14e17e9b91636f4a23d87f1ce49efb330ca16ecd34392c4bd01210289bbfe3504cb3a6e4cea1454d62af9f675bb9cbd694e96dd3d5024a392d9b1c30247304402203b833ad41b658e2279c4711f87a78eac9875dbf4315874cba7df00828f6f01a202204861ef554dd48ce52267b9406e9731b5891ed4bbf5ebe39cc08a0caf4acec0de0121027be97977b898969706a4968e290f6a6f8a10647e1bf22e287f3deea3462d170702473044022042dbedb1c4e2740e35b249eca21fa3782703b3187ff0899e35af9b11727e763a0220340dcd1f1b3bfb70ad16e3b3c7ce5951e310190fcf40f349ea361b0ee18b70740121027a1aef0e2acce457778ab064c2f0abcda4957461d53d802845580489fabfceaf0247304402207161a423a626062cbe03587dcca31fcfb86974c56a21fa2c568630bdc6b7883d0220605343edc8df241d24f0d823cfe977aa0ada1553df270b45f0be6e924844e589012102e9788c5198c022a9a4f69cbeead802057f9f326bc03263bea83fdad500463b190247304402201bd7d455dc8c21c8e322069f388eb251f874aa30b95fc68d6e3f2934dc518d7c02205030a3cbb92623130592178c2738b36c7f2a879d8b2cd37907e26dbcea4f730601210342fe5565fa3a1cceb68f40d0454919e5fd462413514bc49ba465f186a4c6bb160247304402204063871975f78274a54e9320546f413541e638d471472b9593cfb9162f534dfb02201cb5d2f45d416528eea611448df3b4a6d57a9b3eb1a19dbfb716a061f236b199012102e98030816e495f783b1d7424d297c0504df76bc3afccda34eb62ba52efe7dfcb0247304402202c3c20f73dec9bc22a2193d02237ae27df1442a96526b3da4e16a033b0a9f27802207a77f7b02efb60460fe1ecff88991db9b9c4f96f1c06722710884eaa8c392ba7012102f841c1c8ddf8381589c446cbdf82d8645128a491a309b8a88df772b7159d8c5502473044022030fe5c583f4595d6b7292b1b8f92e0ddf219fb20dffe3aae02f93365900b01300220034d544c92b87833bba86e857203d2ef81007e256b4191b34442fb6055cd06a6012102b4753d320209e003a9d57e33314ad4d6932b56ac2b382ebae02a9cdae14da7810247304402205eb673b5c1cd259fb483150050e06cab1f8e3a7947c805c13028cc3b5f4fc6ba0220520499a1db95dbac1e59b78d3f316d717741c2c8c8cb7e032acbd75241ff420401210329180d74bfac971ae65ff86af8c1d3e41c0b8e2e1c2b3e3aa1bcd7e14e9288250247304402202d16548702a900742b9c230744cad1246a963de284d0fd192d7398d342d384db022020ce246d47f672acd8d8aaedd93cf8e25498cd201176d8352d0e8a6a5a6dd1ff012102761fe84c8c2c867040bce55ffd887cd7cd0b6642fa498f10f3556eee0fa1b0cf0247304402201775ca2939f6b47a7ed0f929feea75dd6b081896bfb3cd5a8b9ffe111e08ccda02200d21ee293337456c73a3735719d4461016f6229588f138dd5256236fbda3036e0121022f25b2bab6d9683840686a12435c6d917642da482f53c5900bdf282c5c2e5e6402473044022020a9e3559c65d09ff142b1345f1d86b75b9bce547bf0001054ae6837bb10f332022041011c9798c5bb386634e6b5213d840e1aa6677032a7fb9fa9b8d25d2fe3e78f0121029e280c81932e45efe17c7e7a80094ea7d469f28321b20b1edcc2aaa971b9838902463043022055d71927aff39ace0af723c62c6390d2594755042647076422b18643ff711f91021f704667cac205bccbc44104f3a3a007467c2d28f3abbe7dd04725aaff60fc2d012102d359a838fafa2650354b1363e6cabb1047896e5bb69987b04663121dbcb05f430247304402200e985f4134f7813507e5505a8e0839b8f64ea73f2ce2e4f5bed4faa2047fb14c02202a083a3bc2f90f0fd35a2e5d2450ff18dea9852f6e29f8f20002c99fdf74a5b0012103af92dce7b5dc9d466cdeb294650cd782588fe565023a7edf8c5469b255f7d05d0247304402205a87fcc2500e944192d6d43100e1c388cbe2059bd6e05fa09a1efec9f1daf40a022054d79ff0f3a0c9d8e723dd11023e3702684acef11e92c749053d5e8393e7aacf0121025a3b8feb4e0627e269318e3911cb808462ec85ef85a1523f7001b730ce4317de0247304402207e93d6832d0c964d2ec7239607bfecd5f376854df930dd651f0783f6acbff84002203dc4d6c923d2a2ca8caf6dcc3faa0379894820e4a47208ddf73d256e48bc7d39012103b2859e1d21036270daf8ffaaf091e01bc9448b4a4b8eb86aee08fbd1a76c91180247304402202fab43d6d5e5bbba35facfc73e2e150e720896991d73bfcc1ea249e8bb52e585022038b9424475037830ef9df7be1420c196b402db1cf74eec8ec75dcc53e36beeb201210331bdd4c5b3cdc49180e8c73506b50e5f0fcfa844120d4b8581502595142df9cb0247304402204d82cb21a0d94a76c18e7fe3edd29f04d08363d2ea38d8af025fc2e8d380e9540220074f4bec8ccfe1e3d357ec277ff510473f15f3ac2c3c679e6b61ac57dc0eb199012103a0d84c2bfe6a02351346a48bf02101f90025d922d54e7387fd8104d1b547f05e0247304402202107442bb3859cdea4fe4a4ad193a1d484ec626c446134d7484949d1eb24e00a02202efd3a51e536f648fb0e7fd9bd146d39a3eec4bed0677ed0ae78f15e2cd0570e0121037007aee609e345cc326e73d0f5e8c57745312eaf7a996bd124be0277339529a5024730440220544c44d4f8aaa2580db795bcc8d4aeda6a21e7f254e08fd540d33972f688dfc202205afa92279c38b60000ed455d49a7c7d2868c54eb3607a6caf49b96fc1f50b3910121023782cf6e2fb8d8b6ce9f99c2bd9040fce131ff35532698d8cde2c55dac7fed5e024730440220144fda3e7e1ae4ba71b7bd1c0310adfe105a5d9a9a0d198e97c32ee5898c1f2b0220662b1b79a4a4794dabf83e194ff9f20acf9ac15e3509f0392f6605639d6c8da70121020d4acd51277e41fd7e75c247a53a12cadd61916fc4310a3f455d10f713d6ccce024730440220595c532bcfc99da5144923d0e4789a4eeceb026850271892131b753b37e747ea02207261138edf7661986f6dd839b80480f817ecd4e4671f26bc75ed68d89b18ca03012103fbf412646f4df17ab52f901487f1a6692095d37daf49ae3b9d0136567d6e13e102473044022067c0412b48ed7aecb424e0833973a19f42a1aebd4380d24bbc8d7eae7742b5ba02207a86929473df0a24a5bcbd34f922cfe0dd851d42db1bc49e0fe7f54e2fb75ed00121036474b5de2b9ed7ac0709662ba881085dbafc6dd953104afc6d74591c2e62981d02473044022043c1f3ec7fff4cf338ef552fbf11919e81cdcc8fda635281bdcb6ed7225c397702202d2624d0f4b7849273398ea51327b1f0f33a043a4a2c071c7547dfa9a0bf8c6b012103ae865feba0856c0b75a1cbe3d772b6698c4bcb55abcf99c14a72c8ccac7b9b6b024730440220448576d7859f3bb758cc085bc120c5497ee9555ffd2def2f061a736d05e7df4402207f3cabb2ae135a6b81282c8f4406253244db6ba0f5559c742f12e879bb1dcf6f0121022b9bfde70f0459c5eaee37cee80e9c31273338cdf11c2aa157603fbccc0f374402473044022074daf74af1924326bd265e29373dd353d93ca080a323fcad58f40961dedd0cd002201cd38d2d6677dd3d5df30a16a34698ed0039a29bcfefe20dacf55b917fc0639901210290aaab17a172f495564484f2d3c4861835370cc5b7d28144827697538768f8480247304402205cd2dad10df693fb5fd0c3d6ab0b9af8b1f2741979c8f6802940d5d0e97f83cc022004c9c48410ffe68dcf428a7929180a33484db62c19086be485bfde256083e64401210378ec10af93e5da836500f359a95a351dfed2eaca346f9852a26f60945641f3f20247304402204b93d907792cc202215ac839b3db5e5251dfa8b71c6ab3ccbd4ee396835bbfad0220436fd911432e99c08eae1ebd7a87e63ade81b9f67291216a6755b9328430e6d8012102fe60ae0974f82120c448003f7c45ec4dbe66750d3523ad2933b147cd7839b570024730440220580d6ac8ba378b6b4421a931e6a6819a666095fe600b5d44ddb5a772ccda75fd022040db4e283a8f5ea0e29322608282378096d0370c137f550e6c6b79328f45f440012102265323bc18f904488ff6531f5164960c0bd89b6f61e9d5e8b0628f2064824a970247304402200c3114449ba61492f389c49390a28956640f147e2e0762d11329dd085eac4f2502207af0be3d01cd162eb67af827089bed564cea4aa7c340de9fba7f8b0410fe0402012102b23c0b520b3aa11c6ef84b7c37238b5b4924df73a5b43106e89bd7f207b4161d024730440220044241f75de73cecd0e5d3840369ad7b44f15fabdc5587aacb25ecea6cbbd4b00220399f0b39bcc40d16a346e1f6fde60614066294e40d77ca282381ef062d45beda0121024fabee04be6ac5cf7652491bf634fa331d38c4019b646a9a2fdc8433831a479602473044022021f2013d20340d63d701f5783145a2df63879b755a3988e8ff95dc6c0c6024ab0220682de70ebc72d1e7e51428f15a1880cd18205ec4d5b90a1260100517776f979f012102aa66a25d6b3aa8851159f4fcacbb24256cdeeb3be8f834c21f80f267207ca0b8024730440220487225931ba13916ab2e334515e0aa66267229c6f6ab2a0b283cface2139cf6002206060b01415e056882e6fb17605043ac84d2513049d8afaffa3d2fc6ff5b4ebce0121027c6730dc93f8bee5d0fb37003170dfe721b2aa2dda4a30086c95c0e624a0c3660247304402205a154b7f91e44d6245ce3c2f07ba65a7daeeb5396372362346750f0648c886eb022005d6d538b231d6bb27db87b4d05dae01e7a8d0fa08f460e2db23323eb66a22430121023d4bbf6dfcc79d4739067c5cd101e9b84ac8559e082dfbd8195c34d9a20041a30247304402203fa92c8b5f02bba97508a07aa4e3e502f7824378462f479a89bbbd195c530d3b02200de58a53640873c785e1d78f82efd828e57733683c357b314485fb85ffaf2a3101210240c31d3c78327d27957b0235f23c7863acb0347cbe346efbcf669cdcc352fc1f0247304402206ef38ea141a5341df344bf0200c74688f4cf162db63f7e7e6d500bf46562395c02200d0c979b0f8e7a54dac9bd94f6439312e09b823dbb25e0d21a8d6d12656ed5ae012103505c9a8b99026d5998d2ce56b5b8538bdc00fc1ee398c790a946365e7a7827900247304402202b60677e7f1fdedb56546cf1641144b6db9ec234c292f8643c2c1f8f76d9f16602202718eee9e8d78ae7adb19c201ef294317e753b7db889438be99971949b5b0288012102983887609a2b686a4971bd782d57184e610ada083b7751ca35ea39e9b3ceec59024730440220643068f56036c0ef4a4abfa6e9a3e135b3d893baf8a8b1e0028f7dc59d68494002207431980c75a24b9db8c10539a7b574f543346ca898732f411518aa87d811842601210302f3a561ff54fd392c98ef3c4da32867cdd46cc7910d3e8db60c1fb01c0bccdfc4320000";

        // 155ad532984baae90e7d4e71fa0c74748c95b2f53742e9ca80946f835c64d7b1 tx merkle proof
        const merkleProof = ["b505ef419130bb92dcf8bc1a1259e9ad2e74a928a11fbe9d4c7c48994d84f0e1", "2130c309027089e6f9272eade461846d07daae53634388ca571550935c9bf9b2"];
        const txId = "155ad532984baae90e7d4e71fa0c74748c95b2f53742e9ca80946f835c64d7b1";

        const txIdBytes = Buffer.from(txId, "hex").reverse();
        const txBytes = Buffer.from(bigTxHex, "hex");
        const txPos = new anchor.BN(1);

        const [txAccount, bump] = anchor.web3.PublicKey.findProgramAddressSync(
            [txIdBytes],
            program.programId
        );

        const ix = await program.methods
            .initBigTxVerify(
                txIdBytes,
                new anchor.BN(txBytes.length),
                1,
                txPos,
                merkleProof.map(e => Buffer.from(e, "hex").reverse()),
                initCommittedHeader
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
        await provider.sendAndConfirm(tx, [signer], {
            skipPreflight: false
        }).catch(e => {
            console.error(e);
            throw e
        });

        const chunkSize = 800;
        for (let i = 0; i < txBytes.length; i += chunkSize) {
            const chunk = txBytes.subarray(i, i + chunkSize);

            const ix = await program.methods
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
            await provider.sendAndConfirm(tx, [signer], {
                skipPreflight: false
            });
        }

        const receiverBalanceBefore = await provider.connection.getBalance(mintReceiver);
        const finalizeIx = await program.methods
            .finalizeTxProcessing(
                txIdBytes
            )
            .accounts({
                signer: signer.publicKey,
                txAccount,
                depositAccount,
                mintReceiver
            })
            .signers([signer])
            .instruction();

        // to increase CU limit
        // .add(ComputeBudgetProgram.setComputeUnitLimit({
        //    units: 1_400_000, // Set the desired number of compute units
        //  }))
        const finalizeTx = new anchor.web3.Transaction().add(ComputeBudgetProgram.setComputeUnitLimit({
            units: 500_000, // Set the desired number of compute units
        })).add(finalizeIx);

        // to catch error and log more details about it
        // .catch(e => console.error(e));
        const finalizeResult = await provider.sendAndConfirm(finalizeTx, [signer], {
            skipPreflight: false
        }).catch(e => {
            console.log(e);
            throw e;
        });

        const latestBlockhash = await provider.connection.getLatestBlockhash();
        await provider.connection.confirmTransaction(
            {
                signature: finalizeResult,
                ...latestBlockhash,
            },
            commitment
        );
        const receiverBalanceAfter = await provider.connection.getBalance(mintReceiver);
        const expectedBalance = receiverBalanceBefore + LAMPORTS_PER_SOL;
        chai.expect(receiverBalanceAfter).eq(expectedBalance);

        // should not allow to verify the same transaction again
        await chai.expect(provider.sendAndConfirm(finalizeTx, [signer], {
            skipPreflight: false
        })).to.be.eventually.rejectedWith("DepositTxAlreadyVerified");
    });

    it("Bridge withdraw", async () => {
        const withdrawAmount = 10 * LAMPORTS_PER_SOL;
        const bitcoinAddress = "bcrt1qm3zxtz0evpc0r5ch3az2ulx0cxce9yjkcs73cq";

        const context = {
            signer: program.provider.publicKey,
            depositAccount,
            systemProgram: SystemProgram.programId
        };

        const depositBalanceBefore = await provider.connection.getBalance(depositAccount);
        await program.methods.bridgeWithdraw(new anchor.BN(withdrawAmount), bitcoinAddress).accounts(context).rpc();

        const depositBalanceAfter = await provider.connection.getBalance(depositAccount);
        chai.expect(depositBalanceAfter).eq(depositBalanceBefore + withdrawAmount);
    });
});
