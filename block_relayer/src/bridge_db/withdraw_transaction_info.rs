use anchor_client::solana_sdk::signature::Signature;
use bitcoin::hashes::Hash;
use bitcoin::Txid;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

#[derive(Debug)]
pub struct WithdrawTransactionInfo {
    pub solana_tx_signature: Signature,
    pub bitcoin_tx_id: Txid,
}

impl WithdrawTransactionInfo {
    fn from_row(row: SqliteRow) -> Self {
        let signature_str: String = row.get("solana_tx_signature");
        let solana_tx_signature =
            Signature::from_str(&signature_str).expect("Invalid signature format");

        let bitcoin_tx_id_bytes: Vec<u8> = row.get("bitcoin_tx_id");

        let bitcoin_tx_id = Txid::from_slice(&bitcoin_tx_id_bytes).expect("Invalid txid format");

        WithdrawTransactionInfo {
            solana_tx_signature,
            bitcoin_tx_id,
        }
    }

    pub async fn add_new(
        pool: &SqlitePool,
        solana_tx_signature: &Signature,
        bitcoin_tx_id: &Txid,
    ) -> Result<(), sqlx::Error> {
        let query = "INSERT INTO withdraw_transaction_info (solana_tx_signature, bitcoin_tx_id) VALUES (?1, ?2)";

        // Convert Txid to byte array
        let txid_bytes = bitcoin_tx_id.to_byte_array();

        sqlx::query(query)
            .bind(solana_tx_signature.to_string())
            .bind(&txid_bytes[..])
            .execute(pool)
            .await?;

        Ok(())
    }

    pub async fn get_by_solana_signature(
        pool: &SqlitePool,
        solana_tx_signature: &Signature,
    ) -> Result<Option<WithdrawTransactionInfo>, sqlx::Error> {
        let query = "SELECT * FROM withdraw_transaction_info WHERE solana_tx_signature = ?1";

        let row_opt = sqlx::query(query)
            .bind(solana_tx_signature.to_string())
            .fetch_optional(pool)
            .await?;

        Ok(row_opt.map(Self::from_row))
    }
}
