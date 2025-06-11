pub mod service;

use crate::types::TextChunk;
use anyhow::{Context, Result};
use sqlx::{Row, Sqlite, migrate::MigrateDatabase, sqlite::SqlitePool};
use std::path::Path;

/// SQLite database manager for storing text chunks
pub struct ChunkStorage {
    pub(crate) pool: SqlitePool,
}

impl ChunkStorage {
    /// Create a new chunk storage instance
    pub async fn new(config_dir: &Path) -> Result<Self> {
        // Ensure config directory exists
        tokio::fs::create_dir_all(config_dir)
            .await
            .context("Failed to create config directory")?;

        let db_path = config_dir.join("chunks.db");
        let db_url = format!("sqlite://{}", db_path.display());

        // Create database if it doesn't exist
        if !Sqlite::database_exists(&db_url).await.unwrap_or(false) {
            Sqlite::create_database(&db_url)
                .await
                .context("Failed to create database")?;
        }

        // Connect to database
        let pool = SqlitePool::connect(&db_url)
            .await
            .context("Failed to connect to database")?;

        let storage = Self { pool };

        // Initialize schema
        storage.init_schema().await?;

        Ok(storage)
    }

    /// Initialize database schema
    async fn init_schema(&self) -> Result<()> {
        // Enable maximum performance SQLite settings
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA synchronous = NORMAL")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA cache_size = 10000")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA temp_store = memory")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA mmap_size = 268435456")
            .execute(&self.pool)
            .await?; // 256MB
        sqlx::query("PRAGMA page_size = 4096")
            .execute(&self.pool)
            .await?;

        // Create optimized schema for embedding model inference
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                file_modified_time INTEGER NOT NULL,
                UNIQUE(file_path, chunk_index)
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .context("Failed to create chunks table")?;

        // Add file_modified_time column if it doesn't exist (for existing databases)
        let _ = sqlx::query("ALTER TABLE chunks ADD COLUMN file_modified_time INTEGER DEFAULT 0")
            .execute(&self.pool)
            .await;

        // Create optimized indexes for embedding retrieval
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_file_path ON chunks(file_path)")
            .execute(&self.pool)
            .await?;

        // Index for efficient chunk ordering within files
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_file_chunk ON chunks(file_path, chunk_index)")
            .execute(&self.pool)
            .await?;

        // Index for modification time queries
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_file_modified ON chunks(file_path, file_modified_time)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Store multiple chunks in a batch with optimized performance
    pub async fn store_chunks(&self, chunks: &[TextChunk]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        // Enable optimized SQLite settings for maximum performance
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA synchronous = NORMAL")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA cache_size = 10000")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA temp_store = memory")
            .execute(&self.pool)
            .await?;

        let mut tx = self.pool.begin().await?;

        for chunk in chunks {
            let file_path_str = chunk.file_path.to_string_lossy();
            let modified_timestamp = chunk
                .file_modified_time
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            sqlx::query(
                r#"
                INSERT OR REPLACE INTO chunks 
                (file_path, chunk_index, content, start_line, end_line, file_modified_time)
                VALUES (?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&*file_path_str)
            .bind(chunk.chunk_index as i64)
            .bind(&chunk.content)
            .bind(chunk.start_line as i64)
            .bind(chunk.end_line as i64)
            .bind(modified_timestamp)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    /// Clear all chunks from the database
    pub async fn clear_all_chunks(&self) -> Result<()> {
        sqlx::query("DELETE FROM chunks")
            .execute(&self.pool)
            .await
            .context("Failed to clear all chunks")?;
        Ok(())
    }

    /// Get the total number of chunks in the database
    pub async fn get_chunk_count(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM chunks")
            .fetch_one(&self.pool)
            .await
            .context("Failed to get chunk count")?;

        Ok(row.get("count"))
    }
}
