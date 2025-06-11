use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

mod solana_transaction;
pub use solana_transaction::*;
mod utxo;
pub use utxo::Utxo;
mod withdraw_transaction_info;
pub use withdraw_transaction_info::WithdrawTransactionInfo;

pub async fn init_test_pool() -> SqlitePool {
    let connect_options = SqliteConnectOptions::from_str("sqlite::memory:?cache=shared")
        .unwrap()
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1) // set max connections here
        .connect_with(connect_options)
        .await
        .unwrap();

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Can't migrate");

    pool
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_client::solana_sdk::signature::Signature;
    use bitcoin::Txid;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_bridge_database() {
        let _ = env_logger::try_init();

        let pool = init_test_pool().await;

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
        utxo.insert(&pool).await.unwrap();

        // Test get
        let retrieved_utxo = Utxo::get_utxo(&pool, &[0; 32], 0).await.unwrap().unwrap();
        assert_eq!(retrieved_utxo.txid, utxo.txid);
        assert_eq!(retrieved_utxo.amount, utxo.amount);
        assert_eq!(retrieved_utxo.yona_address, utxo.yona_address);
        assert_eq!(retrieved_utxo.bridge_pubkey, utxo.bridge_pubkey);

        // Test get_all_utxos
        let all_utxos = Utxo::get_all_utxos(&pool).await.unwrap();
        assert_eq!(all_utxos.len(), 1);

        // Test delete
        Utxo::delete_utxo(&pool, &[0; 32], 0).await.unwrap();
        assert!(Utxo::get_utxo(&pool, &[0; 32], 0).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_withdraw_transaction_info() {
        let _ = env_logger::try_init();

        let pool = init_test_pool().await;

        // Create test data
        let solana_signature = Signature::new_unique();

        // Create a Bitcoin TxId
        let txid_hex = "0000000000000000000000000000000000000000000000000000000000000001";
        let bitcoin_txid = Txid::from_str(txid_hex).unwrap();

        // Test add_new
        WithdrawTransactionInfo::add_new(&pool, &solana_signature, &bitcoin_txid)
            .await
            .unwrap();

        // Test get_by_solana_signature
        let retrieved_info =
            WithdrawTransactionInfo::get_by_solana_signature(&pool, &solana_signature)
                .await
                .unwrap()
                .unwrap();

        assert_eq!(retrieved_info.solana_tx_signature, solana_signature);
        assert_eq!(retrieved_info.bitcoin_tx_id, bitcoin_txid);
    }
}
