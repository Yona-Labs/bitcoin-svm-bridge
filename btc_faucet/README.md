# Bitcoin Regtest Faucet

This is a simple Bitcoin faucet application designed for use with a Bitcoin Core node running in regtest mode. It allows
users to request test bitcoins (tBTC) for development and testing purposes.

## Features

- REST API endpoint for requesting funds
- Rate limiting (one request per address per 24 hours)
- Integration with Bitcoin Core RPC
- SQLite database for tracking requests
- Permissive CORS

## Prerequisites

- Rust and Cargo
- Bitcoin Core node running in regtest mode
- SQLite

## Configuration

Before running the application, make sure to configure the following:

1. Bitcoin Core RPC credentials:
    - URL: `http://127.0.0.1:18443` (default for regtest)
    - Username: "test"
    - Password: "test"

   Update these in the `main()` function if your setup differs.

2. Faucet amount:
    - Currently set to 0.1 BTC (10,000,000 satoshis)
    - Modify the `FAUCET_AMOUNT` constant in the `send_funds()` function to change this

3. Server address:
    - Currently binds to `0.0.0.0:8099`
    - Update in the `main()` function if needed

## Building and Running

1. Clone the repository:
   ```
   git clone <repository-url>
   cd bitcoin-regtest-faucet
   ```

2. Build the project:
   ```
   cargo build --release
   ```

3. Run the application:
   ```
   cargo run --release
   ```

The server will start and listen for requests on `http://0.0.0.0:8099`.

## Usage

To request funds, send a GET request to the `/faucet` endpoint with a `address` query parameter:

```
http://localhost:8099/faucet?address=<bitcoin-address>
```

Replace `<bitcoin-address>` with a valid Bitcoin address in your regtest network.

## API Response

- Successful request: Returns HTTP 200 with the transaction ID
- Rate limited: Returns HTTP 429 if the address has already received funds in the last 24 hours
- Error: Returns HTTP 500 with an error message for other issues
