use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::path::{Path, PathBuf};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind, new_index};

use crate::storage::Database;
use crate::types::Chunk;

pub struct SemanticIndex {
    index: Index,
    index_path: PathBuf,
    embedding_model: TextEmbedding,
}

impl SemanticIndex {
    pub fn new(config_dir: &Path, total_chunks: usize) -> Result<Self> {
        let index_path = config_dir.join("semantic_index.usearch");

        // Initialize fastembed embedding model
        let embedding_model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_show_download_progress(false),
        )?;

        println!("Downloaded model");
        // Ensure the model is initialized (downloads automatically if needed)

        // Create usearch index with proper configurations
        let options = IndexOptions {
            dimensions: 384, // AllMiniLML6V2 model dimensions
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 32,
            expansion_add: 256,
            expansion_search: 128,
            multi: false,
        };

        let index = new_index(&options)?;

        // Reserve space for expected number of chunks
        index.reserve(total_chunks)?;

        // Load existing index if it exists
        if index_path.exists() {
            index.load(index_path.to_str().unwrap())?;
        }

        Ok(Self {
            index,
            index_path,
            embedding_model,
        })
    }

    pub fn process_all_chunks(&mut self, db: &Database) -> Result<()> {
        let iterator = db.iterator();

        println!("Hello");

        for item in iterator {
            match item {
                Ok((key, value)) => {
                    // Only process chunk keys, not file index keys
                    if let Ok(key_str) = String::from_utf8(key.to_vec()) {
                        if key_str.starts_with("chunk:") && !key_str.contains("file_index:") {
                            // Deserialize chunk from database
                            if let Ok((chunk, _)) = bincode::decode_from_slice::<Chunk, _>(
                                &value,
                                bincode::config::standard(),
                            ) {
                                // Generate embedding for chunk content
                                let embeddings = self
                                    .embedding_model
                                    .embed(vec![chunk.content.clone()], None)?;
                                let embedding = &embeddings[0];

                                // Store embedding in usearch index using chunk ID as key
                                // Convert chunk ID to numeric key for usearch
                                let numeric_key = self.chunk_id_to_numeric(&chunk.id);
                                self.index.add(numeric_key, embedding)?;
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }

        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        self.index.save(self.index_path.to_str().unwrap())?;
        Ok(())
    }

    // Convert chunk ID string to numeric key for usearch
    fn chunk_id_to_numeric(&self, chunk_id: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        chunk_id.hash(&mut hasher);
        hasher.finish()
    }
}
