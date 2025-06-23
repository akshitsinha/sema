pub mod lance_indexer;
pub mod processor;
pub mod text_indexer;

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::types::Chunk;
use lance_indexer::LanceIndexer;
use processor::FileProcessor;
use text_indexer::TextIndexer;

pub struct StorageManager {
    lance_indexer: LanceIndexer,
    text_indexer: TextIndexer,
}

impl StorageManager {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;

        let lance_indexer = LanceIndexer::new(data_dir).await?;
        let text_indexer = TextIndexer::new(data_dir)?;

        Ok(Self {
            lance_indexer,
            text_indexer,
        })
    }
    pub async fn process_and_index_files(&mut self, files: Vec<PathBuf>) -> Result<usize> {
        let mut files_to_process = Vec::new();

        // Check which files need processing
        for file_path in &files {
            if file_path.exists() {
                let current_hash = Self::calculate_file_hash_from_path(file_path).await?;

                match self.lance_indexer.get_file_index(file_path).await? {
                    Some(file_index) if file_index.hash == current_hash => {
                        // File unchanged, skip
                        continue;
                    }
                    Some(_) => {
                        // File changed, need to reprocess
                        self.lance_indexer.remove_file_chunks(file_path).await?;
                        files_to_process.push(file_path.clone());
                    }
                    None => {
                        // New file, need to process
                        files_to_process.push(file_path.clone());
                    }
                }
            }
        }

        let chunks = FileProcessor::process_files(files_to_process.clone()).await?;
        let chunk_count = chunks.len();

        if !chunks.is_empty() {
            self.index_chunks(&chunks).await?;

            // Update file indices
            for file_path in &files_to_process {
                if let Ok(hash) = Self::calculate_file_hash_from_path(file_path).await {
                    let _ = self.lance_indexer.update_file_index(file_path, &hash).await;
                }
            }
        }

        Ok(chunk_count)
    }

    async fn calculate_file_hash_from_path(file_path: &Path) -> Result<String> {
        let content = tokio::fs::read_to_string(file_path).await?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(content.as_bytes());
        Ok(hasher.finalize().to_hex().to_string())
    }

    pub async fn index_chunks(&mut self, chunks: &[Chunk]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        if let Err(e) = self.lance_indexer.index_chunks(chunks).await {
            eprintln!("Warning: Failed to index chunks in LanceDB: {}", e);
        }

        if let Err(e) = self.text_indexer.index_chunks(chunks) {
            eprintln!("Warning: Failed to index chunks in Tantivy: {}", e);
        }

        Ok(())
    }

    pub async fn search(&mut self, query: &str, limit: usize) -> Result<Vec<(Chunk, f32)>> {
        let query = query.trim();

        if let Some(stripped) = query.strip_prefix('\'') {
            // Text search mode - strip the leading quote
            if !stripped.is_empty() {
                self.text_indexer.search(stripped, limit)
            } else {
                Ok(Vec::new())
            }
        } else {
            // Vector search mode
            let chunks = self.lance_indexer.search(query, limit).await?;
            Ok(chunks.into_iter().map(|c| (c, 1.0)).collect())
        }
    }

    pub async fn close(mut self) {
        if let Err(e) = self.text_indexer.commit() {
            eprintln!("Warning: Failed to commit text index changes: {}", e);
        }
    }
}
