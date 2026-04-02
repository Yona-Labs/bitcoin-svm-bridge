ALTER TABLE withdraw_transaction_info ADD COLUMN created_at_unix INTEGER;

UPDATE withdraw_transaction_info
SET created_at_unix = CAST(strftime('%s', 'now') AS INTEGER)
WHERE created_at_unix IS NULL;
