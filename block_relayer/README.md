# Yona Block Relayer

The Block Relayer is a command line tool responsible for interaction with the on-chain BTC relay program. It has the
following features:

- Continuously monitors the Bitcoin network and submits headers once new blocks are mined.
- Provides an HTTP API allowing users to submit their bridge transaction ID. The transaction is then relayed to the Yona
  network, completing the bridge deposit process.
- Provides various commands to call specific functions of the BTC relay program.

## Configuration

Block Relayer uses TOML configuration format. Once tool is started, it tries to open `config.toml` file in its working
directory. Check [example](example.toml) for more details.


