use sqlx::SqlitePool;

pub async fn insert_solana_transaction(
    pool: &SqlitePool,
    signature: &str,
    block: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO solana_transactions (signature, block)
        VALUES (?, ?)
        "#,
    )
    .bind(signature)
    .bind(block)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn solana_transaction_processed(
    pool: &SqlitePool,
    signature: &str,
) -> Result<bool, sqlx::Error> {
    let row: Option<(bool,)> = sqlx::query_as(
        r#"
        SELECT processed FROM solana_transactions
        WHERE signature = ?
        LIMIT 1
        "#,
    )
    .bind(signature)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(processed,)| processed).unwrap_or(false))
}

pub async fn set_transaction_processed(
    pool: &SqlitePool,
    signature: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE solana_transactions
        SET processed = 1
        WHERE signature = ?
        "#,
    )
    .bind(signature)
    .execute(pool)
    .await?;

    Ok(())
}
