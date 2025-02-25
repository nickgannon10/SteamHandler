use sqlx::{Pool, Postgres, postgres::PgPoolOptions, Executor};
use anyhow::Result;
use serde_json::Value;


pub struct Database {
    pub pool: Pool<Postgres>,
}

impl Database {
    /// Create a new connection pool using the provided URL and max connections.
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await?;

        let db = Database { pool };
        db.initialize().await?;
        Ok(db)
    }

    async fn initialize(&self) -> Result<()> {
        let create_table_query = r#"
            CREATE TABLE IF NOT EXISTS liquidity_pools_v4 (
                id SERIAL PRIMARY KEY,
                signature TEXT NOT NULL,
                pool_address TEXT NOT NULL,
                token_mint TEXT NOT NULL,
                raw_transaction JSONB NOT NULL,
                is_active BOOLEAN NOT NULL DEFAULT TRUE,
                death_reason TEXT DEFAULT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );
        "#;
    
        self.pool.execute(create_table_query).await?;
        Ok(())
    }

    pub async fn insert_pool(
        &self,
        signature: &str,
        pool_address: &str,
        token_mint: &str,
        raw_tx: &Value,  // JSONB
    ) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO liquidity_pools_v4 
                (signature, pool_address, token_mint, raw_transaction, is_active, death_reason)
            VALUES 
                ($1, $2, $3, $4, TRUE, NULL)
            "#,
            signature,
            pool_address,
            token_mint,
            raw_tx
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    } 

    pub async fn mark_all_pools_inactive(&self, reason: &str) -> Result<i64> {
        let result = sqlx::query!(
            r#"
            UPDATE liquidity_pools_v4
            SET is_active = FALSE,
                death_reason = $1
            WHERE is_active = TRUE
            "#,
            reason
        )
        .execute(&self.pool)
        .await?;
        
        // Return the number of rows affected
        Ok(result.rows_affected() as i64)
    }
    
}
