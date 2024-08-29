use log::debug;
use rusqlite::{params, Connection, Result};
use std::path::Path;

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

pub struct UtxoDatabase {
    conn: Connection,
}

const CREATE_STMT: &str = "CREATE TABLE IF NOT EXISTS utxos (
    txid BLOB,
    vout INTEGER,
    amount INTEGER,
    script_pubkey BLOB,
    yona_address VARCHAR(255),
    bridge_pubkey BLOB,
    redeem_script BLOB,
    PRIMARY KEY (txid, vout)
)";

impl UtxoDatabase {
    pub fn new_from_conn(conn: Connection) -> Result<Self> {
        conn.execute(CREATE_STMT, [])?;

        Ok(UtxoDatabase { conn })
    }

    pub fn new_from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;

        conn.execute(CREATE_STMT, [])?;

        Ok(UtxoDatabase { conn })
    }

    pub fn insert_utxo(&self, utxo: &Utxo) -> Result<()> {
        debug!("Inserting utxo {utxo:?}");
        self.conn.execute(
            "INSERT INTO utxos (txid, vout, amount, script_pubkey, yona_address, bridge_pubkey, redeem_script)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                utxo.txid,
                utxo.vout,
                utxo.amount,
                utxo.script_pubkey,
                utxo.yona_address,
                utxo.bridge_pubkey,
                utxo.redeem_script,
            ],
        )?;
        Ok(())
    }

    pub fn get_utxo(&self, txid: &[u8], vout: usize) -> Result<Option<Utxo>> {
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM utxos WHERE txid = ?1 AND vout = ?2")?;
        let utxo_iter = stmt.query_map(params![txid, vout], |row| {
            Ok(Utxo {
                txid: row.get(0)?,
                vout: row.get(1)?,
                amount: row.get(2)?,
                script_pubkey: row.get(3)?,
                yona_address: row.get(4)?,
                bridge_pubkey: row.get(5)?,
                redeem_script: row.get(6)?,
            })
        })?;

        let utxo = utxo_iter.flatten().next();
        Ok(utxo)
    }

    pub fn get_all_utxos(&self) -> Result<Vec<Utxo>> {
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM utxos ORDER BY amount ASC")?;
        let utxo_iter = stmt.query_map([], |row| {
            Ok(Utxo {
                txid: row.get(0)?,
                vout: row.get(1)?,
                amount: row.get(2)?,
                script_pubkey: row.get(3)?,
                yona_address: row.get(4)?,
                bridge_pubkey: row.get(5)?,
                redeem_script: row.get(6)?,
            })
        })?;

        let utxos: Result<Vec<Utxo>> = utxo_iter.collect();
        utxos
    }

    pub fn delete_utxo(&self, txid: &[u8], vout: usize) -> Result<()> {
        self.conn.execute(
            "DELETE FROM utxos WHERE txid = ?1 AND vout = ?2",
            params![txid, vout],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_utxo_database() {
        let db = UtxoDatabase::new_from_conn(Connection::open_in_memory().unwrap()).unwrap();

        let utxo = Utxo {
            txid: [0; 32],
            vout: 0,
            amount: 150000000, // 1.5 BTC in satoshis
            script_pubkey: vec![5, 6, 7, 8],
            yona_address: "yona1234567890".to_string(),
            bridge_pubkey: vec![9, 10, 11, 12],
            redeem_script: vec![13, 14, 15, 16],
        };

        // Test insert
        db.insert_utxo(&utxo).unwrap();

        // Test get
        let retrieved_utxo = db.get_utxo(&[0; 32], 0).unwrap().unwrap();
        assert_eq!(retrieved_utxo.txid, utxo.txid);
        assert_eq!(retrieved_utxo.amount, utxo.amount);
        assert_eq!(retrieved_utxo.yona_address, utxo.yona_address);
        assert_eq!(retrieved_utxo.bridge_pubkey, utxo.bridge_pubkey);

        // Test get_all_utxos
        let all_utxos = db.get_all_utxos().unwrap();
        assert_eq!(all_utxos.len(), 1);

        // Test delete
        db.delete_utxo(&[0; 32], 0).unwrap();
        assert!(db.get_utxo(&[0; 32], 0).unwrap().is_none());
    }
}
