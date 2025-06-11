CREATE TABLE withdraw_transaction_info
(
    solana_tx_signature VARCHAR(88) PRIMARY KEY,
    bitcoin_tx_id       BLOB NOT NULL
);