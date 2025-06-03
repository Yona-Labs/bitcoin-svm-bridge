use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

#[derive(Debug)]
pub struct Utxo {
    pub txid: [u8; 32],
    pub vout: u32,
    pub amount: u64,
    pub script_pubkey: Vec<u8>,
    pub yona_address: String,
    pub bridge_pubkey: Vec<u8>,
    pub redeem_script: Vec<u8>,
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
        }
    }

    pub async fn insert(&self, pool: &SqlitePool) -> Result<(), sqlx::Error> {
        let query= "INSERT INTO utxos (txid, vout, amount, script_pubkey, yona_address, bridge_pubkey, redeem_script) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";
        sqlx::query(query)
            .bind(self.txid.as_slice())
            .bind(self.vout)
            .bind(self.amount as i64)
            .bind(self.script_pubkey.as_slice())
            .bind(self.yona_address.as_str())
            .bind(self.bridge_pubkey.as_slice())
            .bind(self.redeem_script.as_slice())
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

    pub async fn delete_utxo(pool: &SqlitePool, txid: &[u8], vout: i64) -> Result<(), sqlx::Error> {
        let query = "DELETE FROM utxos WHERE txid = ?1 AND vout = ?2";

        sqlx::query(query)
            .bind(txid)
            .bind(vout)
            .execute(pool)
            .await?;

        Ok(())
    }
}
