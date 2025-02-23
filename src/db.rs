use sqlx::{Pool, Postgres, postgres::PgPoolOptions, Executor};
use anyhow::Result;

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

    /// Initialize the database schema.
    async fn initialize(&self) -> Result<()> {
        let create_table_query = r#"
            CREATE TABLE IF NOT EXISTS RaydiumLPV4 (
                id SERIAL PRIMARY KEY,
                signature TEXT NOT NULL,
                pool_address TEXT NOT NULL,
                is_vote BOOLEAN NOT NULL DEFAULT FALSE,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
        "#;

        self.pool.execute(create_table_query).await?;
        Ok(())
    }
}
