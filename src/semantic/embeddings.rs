use anyhow::Result;
use hf_hub::api::sync::Api;
use ort::{inputs, session::Session, value::TensorRef};
use std::path::PathBuf;
use tokenizers::Tokenizer;

const MAX_LENGTH: usize = 256;

pub struct VectorStore {
    session: Session,
    tokenizer: Tokenizer,
}

impl VectorStore {
    pub fn new() -> Result<Self> {
        let model_path = download_model()?;
        let tokenizer_path = download_tokenizer()?;

        let session = Session::builder()?.commit_from_file(&model_path)?;
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

        Ok(Self { session, tokenizer })
    }

    pub fn generate_embedding(&mut self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Failed to encode text: {}", e))?;

        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        let mut input_ids_array = ndarray::Array2::<i64>::zeros((1, MAX_LENGTH));
        let mut attention_mask_array = ndarray::Array2::<i64>::zeros((1, MAX_LENGTH));
        let token_type_ids_array = ndarray::Array2::<i64>::zeros((1, MAX_LENGTH));
        let mut attention_mask_f32 = vec![0.0f32; MAX_LENGTH];

        for (i, &id) in input_ids.iter().enumerate().take(MAX_LENGTH) {
            input_ids_array[[0, i]] = id as i64;
        }
        for (i, &mask) in attention_mask.iter().enumerate().take(MAX_LENGTH) {
            attention_mask_array[[0, i]] = mask as i64;
            attention_mask_f32[i] = mask as f32;
        }

        let outputs = self.session.run(inputs![
            "input_ids" => TensorRef::from_array_view(&input_ids_array)?,
            "attention_mask" => TensorRef::from_array_view(&attention_mask_array)?,
            "token_type_ids" => TensorRef::from_array_view(&token_type_ids_array)?,
        ])?;

        let output_array = outputs[0].try_extract_array::<f32>()?;
        let embedding = mean_pool(output_array, &attention_mask_f32);

        Ok(embedding)
    }
}

fn mean_pool(token_embeddings: ndarray::ArrayViewD<f32>, attention_mask: &[f32]) -> Vec<f32> {
    let shape = token_embeddings.shape();
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

    if mask_sum > 0.0 {
        for val in &mut pooled {
            *val /= mask_sum;
        }
    }

    let norm: f32 = pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for val in &mut pooled {
            *val /= norm;
        }
    }

    pooled
}

fn download_model() -> Result<PathBuf> {
    let api = Api::new()?;
    let repo = api.model("sentence-transformers/all-MiniLM-L6-v2".to_string());
    Ok(repo.get("onnx/model.onnx")?)
}

fn download_tokenizer() -> Result<PathBuf> {
    let api = Api::new()?;
    let repo = api.model("sentence-transformers/all-MiniLM-L6-v2".to_string());
    Ok(repo.get("tokenizer.json")?)
}
