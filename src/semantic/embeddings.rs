use anyhow::Result;
use hf_hub::api::sync::Api;
use ort::{inputs, session::Session, value::TensorRef};
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

use crate::types::Chunk;

pub struct VectorStore {
    session: Session,
    tokenizer: Tokenizer,
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
            db_path,
            input_ids_array,
            attention_mask_array,
            token_type_ids_array,
            attention_mask_f32,
        })
    }

    // TODO: Reimplement to work with LanceDB directly instead of old Database
    pub async fn process_all_chunks(&mut self, _chunks: Vec<Chunk>) -> Result<()> {
        // Commented out until we can process chunks from LanceDB
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
