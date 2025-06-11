pub mod service;

use anyhow::{Context, Result};
use sqlx::{migrate::MigrateDatabase, sqlite::SqlitePool, Sqlite, Row};
use std::path::{Path, PathBuf};
use crate::types::TextChunk;

/// SQLite database manager for storing text chunks
pub struct ChunkStorage {
    pub(crate) pool: SqlitePool,
}

impl ChunkStorage {
    /// Create a new chunk storage instance
    pub async fn new(config_dir: &Path) -> Result<Self> {
        // Ensure config directory exists
        tokio::fs::create_dir_all(config_dir).await
            .context("Failed to create config directory")?;

        let db_path = config_dir.join("chunks.db");
        let db_url = format!("sqlite://{}", db_path.display());

        // Create database if it doesn't exist
        if !Sqlite::database_exists(&db_url).await.unwrap_or(false) {
            Sqlite::create_database(&db_url).await
                .context("Failed to create database")?;
        }

        // Connect to database
        let pool = SqlitePool::connect(&db_url).await
            .context("Failed to connect to database")?;

        let storage = Self { pool };
        
        // Initialize schema
        storage.init_schema().await?;
        
        Ok(storage)
    }

    /// Initialize database schema
    async fn init_schema(&self) -> Result<()> {
        // Enable maximum performance SQLite settings
        sqlx::query("PRAGMA journal_mode = WAL").execute(&self.pool).await?;
        sqlx::query("PRAGMA synchronous = NORMAL").execute(&self.pool).await?;
        sqlx::query("PRAGMA cache_size = 10000").execute(&self.pool).await?;
        sqlx::query("PRAGMA temp_store = memory").execute(&self.pool).await?;
        sqlx::query("PRAGMA mmap_size = 268435456").execute(&self.pool).await?; // 256MB
        sqlx::query("PRAGMA page_size = 4096").execute(&self.pool).await?;
        
        // Create optimized schema for embedding model inference
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL,
                file_name TEXT NOT NULL,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                content_hash TEXT NOT NULL UNIQUE,
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
        
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_file_name ON chunks(file_name)")
            .execute(&self.pool)
            .await?;
        
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_content_hash ON chunks(content_hash)")
            .execute(&self.pool)
            .await?;

        // Index for efficient chunk ordering within files
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_file_chunk ON chunks(file_path, chunk_index)")
            .execute(&self.pool)
            .await?;

        // Index for modification time queries
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_file_modified ON chunks(file_path, file_modified_time)")
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Store a text chunk in the database
    pub async fn store_chunk(&self, chunk: &TextChunk) -> Result<i64> {
        let file_path_str = chunk.file_path.to_string_lossy();
        let modified_timestamp = chunk.file_modified_time
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let result = sqlx::query(
            r#"
            INSERT OR REPLACE INTO chunks 
            (file_path, file_name, chunk_index, content, start_line, end_line, content_hash, file_modified_time)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&*file_path_str)
        .bind(&chunk.file_name)
        .bind(chunk.chunk_index as i64)
        .bind(&chunk.content)
        .bind(chunk.start_line as i64)
        .bind(chunk.end_line as i64)
        .bind(&chunk.content_hash)
        .bind(modified_timestamp)
        .bind(&chunk.content_hash)
        .execute(&self.pool)
        .await
        .context("Failed to store chunk")?;

        Ok(result.last_insert_rowid())
    }

    /// Store multiple chunks in a batch with optimized performance
    pub async fn store_chunks(&self, chunks: &[TextChunk]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        
        // Enable optimized SQLite settings for maximum performance
        sqlx::query("PRAGMA journal_mode = WAL").execute(&self.pool).await?;
        sqlx::query("PRAGMA synchronous = NORMAL").execute(&self.pool).await?;
        sqlx::query("PRAGMA cache_size = 10000").execute(&self.pool).await?;
        sqlx::query("PRAGMA temp_store = memory").execute(&self.pool).await?;
        
        let mut tx = self.pool.begin().await?;
        
        for chunk in chunks {
            let file_path_str = chunk.file_path.to_string_lossy();
            let modified_timestamp = chunk.file_modified_time
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            sqlx::query(
                r#"
                INSERT OR REPLACE INTO chunks 
                (file_path, file_name, chunk_index, content, start_line, end_line, content_hash, file_modified_time)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&*file_path_str)
            .bind(&chunk.file_name)
            .bind(chunk.chunk_index as i64)
            .bind(&chunk.content)
            .bind(chunk.start_line as i64)
            .bind(chunk.end_line as i64)
            .bind(&chunk.content_hash)
            .bind(modified_timestamp)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Get chunks for a specific file
    pub async fn get_chunks_for_file(&self, file_path: &Path) -> Result<Vec<TextChunk>> {
        let file_path_str = file_path.to_string_lossy();
        
        let rows = sqlx::query(
            "SELECT id, file_path, file_name, chunk_index, content, start_line, end_line, content_hash, file_modified_time 
             FROM chunks WHERE file_path = ? ORDER BY chunk_index"
        )
        .bind(&*file_path_str)
        .fetch_all(&self.pool)
        .await?;

        let mut chunks = Vec::new();
        for row in rows {
            let modified_timestamp = row.get::<i64, _>("file_modified_time");
            let file_modified_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified_timestamp as u64);
            
            chunks.push(TextChunk {
                id: Some(row.get("id")),
                file_path: PathBuf::from(row.get::<String, _>("file_path")),
                file_name: row.get("file_name"),
                chunk_index: row.get::<i64, _>("chunk_index") as usize,
                content: row.get("content"),
                start_line: row.get::<i64, _>("start_line") as usize,
                end_line: row.get::<i64, _>("end_line") as usize,
                content_hash: row.get("content_hash"),
                file_modified_time,
            });
        }

        Ok(chunks)
    }

    /// Delete all chunks for a specific file
    pub async fn delete_chunks_for_file(&self, file_path: &Path) -> Result<()> {
        let file_path_str = file_path.to_string_lossy();
        
        sqlx::query("DELETE FROM chunks WHERE file_path = ?")
            .bind(&*file_path_str)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get total chunk count
    pub async fn get_chunk_count(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM chunks")
            .fetch_one(&self.pool)
            .await?;
        
        Ok(row.get("count"))
    }

    /// Get all chunks for embedding processing
    pub async fn get_all_chunks(&self) -> Result<Vec<TextChunk>> {
        let rows = sqlx::query(
            "SELECT id, file_path, file_name, chunk_index, content, start_line, end_line, content_hash, file_modified_time 
             FROM chunks ORDER BY file_path, chunk_index"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut chunks = Vec::new();
        for row in rows {
            let modified_time_timestamp = row.get::<i64, _>("file_modified_time");
            let file_modified_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified_time_timestamp as u64);
            
            chunks.push(TextChunk {
                id: Some(row.get("id")),
                file_path: PathBuf::from(row.get::<String, _>("file_path")),
                file_name: row.get("file_name"),
                chunk_index: row.get::<i64, _>("chunk_index") as usize,
                content: row.get("content"),
                start_line: row.get::<i64, _>("start_line") as usize,
                end_line: row.get::<i64, _>("end_line") as usize,
                content_hash: row.get("content_hash"),
                file_modified_time,
            });
        }

        Ok(chunks)
    }

    /// Get all chunks for embedding processing with memory-efficient streaming
    pub async fn get_all_chunks_streaming<F>(&self, mut chunk_processor: F) -> Result<()>
    where
        F: FnMut(TextChunk) -> Result<()>,
    {
        let mut stream = sqlx::query(
            "SELECT id, file_path, file_name, chunk_index, content, start_line, end_line, content_hash, file_modified_time 
             FROM chunks ORDER BY file_path, chunk_index"
        )
        .fetch(&self.pool);

        use futures::TryStreamExt;
        
        while let Some(row) = stream.try_next().await? {
            let modified_time_timestamp = row.get::<i64, _>("file_modified_time");
            let file_modified_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified_time_timestamp as u64);
            
            let chunk = TextChunk {
                id: Some(row.get("id")),
                file_path: PathBuf::from(row.get::<String, _>("file_path")),
                file_name: row.get("file_name"),
                chunk_index: row.get::<i64, _>("chunk_index") as usize,
                content: row.get("content"),
                start_line: row.get::<i64, _>("start_line") as usize,
                end_line: row.get::<i64, _>("end_line") as usize,
                content_hash: row.get("content_hash"),
                file_modified_time,
            };
            
            chunk_processor(chunk)?;
        }

        Ok(())
    }

    /// Get chunks by content hash (useful for deduplication)
    pub async fn get_chunk_by_hash(&self, content_hash: &str) -> Result<Option<TextChunk>> {
        let row = sqlx::query(
            "SELECT id, file_path, file_name, chunk_index, content, start_line, end_line, content_hash, file_modified_time 
             FROM chunks WHERE content_hash = ?"
        )
        .bind(content_hash)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            let modified_time_timestamp = row.get::<i64, _>("file_modified_time");
            let file_modified_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified_time_timestamp as u64);
            
            Ok(Some(TextChunk {
                id: Some(row.get("id")),
                file_path: PathBuf::from(row.get::<String, _>("file_path")),
                file_name: row.get("file_name"),
                chunk_index: row.get::<i64, _>("chunk_index") as usize,
                content: row.get("content"),
                start_line: row.get::<i64, _>("start_line") as usize,
                end_line: row.get::<i64, _>("end_line") as usize,
                content_hash: row.get("content_hash"),
                file_modified_time,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get file summary with chunk counts
    pub async fn get_file_summary(&self) -> Result<Vec<(String, String, usize)>> {
        let rows = sqlx::query(
            "SELECT file_path, file_name, COUNT(*) as chunk_count 
             FROM chunks 
             GROUP BY file_path, file_name 
             ORDER BY file_path"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut summary = Vec::new();
        for row in rows {
            summary.push((
                row.get::<String, _>("file_path"),
                row.get::<String, _>("file_name"),
                row.get::<i64, _>("chunk_count") as usize,
            ));
        }

        Ok(summary)
    }

    /// Check if a file needs processing (doesn't exist in DB or has been modified)
    pub async fn needs_processing(&self, file_path: &Path, file_modified_time: std::time::SystemTime) -> Result<bool> {
        let file_path_str = file_path.to_string_lossy();
        let current_timestamp = file_modified_time
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        
        let row = sqlx::query(
            "SELECT file_modified_time FROM chunks WHERE file_path = ? LIMIT 1"
        )
        .bind(&*file_path_str)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let stored_timestamp = row.get::<i64, _>("file_modified_time");
                // File needs processing if it's been modified since last processing
                Ok(current_timestamp > stored_timestamp)
            }
            None => {
                // File doesn't exist in database, needs processing
                Ok(true)
            }
        }
    }

    /// Close the database connection
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
}
