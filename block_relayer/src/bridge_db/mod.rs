use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

mod solana_transaction;
pub use solana_transaction::*;
mod utxo;
pub use utxo::{
    Utxo, UTXO_STATUS_CONFIRMED, UTXO_STATUS_PENDING_CHANGE, UTXO_STATUS_SPENT_PENDING,
};
mod withdraw_transaction_info;
pub use withdraw_transaction_info::{
    PendingWithdrawStats, WithdrawStatus, WithdrawTransactionInfo,
};

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
            status: UTXO_STATUS_CONFIRMED.to_string(),
            spent_by_txid: None,
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
        WithdrawTransactionInfo::add_new(&pool, &solana_signature, &bitcoin_txid, &[1, 2, 3])
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
        assert_eq!(retrieved_info.status, WithdrawStatus::Broadcasted);
        assert_eq!(retrieved_info.raw_tx, Some(vec![1, 2, 3]));
        assert!(retrieved_info.created_at_unix > 0);

        let pending_stats = WithdrawTransactionInfo::pending_stats(&pool).await.unwrap();
        assert_eq!(pending_stats.count, 1);
        assert_eq!(
            pending_stats.oldest_created_at_unix,
            Some(retrieved_info.created_at_unix)
        );

        WithdrawTransactionInfo::set_status(&pool, &solana_signature, WithdrawStatus::Confirmed)
            .await
            .unwrap();

        let pending_stats = WithdrawTransactionInfo::pending_stats(&pool).await.unwrap();
        assert_eq!(pending_stats.count, 0);
        assert_eq!(pending_stats.oldest_created_at_unix, None);
    }

    #[tokio::test]
    async fn test_get_non_finalized_excludes_confirmed_withdrawals() {
        let pool = init_test_pool().await;

        let pending_signature = Signature::new_unique();
        let confirmed_signature = Signature::new_unique();

        let pending_txid =
            Txid::from_str("0000000000000000000000000000000000000000000000000000000000000002")
                .unwrap();
        let confirmed_txid =
            Txid::from_str("0000000000000000000000000000000000000000000000000000000000000003")
                .unwrap();

        WithdrawTransactionInfo::add_new(&pool, &pending_signature, &pending_txid, &[1])
            .await
            .unwrap();
        WithdrawTransactionInfo::add_new(&pool, &confirmed_signature, &confirmed_txid, &[2])
            .await
            .unwrap();
        WithdrawTransactionInfo::set_status(&pool, &confirmed_signature, WithdrawStatus::Confirmed)
            .await
            .unwrap();

        let non_finalized = WithdrawTransactionInfo::get_non_finalized(&pool)
            .await
            .unwrap();

        assert_eq!(non_finalized.len(), 1);
        assert_eq!(non_finalized[0].solana_tx_signature, pending_signature);
        assert_eq!(non_finalized[0].bitcoin_tx_id, pending_txid);
        assert_eq!(non_finalized[0].status, WithdrawStatus::Broadcasted);
    }
}
