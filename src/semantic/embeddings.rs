use anyhow::Result;
use arrow_array::{
    FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use hf_hub::api::sync::Api;
use lancedb::{Table, connect};
use ort::{inputs, session::Session, value::TensorRef};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokenizers::Tokenizer;

use crate::storage::Database;
use crate::types::Chunk;

pub struct VectorStore {
    session: Session,
    tokenizer: Tokenizer,
    table: Option<Table>,
    db_path: PathBuf,
    // Pre-allocated buffers
    input_ids_array: ndarray::Array2<i64>,
    attention_mask_array: ndarray::Array2<i64>,
    token_type_ids_array: ndarray::Array2<i64>,
    attention_mask_f32: Vec<f32>,
}

impl VectorStore {
    pub fn new(config_dir: &Path, _total_chunks: usize) -> Result<Self> {
        let db_path = config_dir.join("embeddings_lancedb");

        // Download model and tokenizer
        let model_path = download_model()?;
        let tokenizer_path = download_tokenizer()?;

        // Initialize ONNX session
        let session = Session::builder()?.commit_from_file(&model_path)?;

        // Initialize tokenizer
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

        // Pre-allocate buffers
        let max_length = 256;
        let input_ids_array = ndarray::Array2::zeros((1, max_length));
        let attention_mask_array = ndarray::Array2::zeros((1, max_length));
        let token_type_ids_array = ndarray::Array2::zeros((1, max_length));
        let attention_mask_f32 = vec![0.0f32; max_length];

        Ok(Self {
            session,
            tokenizer,
            table: None,
            db_path,
            input_ids_array,
            attention_mask_array,
            token_type_ids_array,
            attention_mask_f32,
        })
    }

    pub async fn process_all_chunks(&mut self, db: &Database) -> Result<()> {
        let mut embeddings_batch = Vec::new();
        let batch_size = 1000;

        // Setup database
        if self.db_path.exists() {
            std::fs::remove_dir_all(&self.db_path)?;
        }
        let lancedb = connect(&self.db_path.to_string_lossy()).execute().await?;

        let iterator = db.iterator();

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
                                let embedding = self.generate_embedding(&chunk.content)?;
                                embeddings_batch.push((
                                    chunk.id.clone(),
                                    chunk.content.clone(),
                                    embedding,
                                ));

                                // Store batch when full
                                if embeddings_batch.len() >= batch_size {
                                    if self.table.is_none() {
                                        self.table = Some(
                                            create_vector_table(&lancedb, &embeddings_batch)
                                                .await?,
                                        );
                                    } else {
                                        add_embeddings_to_table(
                                            self.table.as_ref().unwrap(),
                                            &embeddings_batch,
                                        )
                                        .await?;
                                    }
                                    embeddings_batch.clear();
                                }
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }

        // Store remaining embeddings
        if !embeddings_batch.is_empty() {
            if self.table.is_none() {
                self.table = Some(create_vector_table(&lancedb, &embeddings_batch).await?);
            } else {
                add_embeddings_to_table(self.table.as_ref().unwrap(), &embeddings_batch).await?;
            }
        }

        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        // LanceDB automatically persists data, no explicit save needed
        Ok(())
    }

    pub fn get_db_path(&self) -> &PathBuf {
        &self.db_path
    }

    pub fn generate_embedding(&mut self, text: &str) -> Result<Vec<f32>> {
        let max_length = 256;

        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Failed to encode text: {}", e))?;
        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        // Clear and reuse buffers
        self.attention_mask_f32.fill(0.0);

        // Fill arrays directly
        self.input_ids_array.fill(0);
        self.attention_mask_array.fill(0);
        self.token_type_ids_array.fill(0);

        for (i, &id) in input_ids.iter().enumerate().take(max_length) {
            self.input_ids_array[[0, i]] = id as i64;
        }
        for (i, &mask) in attention_mask.iter().enumerate().take(max_length) {
            self.attention_mask_array[[0, i]] = mask as i64;
            self.attention_mask_f32[i] = mask as f32;
        }

        let outputs = self.session.run(inputs![
            "input_ids" => TensorRef::from_array_view(self.input_ids_array.view())?,
            "attention_mask" => TensorRef::from_array_view(self.attention_mask_array.view())?,
            "token_type_ids" => TensorRef::from_array_view(self.token_type_ids_array.view())?,
        ])?;

        let output_array = outputs[0].try_extract_array::<f32>()?;

        let embedding = mean_pool_ndarray(output_array, &self.attention_mask_f32);

        Ok(embedding)
    }
}

