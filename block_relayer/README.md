# Yona Block Relayer

The Block Relayer is a command-line tool serving as a key component interacting with the on-chain Relay Program.

## Key Features

1. **Bitcoin Block Monitoring**: Continuously monitors the Bitcoin network for new blocks.
2. **Header Submission**: Automatically submits new Bitcoin block headers to the Yona network's BTC relay program.
3. **Transaction Relaying**: Provides an HTTP API for Bridge UI to submit users' deposit transaction IDs, which are then
   relayed to the Relay program to complete the BTC minting process on the Yona side.
4. **Command-Line Interface**: Offers various commands to interact with the BTC relay program and perform specific
   functions.

## Configuration

The Block Relayer uses a TOML configuration format. On startup, it attempts to open a `config.toml` file in its working
directory. For detailed configuration options, refer to the [example configuration file](example.toml).

## Usage

The Block Relayer is a command-line tool with several subcommands:

```
block-relayer [SUBCOMMAND]
```

Available subcommands:

- `init-deposit`: Initialize a BTC deposit to the Relay program's PDA (currently unimplemented)
- `init-program`: Initialize the BTC relay program on the Yona network
- `relay-blocks`: Start relaying Bitcoin blocks to the Yona network
- `relay-transactions`: Start the transaction relaying service

## Getting Started

1. Clone the repository:
   ```
   git clone https://github.com/your-repo/yona-block-relayer.git
   cd yona-block-relayer
   ```

2. Create a `config.toml` file in the project root directory, using the provided [example](example.toml) as a
   template.

3. Run the desired command:
   ```
   cargo run -- [SUBCOMMAND]
   ```
