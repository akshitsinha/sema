use anyhow::Result;
use arrow_array::{Array, FixedSizeListArray, Float32Array, StringArray};
use futures::TryStreamExt;
use lancedb::connect;
use lancedb::query::{ExecutableQuery, QueryBase};
use std::path::{Path, PathBuf};

use crate::semantic::embeddings::VectorStore;
use crate::storage::Database;
use crate::types::Chunk;

pub struct SemanticSearch {
    vector_store: VectorStore,
}

impl SemanticSearch {
    pub fn new(config_dir: &Path, total_chunks: usize) -> Result<Self> {
        let vector_store = VectorStore::new(config_dir, total_chunks)?;
        Ok(Self { vector_store })
    }

    pub async fn process_all_chunks(&mut self, db: &Database) -> Result<()> {
        self.vector_store.process_all_chunks(db).await
    }

    pub fn save(&self) -> Result<()> {
        self.vector_store.save()
    }

    pub fn get_db_path(&self) -> &PathBuf {
        self.vector_store.get_db_path()
    }

    pub async fn search(
        &mut self,
        query: &str,
        limit: usize,
        db: &Database,
    ) -> Result<Vec<(Chunk, f32)>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Generate embedding for search query
        let query_embedding = self.vector_store.generate_embedding(query)?;

        // Connect to LanceDB
        let db_path = self.vector_store.get_db_path();
        if !db_path.exists() {
            return Ok(Vec::new()); // No embeddings stored yet
        }

        let lancedb = connect(&db_path.to_string_lossy()).execute().await?;
        let table = lancedb.open_table("embeddings").execute().await?;

        // Perform vector search
        let results = table
            .query()
            .nearest_to(query_embedding.clone())?
            .limit(limit)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;

        let mut search_results = Vec::new();

        // Convert results back to chunks
        for batch in results {
            let id_array = batch.column_by_name("id").unwrap();
            let text_array = batch.column_by_name("text").unwrap();
            let vector_array = batch.column_by_name("vector").unwrap();

            let ids = id_array.as_any().downcast_ref::<StringArray>().unwrap();
            let texts = text_array.as_any().downcast_ref::<StringArray>().unwrap();
            let vectors = vector_array
                .as_any()
                .downcast_ref::<FixedSizeListArray>()
                .unwrap();

            for i in 0..ids.len() {
                let chunk_id = ids.value(i);
                let _chunk_text = texts.value(i);

                // Retrieve full chunk from database
                if let Some(chunk) = get_chunk_by_id(db, chunk_id)? {
                    // Calculate similarity score (cosine similarity)
                    let score = calculate_similarity(&query_embedding, vectors, i);
                    search_results.push((chunk, score));
                }
            }
        }

        Ok(search_results)
    }
}

fn get_chunk_by_id(db: &Database, chunk_id: &str) -> Result<Option<Chunk>> {
    let key = format!("chunk:{}", chunk_id);

    match db.get(key.as_bytes()) {
        Ok(Some(value)) => {
            let (chunk, _) =
                bincode::decode_from_slice::<Chunk, _>(&value, bincode::config::standard())?;
            Ok(Some(chunk))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("Database error: {}", e)),
    }
}

fn calculate_similarity(
    query_embedding: &[f32],
    vectors: &FixedSizeListArray,
    index: usize,
) -> f32 {
    let vector_values = vectors.values();
    let vector_data = vector_values
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();

    let embedding_dim = query_embedding.len();
    let start_idx = index * embedding_dim;
    let end_idx = start_idx + embedding_dim;

    let document_embedding = &vector_data.values()[start_idx..end_idx];

    // Calculate cosine similarity
    let mut dot_product = 0.0;
    let mut query_norm = 0.0;
    let mut doc_norm = 0.0;

    for i in 0..embedding_dim {
        dot_product += query_embedding[i] * document_embedding[i];
        query_norm += query_embedding[i] * query_embedding[i];
        doc_norm += document_embedding[i] * document_embedding[i];
    }

    if query_norm == 0.0 || doc_norm == 0.0 {
        return 0.0;
    }

    dot_product / (query_norm.sqrt() * doc_norm.sqrt())
}
