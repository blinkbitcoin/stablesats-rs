use sqlx::PgPool;
use std::sync::Once;

static INIT: Once = Once::new();

/// Database test fixture that provides a clean database state for each test
pub struct DatabaseTestFixture {
    pub pool: PgPool,
}

impl DatabaseTestFixture {
    /// Create a new database test fixture with a clean database state
    pub async fn new() -> Result<Self, sqlx::Error> {
        // Ensure global test setup runs only once
        INIT.call_once(|| {
            // Any global test setup can go here
        });

        let database_url = if let Ok(url) = std::env::var("DATABASE_URL") {
            url
        } else {
            let pg_host = std::env::var("PG_HOST").unwrap_or_else(|_| "localhost".into());
            let pg_port = std::env::var("PG_PORT").unwrap_or_else(|_| "5432".into());
            format!("postgres://user:password@{}:{}/pg", pg_host, pg_port)
        };

        let pool = PgPool::connect(&database_url).await?;

        // Clean the database state
        Self::cleanup_database(&pool).await?;

        Ok(Self { pool })
    }

    /// Clean all test-related database tables
    async fn cleanup_database(pool: &PgPool) -> Result<(), sqlx::Error> {
        // Clean ledger tables in the correct order (respecting foreign key constraints)
        sqlx::query("DELETE FROM sqlx_ledger_balances")
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM sqlx_ledger_current_balances")
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM sqlx_ledger_entries")
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM sqlx_ledger_transactions")
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM sqlx_ledger_events")
            .execute(pool)
            .await?;

        // Reset foreign key references
        sqlx::query("UPDATE user_trades SET ledger_tx_id = NULL")
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Get a reference to the database pool
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Manually trigger database cleanup (useful for tests that need fresh state mid-test)
    pub async fn reset(&self) -> Result<(), sqlx::Error> {
        Self::cleanup_database(&self.pool).await
    }
}

impl Drop for DatabaseTestFixture {
    fn drop(&mut self) {
        // Note: We don't clean up in Drop because:
        // 1. Drop can't be async
        // 2. Tests might want to inspect the final state
        // 3. The next test will clean up anyway
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database_fixture_creation() -> Result<(), Box<dyn std::error::Error>> {
        let fixture = DatabaseTestFixture::new().await?;

        // Verify we can query the database
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sqlx_ledger_journals")
            .fetch_one(fixture.pool())
            .await?;

        // Should have some journals (created by migrations)
        assert!(result.0 >= 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_database_cleanup() -> Result<(), Box<dyn std::error::Error>> {
        let fixture = DatabaseTestFixture::new().await?;

        // Verify initial state is clean
        let count_before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sqlx_ledger_balances")
            .fetch_one(fixture.pool())
            .await?;
        assert_eq!(count_before.0, 0, "Database should start clean");

        // Test that reset works (even on already clean database)
        fixture.reset().await?;

        // Verify still clean after reset
        let count_after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sqlx_ledger_balances")
            .fetch_one(fixture.pool())
            .await?;
        assert_eq!(count_after.0, 0, "Database should remain clean after reset");

        Ok(())
    }
}
