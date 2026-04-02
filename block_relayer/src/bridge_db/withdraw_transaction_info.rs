use anchor_client::solana_sdk::signature::Signature;
use bitcoin::hashes::Hash;
use bitcoin::Txid;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawStatus {
    Broadcasted,
    Confirmed,
}

impl WithdrawStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Broadcasted => "broadcasted",
            Self::Confirmed => "confirmed",
        }
    }

    fn from_db(value: &str) -> Self {
        match value {
            "broadcasted" => Self::Broadcasted,
            "confirmed" => Self::Confirmed,
            other => panic!("invalid withdraw status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingWithdrawStats {
    pub count: i64,
    pub oldest_created_at_unix: Option<i64>,
}

#[derive(Debug)]
pub struct WithdrawTransactionInfo {
    pub solana_tx_signature: Signature,
    pub bitcoin_tx_id: Txid,
    pub raw_tx: Option<Vec<u8>>,
    pub status: WithdrawStatus,
    pub created_at_unix: i64,
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
            status: WithdrawStatus::from_db(&row.get::<String, _>("status")),
            created_at_unix: row.get("created_at_unix"),
        }
    }

    pub async fn add_new(
        pool: &SqlitePool,
        solana_tx_signature: &Signature,
        bitcoin_tx_id: &Txid,
        raw_tx: &[u8],
    ) -> Result<(), sqlx::Error> {
        let query = "INSERT INTO withdraw_transaction_info (solana_tx_signature, bitcoin_tx_id, raw_tx, status, created_at_unix) VALUES (?1, ?2, ?3, ?4, ?5)";

        let txid_bytes = bitcoin_tx_id.to_byte_array();
        let created_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("unix time")
            .as_secs() as i64;

        sqlx::query(query)
            .bind(solana_tx_signature.to_string())
            .bind(&txid_bytes[..])
            .bind(raw_tx)
            .bind(WithdrawStatus::Broadcasted.as_str())
            .bind(created_at_unix)
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

        let rows = sqlx::query(query)
            .bind(WithdrawStatus::Confirmed.as_str())
            .fetch_all(pool)
            .await?;

        Ok(rows.into_iter().map(Self::from_row).collect())
    }

    pub async fn set_status(
        pool: &SqlitePool,
        solana_tx_signature: &Signature,
        status: WithdrawStatus,
    ) -> Result<(), sqlx::Error> {
        let query =
            "UPDATE withdraw_transaction_info SET status = ?1 WHERE solana_tx_signature = ?2";

        sqlx::query(query)
            .bind(status.as_str())
            .bind(solana_tx_signature.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    pub async fn count_non_finalized(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM withdraw_transaction_info WHERE status != ?1")
                .bind(WithdrawStatus::Confirmed.as_str())
                .fetch_one(pool)
                .await?;

        Ok(row.0)
    }

    pub async fn oldest_non_finalized_created_at(
        pool: &SqlitePool,
    ) -> Result<Option<i64>, sqlx::Error> {
        let oldest_created_at_unix = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MIN(created_at_unix) FROM withdraw_transaction_info WHERE status != ?1",
        )
        .bind(WithdrawStatus::Confirmed.as_str())
        .fetch_one(pool)
        .await?;

        Ok(oldest_created_at_unix)
    }

    pub async fn pending_stats(pool: &SqlitePool) -> Result<PendingWithdrawStats, sqlx::Error> {
        let count = Self::count_non_finalized(pool).await?;
        let oldest_created_at_unix = Self::oldest_non_finalized_created_at(pool).await?;

        Ok(PendingWithdrawStats {
            count,
            oldest_created_at_unix,
        })
    }
}
