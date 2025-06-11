pub mod service;

use crate::types::TextChunk;
use anyhow::{Context, Result};
use bincode::{Decode, Encode, config};
use rocksdb::{DB, Options, WriteBatch};
use std::path::Path;
use std::sync::Arc;

pub struct ChunkStorage {
    pub(crate) db: Arc<DB>,
}

impl ChunkStorage {
    pub async fn new(config_dir: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(config_dir)
            .await
            .context("Failed to create config directory")?;

        let db_path = config_dir.join("chunks.rocksdb");

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        opts.set_write_buffer_size(64 * 1024 * 1024);
        opts.set_max_write_buffer_number(3);
        opts.set_target_file_size_base(64 * 1024 * 1024);
        opts.set_level_zero_file_num_compaction_trigger(4);
        opts.set_level_zero_slowdown_writes_trigger(20);
        opts.set_level_zero_stop_writes_trigger(36);
        opts.set_max_background_jobs(4);
        opts.set_max_bytes_for_level_base(256 * 1024 * 1024);
        opts.set_max_bytes_for_level_multiplier(10.0);
        opts.set_compaction_style(rocksdb::DBCompactionStyle::Level);

        let db = DB::open(&opts, db_path).context("Failed to open RocksDB database")?;

        Ok(Self { db: Arc::new(db) })
    }

    pub async fn store_chunks(&self, chunks: &[TextChunk]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        let mut batch = WriteBatch::default();

        for chunk in chunks {
            let file_path_str = chunk.file_path.to_string_lossy();
            let key = format!("{}:{}", file_path_str, chunk.chunk_index);

            let chunk_data = ChunkData {
                file_path: file_path_str.to_string(),
                chunk_index: chunk.chunk_index,
                content: chunk.content.clone(),
                start_line: chunk.start_line,
                end_line: chunk.end_line,
                file_hash: chunk.file_hash.clone(),
            };

            let serialized = bincode::encode_to_vec(&chunk_data, config::standard())
                .context("Failed to serialize chunk data")?;

            batch.put(key.as_bytes(), serialized);

            let file_key = format!("file:{}", file_path_str);
            batch.put(file_key.as_bytes(), chunk.file_hash.as_bytes());
        }

        self.db
            .write(batch)
            .context("Failed to write chunks to database")?;

        Ok(())
    }

    pub async fn close(self) {
        drop(self.db);
    }

    pub async fn clear_all_chunks(&self) -> Result<()> {
        let mut batch = WriteBatch::default();
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, _) = item.context("Failed to iterate database")?;
            batch.delete(&key);
        }

        self.db.write(batch).context("Failed to clear all chunks")?;

        Ok(())
    }

    pub async fn get_chunk_count(&self) -> Result<i64> {
        let mut count = 0i64;
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, _) = item.context("Failed to iterate database")?;
            let key_str = String::from_utf8_lossy(&key);
            if !key_str.starts_with("file:") {
                count += 1;
            }
        }

        Ok(count)
    }

    pub fn get_file_hashes(
        &self,
        file_paths: &[String],
    ) -> Result<std::collections::HashMap<String, String>> {
        let mut result = std::collections::HashMap::new();

        for file_path in file_paths {
            let file_key = format!("file:{}", file_path);
            if let Some(value) = self
                .db
                .get(file_key.as_bytes())
                .context("Failed to read from database")?
            {
                if let Ok(hash_str) = String::from_utf8(value) {
                    result.insert(file_path.clone(), hash_str);
                }
            }
        }

        Ok(result)
    }

    pub fn delete_chunks_for_files(&self, file_paths: &[String]) -> Result<()> {
        let mut batch = WriteBatch::default();

        for file_path in file_paths {
            let file_key = format!("file:{}", file_path);
            batch.delete(file_key.as_bytes());

            let prefix = format!("{}:", file_path);
            let mut keys_to_delete = Vec::new();

            let iter = self.db.iterator(rocksdb::IteratorMode::Start);
            for item in iter {
                let (key, _) = item.context("Failed to iterate database")?;
                let key_str = String::from_utf8_lossy(&key);
                if key_str.starts_with(&prefix) {
                    keys_to_delete.push(key.to_vec());
                }
            }

            for key in keys_to_delete {
                batch.delete(&key);
            }
        }

        self.db.write(batch).context("Failed to delete chunks")?;

        Ok(())
    }
}

#[derive(Encode, Decode)]
struct ChunkData {
    file_path: String,
    chunk_index: usize,
    content: String,
    start_line: usize,
    end_line: usize,
    file_hash: String,
}
