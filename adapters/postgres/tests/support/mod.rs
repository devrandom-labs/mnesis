//! Native fixture identities. Event and transaction assertions stay with each law.

pub async fn key(pool: &sqlx::PgPool, value: &[u8]) -> i64 {
    sqlx::query("INSERT INTO mnesis.keys (value) VALUES ($1) ON CONFLICT ON CONSTRAINT keys_value_key DO NOTHING")
        .bind(value).execute(pool).await.unwrap();
    sqlx::query_scalar("SELECT number FROM mnesis.keys WHERE value = $1")
        .bind(value)
        .fetch_one(pool)
        .await
        .unwrap()
}
