pub mod service;

use anyhow::Result;
use rocksdb::{DB, Options, WriteBatch};
use std::path::Path;
use std::sync::Arc;

pub struct Database {
    db: Arc<DB>,
}

impl Database {
    pub fn new(path: &Path) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

        // Enable multithreading options
        opts.set_max_background_jobs(6);
        opts.set_max_write_buffer_number(4);
        opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
        opts.set_target_file_size_base(64 * 1024 * 1024); // 64MB
        opts.set_level_zero_file_num_compaction_trigger(4);
        opts.set_level_zero_slowdown_writes_trigger(20);
        opts.set_level_zero_stop_writes_trigger(36);
        opts.set_max_bytes_for_level_base(256 * 1024 * 1024); // 256MB

        let db = Arc::new(DB::open(&opts, path)?);

        Ok(Self { db })
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.db.get(key)?)
    }

    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        Ok(self.db.put(key, value)?)
    }

    pub fn delete(&self, key: &[u8]) -> Result<()> {
        Ok(self.db.delete(key)?)
    }

    pub fn exists(&self, key: &[u8]) -> Result<bool> {
        Ok(self.db.get(key)?.is_some())
    }

    pub fn put_batch(&self, batch: WriteBatch) -> Result<()> {
        Ok(self.db.write(batch)?)
    }

    pub fn create_batch(&self) -> WriteBatch {
        WriteBatch::default()
    }

    // Clone for concurrent access - RocksDB handles thread safety internally
    pub fn clone_handle(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
        }
    }
}
