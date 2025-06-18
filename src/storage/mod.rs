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

        // More aggressive multithreading options for maximum write performance
        let cpu_count = num_cpus::get() as i32;
        opts.set_max_background_jobs((cpu_count * 2).max(8));
        opts.set_max_write_buffer_number(8); // More write buffers
        opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB - larger buffers
        opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB
        opts.set_level_zero_file_num_compaction_trigger(8); // More aggressive compaction
        opts.set_level_zero_slowdown_writes_trigger(32);
        opts.set_level_zero_stop_writes_trigger(64);
        opts.set_max_bytes_for_level_base(512 * 1024 * 1024); // 512MB

        // Additional performance optimizations
        opts.set_allow_concurrent_memtable_write(true);
        opts.set_enable_write_thread_adaptive_yield(true);
        opts.set_max_open_files(10000); // Keep more files open
        opts.set_use_direct_io_for_flush_and_compaction(true); // Skip OS cache for compaction

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

    pub fn iterator(&self) -> rocksdb::DBIterator {
        self.db.iterator(rocksdb::IteratorMode::Start)
    }

    // Clone for concurrent access - RocksDB handles thread safety internally
    pub fn clone_handle(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
        }
    }
}
