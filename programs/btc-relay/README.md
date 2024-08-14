# Yona Bitcoin Relay Program

This module is a Solana program implementing an on-chain Bitcoin blocks and transactions relay (aka light-client) needed
for the proper functioning of the Yona Bridge.

## Features

- Accepting and verifying Bitcoin block headers
- Handling blockchain forks of varying lengths
- Accepting and verifying Bitcoin deposit transactions with minting the corresponding bridged amount on Yona
- Handling large Bitcoin transactions via splitting their data to multiple Yona txns
- (TODO) Burning BTC on Yona allowing further withdrawal on the Bitcoin side

## Workflow

- Initialize the program with a known Bitcoin block header
- Deposit the expected bridged amount to the program's PDA
- Relay new blocks using [block relayer](../../block_relayer)
- Once a new deposit transaction is made on Bitcoin, relay it using block relayer's `relay-transaction` mode API

## Block headers verification

The program checks block header consensus rules:

- Correct PoW difficulty target
- Possible difficulty adjustments
- Previous block hash
- Blockhash is lower than target (block's PoW)
- Timestamp is greater than the median of the last 11 blocks
- Timestamp is less than the current time plus 4 hours

## Deposit transaction processing

When a deposit transaction is relayed, the program checks its Merkle inclusion proof and then searches the outputs sent
to deposit script pubkey. When such an output is found, its amount is added to the total BTC that will be minted to the
selected Yona address.

## Credits

This module is forked from https://github.com/adambor/BTCRelay-Sol.
