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
    pub raw_tx: Option<Vec<u8>>,
    pub status: String,
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
            raw_tx: row.get("raw_tx"),
            status: row.get("status"),
        }
    }

    pub async fn add_new(
        pool: &SqlitePool,
        solana_tx_signature: &Signature,
        bitcoin_tx_id: &Txid,
        raw_tx: &[u8],
    ) -> Result<(), sqlx::Error> {
        let query = "INSERT INTO withdraw_transaction_info (solana_tx_signature, bitcoin_tx_id, raw_tx, status) VALUES (?1, ?2, ?3, ?4)";

        let txid_bytes = bitcoin_tx_id.to_byte_array();

        sqlx::query(query)
            .bind(solana_tx_signature.to_string())
            .bind(&txid_bytes[..])
            .bind(raw_tx)
            .bind("broadcasted")
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

    pub async fn get_non_finalized(
        pool: &SqlitePool,
    ) -> Result<Vec<WithdrawTransactionInfo>, sqlx::Error> {
        let query = "SELECT * FROM withdraw_transaction_info WHERE status != ?1 ORDER BY solana_tx_signature";

        let rows = sqlx::query(query).bind("confirmed").fetch_all(pool).await?;

        Ok(rows.into_iter().map(Self::from_row).collect())
    }

    pub async fn set_status(
        pool: &SqlitePool,
        solana_tx_signature: &Signature,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        let query = "UPDATE withdraw_transaction_info SET status = ?1 WHERE solana_tx_signature = ?2";

        sqlx::query(query)
            .bind(status)
            .bind(solana_tx_signature.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }
}
