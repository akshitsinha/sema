use anyhow::Result;
use std::path::Path;

use super::LanceDBStorage;
use crate::types::Chunk;

/// LanceDB indexer for semantic/vector storage of text chunks
pub struct LanceIndexer {
    storage: LanceDBStorage,
}

impl LanceIndexer {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        let mut storage = LanceDBStorage::new(data_dir).await?;
        storage.init_table().await?;

        Ok(Self { storage })
    }

    pub async fn index_chunks(&mut self, chunks: &[Chunk]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        self.storage.add_chunks(chunks).await
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<Chunk>> {
        self.storage.search_chunks(query, limit).await
    }
}
