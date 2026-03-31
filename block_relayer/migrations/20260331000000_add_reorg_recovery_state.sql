ALTER TABLE utxos ADD COLUMN status TEXT NOT NULL DEFAULT 'confirmed';
ALTER TABLE utxos ADD COLUMN spent_by_txid BLOB;

ALTER TABLE withdraw_transaction_info ADD COLUMN raw_tx BLOB;
ALTER TABLE withdraw_transaction_info ADD COLUMN status TEXT NOT NULL DEFAULT 'broadcasted';

UPDATE withdraw_transaction_info
SET status = 'confirmed'
WHERE raw_tx IS NULL;
