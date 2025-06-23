use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::semantic::embeddings::VectorStore;
use crate::types::Chunk;

pub struct SemanticSearch {
    vector_store: VectorStore,
}

impl SemanticSearch {
    pub fn new(config_dir: &Path, total_chunks: usize) -> Result<Self> {
        let vector_store = VectorStore::new(config_dir, total_chunks)?;
        Ok(Self { vector_store })
    }

    pub async fn process_all_chunks(&mut self, chunks: Vec<Chunk>) -> Result<()> {
        self.vector_store.process_all_chunks(chunks).await
    }

    pub fn save(&self) -> Result<()> {
        self.vector_store.save()
    }

    pub fn get_db_path(&self) -> &PathBuf {
        self.vector_store.get_db_path()
    }

    // TODO: Reimplement search without Database dependency
    // pub async fn search(
    //     &mut self,
    //     query: &str,
    //     limit: usize,
    // ) -> Result<Vec<(Chunk, f32)>> {
    //     // Implementation needed
    //     Ok(Vec::new())
    // }
}