fn mean_pool_ndarray(
    token_embeddings: ndarray::ArrayViewD<f32>,
    attention_mask: &[f32],
) -> Vec<f32> {
    // Ensure it's a 3D array [batch, seq_len, hidden_size]
    let shape = token_embeddings.shape();
    if shape.len() != 3 {
        panic!("Expected 3D tensor, got {}D", shape.len());
    }

    let seq_len = shape[1];
    let hidden_size = shape[2];
    let mut pooled = vec![0.0; hidden_size];
    let mut mask_sum = 0.0;

    for i in 0..seq_len {
        let mask_val = attention_mask[i];
        mask_sum += mask_val;
        for j in 0..hidden_size {
            pooled[j] += token_embeddings[[0, i, j]] * mask_val;
        }
    }

    if mask_sum > 1e-9 {
        for val in &mut pooled {
            *val /= mask_sum;
        }
    }

    // L2 normalize
    let norm: f32 = pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        for val in &mut pooled {
            *val /= norm;
        }
    }

    pooled
}

fn download_model() -> Result<PathBuf> {
    // Suppress any potential stdout output during download
    let api = Api::new()?;
    let repo = api.model("sentence-transformers/all-MiniLM-L6-v2".to_string());
    let model_path = repo.get("onnx/model.onnx")?;
    Ok(model_path)
}

fn download_tokenizer() -> Result<PathBuf> {
    // Suppress any potential stdout output during download
    let api = Api::new()?;
    let repo = api.model("sentence-transformers/all-MiniLM-L6-v2".to_string());
    let tokenizer_path = repo.get("tokenizer.json")?;
    Ok(tokenizer_path)
}

async fn create_vector_table(
    db: &lancedb::Connection,
    embeddings: &[(String, String, Vec<f32>)],
) -> Result<Table> {
    if embeddings.is_empty() {
        return Err(anyhow::anyhow!("No embeddings to store"));
    }

    let embedding_dim = embeddings[0].2.len();

    // Create schema for id + text + fixed-size vector embeddings
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("text", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                embedding_dim as i32,
            ),
            true,
        ),
    ]));

    // Create data arrays
    let batch = create_record_batch(&schema, embeddings)?;

    // Create table with single batch
    let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema.clone());

    let table = db
        .create_table("embeddings", Box::new(batches))
        .execute()
        .await?;

    Ok(table)
}

async fn add_embeddings_to_table(
    table: &Table,
    embeddings: &[(String, String, Vec<f32>)],
) -> Result<()> {
    if embeddings.is_empty() {
        return Ok(());
    }

    let schema = table.schema().await?;
    let batch = create_record_batch(&schema, embeddings)?;

    let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema.clone());

    table.add(Box::new(batches)).execute().await?;
    Ok(())
}

fn create_record_batch(
    schema: &Schema,
    embeddings: &[(String, String, Vec<f32>)],
) -> Result<RecordBatch> {
    let embedding_dim = embeddings[0].2.len();

    // Create data arrays
    let ids: Vec<&str> = embeddings.iter().map(|(id, _, _)| id.as_str()).collect();
    let id_array = StringArray::from(ids);

    let texts: Vec<&str> = embeddings
        .iter()
        .map(|(_, text, _)| text.as_str())
        .collect();
    let text_array = StringArray::from(texts);

    // Create fixed-size list array for embeddings
    let flat_embeddings: Vec<f32> = embeddings
        .iter()
        .flat_map(|(_, _, embedding)| embedding.iter().cloned())
        .collect();

    let values = Float32Array::from(flat_embeddings);
    let vector_array = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        embedding_dim as i32,
        Arc::new(values),
        None,
    );

    // Create record batch
    let batch = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(id_array),
            Arc::new(text_array),
            Arc::new(vector_array),
        ],
    )?;

    Ok(batch)
}
