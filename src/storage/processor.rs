use anyhow::Result;
use blake3::Hasher;
use std::path::{Path, PathBuf};

use crate::types::Chunk;

const CHUNK_SIZE: usize = 1000;
const OVERLAP_SIZE: usize = 100;
const MIN_CHUNK_SIZE: usize = 50;

pub struct FileProcessor;

impl FileProcessor {
    pub async fn process_files(files: Vec<PathBuf>) -> Result<Vec<Chunk>> {
        let mut all_chunks = Vec::new();

        for file_path in files {
            if let Ok(chunks) = Self::process_file(&file_path).await {
                all_chunks.extend(chunks);
            }
        }

        Ok(all_chunks)
    }

    async fn process_file(file_path: &Path) -> Result<Vec<Chunk>> {
        let content = tokio::fs::read_to_string(file_path).await?;
        let file_hash = Self::hash_content(&content);
        let chunks = Self::create_chunks(file_path, &content, &file_hash);
        Ok(chunks)
    }

    fn hash_content(content: &str) -> String {
        let mut hasher = Hasher::new();
        hasher.update(content.as_bytes());
        hasher.finalize().to_hex().to_string()
    }

    fn create_chunks(file_path: &Path, content: &str, file_hash: &str) -> Vec<Chunk> {
        let mut chunks = Vec::new();

        if content.len() < MIN_CHUNK_SIZE {
            return chunks;
        }

        let mut start = 0;
        let mut chunk_id = 0;

        while start < content.len() {
            let end = (start + CHUNK_SIZE).min(content.len());

            let mut safe_end = end;
            while safe_end > start && !content.is_char_boundary(safe_end) {
                safe_end -= 1;
            }

            if safe_end < content.len() {
                if let Some(newline_pos) = content[start..safe_end].rfind('\n') {
                    safe_end = start + newline_pos + 1;
                }
            }

            let chunk_content = &content[start..safe_end];

            if chunk_content.len() >= MIN_CHUNK_SIZE || chunk_id == 0 {
                let start_line = content[..start].matches('\n').count() + 1;
                let end_line = start_line + chunk_content.matches('\n').count();

                chunks.push(Chunk {
                    id: format!("{}:{}", file_hash, chunk_id),
                    file_path: file_path.to_owned(),
                    start_line,
                    end_line,
                    content: chunk_content.to_string(),
                });

                chunk_id += 1;
            }

            let next_start = safe_end.saturating_sub(OVERLAP_SIZE);
            start = if next_start <= start {
                safe_end
            } else {
                next_start
            };

            if start >= content.len() {
                break;
            }
        }

        chunks
    }
}
