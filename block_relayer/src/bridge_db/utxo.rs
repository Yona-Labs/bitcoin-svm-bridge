use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

pub const UTXO_STATUS_CONFIRMED: &str = "confirmed";
pub const UTXO_STATUS_PENDING_CHANGE: &str = "pending_change";
pub const UTXO_STATUS_SPENT_PENDING: &str = "spent_pending";

#[derive(Debug)]
pub struct Utxo {
    pub txid: [u8; 32],
    pub vout: u32,
    pub amount: u64,
    pub script_pubkey: Vec<u8>,
    pub yona_address: String,
    pub bridge_pubkey: Vec<u8>,
    pub redeem_script: Vec<u8>,
    pub status: String,
    pub spent_by_txid: Option<[u8; 32]>,
}

impl Utxo {
    fn from_row(row: SqliteRow) -> Self {
        Utxo {
            txid: row.get::<Vec<u8>, _>("txid").try_into().expect("32 bytes"),
            vout: row.get("vout"),
            amount: row.get("amount"),
            script_pubkey: row.get("script_pubkey"),
            yona_address: row.get("yona_address"),
            bridge_pubkey: row.get("bridge_pubkey"),
            redeem_script: row.get("redeem_script"),
            status: row.get("status"),
            spent_by_txid: row
                .get::<Option<Vec<u8>>, _>("spent_by_txid")
                .map(|bytes| bytes.try_into().expect("32 bytes")),
        }
    }

    pub async fn insert(&self, pool: &SqlitePool) -> Result<(), sqlx::Error> {
        let query= "INSERT INTO utxos (txid, vout, amount, script_pubkey, yona_address, bridge_pubkey, redeem_script, status, spent_by_txid) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";
        sqlx::query(query)
            .bind(self.txid.as_slice())
            .bind(self.vout)
            .bind(self.amount as i64)
            .bind(self.script_pubkey.as_slice())
            .bind(self.yona_address.as_str())
            .bind(self.bridge_pubkey.as_slice())
            .bind(self.redeem_script.as_slice())
            .bind(self.status.as_str())
            .bind(self.spent_by_txid.map(|txid| txid.to_vec()))
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn get_utxo(
        pool: &SqlitePool,
        txid: &[u8],
        vout: i64,
    ) -> Result<Option<Utxo>, sqlx::Error> {
        let query = "SELECT * FROM utxos WHERE txid = ?1 AND vout = ?2";

        let row_opt = sqlx::query(query)
            .bind(txid)
            .bind(vout)
            .fetch_optional(pool)
            .await?;

        Ok(row_opt.map(Self::from_row))
    }

    pub async fn get_all_utxos(pool: &SqlitePool) -> Result<Vec<Utxo>, sqlx::Error> {
        let query = "SELECT * FROM utxos ORDER BY amount ASC";

        let rows = sqlx::query(query).fetch_all(pool).await?;

        let utxos = rows.into_iter().map(Self::from_row).collect();

        Ok(utxos)
    }

    pub async fn get_spendable_utxos(pool: &SqlitePool) -> Result<Vec<Utxo>, sqlx::Error> {
        let query = "SELECT * FROM utxos WHERE status = ?1 ORDER BY amount ASC";

        let rows = sqlx::query(query)
            .bind(UTXO_STATUS_CONFIRMED)
            .fetch_all(pool)
            .await?;

        Ok(rows.into_iter().map(Self::from_row).collect())
    }

    pub async fn get_by_spending_txid(
        pool: &SqlitePool,
        spent_by_txid: &[u8],
    ) -> Result<Vec<Utxo>, sqlx::Error> {
        let query = "SELECT * FROM utxos WHERE spent_by_txid = ?1 ORDER BY amount ASC";

        let rows = sqlx::query(query)
            .bind(spent_by_txid)
            .fetch_all(pool)
            .await?;

        Ok(rows.into_iter().map(Self::from_row).collect())
    }

    pub async fn mark_spent_pending(
        pool: &SqlitePool,
        txid: &[u8],
        vout: u32,
        spending_txid: &[u8],
    ) -> Result<(), sqlx::Error> {
        let query = "UPDATE utxos SET status = ?1, spent_by_txid = ?2 WHERE txid = ?3 AND vout = ?4";

        sqlx::query(query)
            .bind(UTXO_STATUS_SPENT_PENDING)
            .bind(spending_txid)
            .bind(txid)
            .bind(vout)
            .execute(pool)
            .await?;

        Ok(())
    }

    pub async fn release_spent_by_txid(
        pool: &SqlitePool,
        spending_txid: &[u8],
    ) -> Result<(), sqlx::Error> {
        let query = "UPDATE utxos SET status = ?1, spent_by_txid = NULL WHERE spent_by_txid = ?2 AND status = ?3";

        sqlx::query(query)
            .bind(UTXO_STATUS_CONFIRMED)
            .bind(spending_txid)
            .bind(UTXO_STATUS_SPENT_PENDING)
            .execute(pool)
            .await?;

        Ok(())
    }

    pub async fn promote_pending_change(
        pool: &SqlitePool,
        txid: &[u8],
    ) -> Result<(), sqlx::Error> {
        let query = "UPDATE utxos SET status = ?1 WHERE txid = ?2 AND status = ?3";

        sqlx::query(query)
            .bind(UTXO_STATUS_CONFIRMED)
            .bind(txid)
            .bind(UTXO_STATUS_PENDING_CHANGE)
            .execute(pool)
            .await?;

        Ok(())
    }

    pub async fn delete_by_txid_and_status(
        pool: &SqlitePool,
        txid: &[u8],
        status: &str,
    ) -> Result<(), sqlx::Error> {
        let query = "DELETE FROM utxos WHERE txid = ?1 AND status = ?2";

        sqlx::query(query)
            .bind(txid)
            .bind(status)
            .execute(pool)
            .await?;

        Ok(())
    }

    pub async fn delete_utxo(pool: &SqlitePool, txid: &[u8], vout: u32) -> Result<(), sqlx::Error> {
        let query = "DELETE FROM utxos WHERE txid = ?1 AND vout = ?2";

        sqlx::query(query)
            .bind(txid)
            .bind(vout)
            .execute(pool)
            .await?;

        Ok(())
    }
}
