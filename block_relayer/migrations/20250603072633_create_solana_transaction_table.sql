CREATE TABLE solana_transactions
(
    signature VARCHAR(88) PRIMARY KEY,
    block     INT NOT NULL,
    processed BOOLEAN DEFAULT FALSE
);
