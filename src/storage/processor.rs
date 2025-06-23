use anyhow::Result;
use blake3::Hasher;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::task;

use crate::types::{Chunk, ChunkConfig};

pub struct FileProcessor;

impl FileProcessor {
    pub async fn process_file(file_path: &Path) -> Result<Vec<Chunk>> {
        let metadata = fs::metadata(file_path)?;

        let content = if metadata.len() > 1_000_000 {
            task::spawn_blocking({
                let file_path = file_path.to_owned();
                move || {
                    use std::fs::File;
                    use std::io::Read;
                    let mut file = File::open(file_path)?;
                    let mut content = String::new();
                    file.read_to_string(&mut content)?;
                    Ok::<String, std::io::Error>(content)
                }
            })
            .await??
        } else {
            task::spawn_blocking({
                let file_path = file_path.to_owned();
                move || fs::read_to_string(file_path)
            })
            .await??
        };

        let file_hash = Self::calculate_file_hash(&content);
        let chunks = Self::create_chunks(file_path, &content, &file_hash, &ChunkConfig::default());

        Ok(chunks)
    }

    pub async fn process_files(files: Vec<PathBuf>) -> Result<Vec<Chunk>> {
        let batch_size = (num_cpus::get() * 2).clamp(8, 32);
        let mut all_chunks = Vec::new();

        let batch_futures: Vec<_> = files
            .chunks(batch_size)
            .map(|batch| {
                let batch = batch.to_vec();
                task::spawn(async move {
                    let mut batch_chunks = Vec::new();

                    let file_futures: Vec<_> = batch
                        .into_iter()
                        .map(|file_path| async move { Self::process_file(&file_path).await.ok() })
                        .collect();

                    let file_results = futures::future::join_all(file_futures).await;

                    for chunks in file_results.into_iter().flatten() {
                        if !chunks.is_empty() {
                            batch_chunks.extend(chunks);
                        }
                    }

                    Ok::<Vec<Chunk>, anyhow::Error>(batch_chunks)
                })
            })
            .collect();

        let batch_results = futures::future::join_all(batch_futures).await;

        for chunks in batch_results.into_iter().flatten().flatten() {
            all_chunks.extend(chunks);
        }

        Ok(all_chunks)
    }

    fn calculate_file_hash(content: &str) -> String {
        let mut hasher = Hasher::new();
        hasher.update(content.as_bytes());
        hasher.finalize().to_hex().to_string()
    }

    fn create_chunks(
        file_path: &Path,
        content: &str,
        file_hash: &str,
        chunk_config: &ChunkConfig,
    ) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let content_len = content.len();

        if content_len < chunk_config.min_chunk_size {
            return chunks;
        }

        let mut chunk_id = 0;
        let mut start_byte = 0;

        while start_byte < content_len {
            let original_start = start_byte;
            let chunk_content =
                Self::extract_chunk_content(content, start_byte, chunk_config, &mut start_byte);

            if chunk_content.len() < chunk_config.min_chunk_size && chunk_id > 0 {
                break;
            }

            let chunk = Self::create_chunk(
                file_path,
                file_hash,
                chunk_id,
                &chunk_content,
                content,
                original_start,
            );

            chunks.push(chunk);
            chunk_id += 1;
        }

        chunks
    }

    fn extract_chunk_content(
        content: &str,
        start_byte: usize,
        chunk_config: &ChunkConfig,
        next_start_byte: &mut usize,
    ) -> String {
        let content_len = content.len();
        let target_end = (start_byte + chunk_config.chunk_size).min(content_len);
        let mut actual_end = target_end;

        while actual_end > start_byte && !content.is_char_boundary(actual_end) {
            actual_end -= 1;
        }

        if actual_end < content_len {
            if let Some(slice) = content.get(start_byte..actual_end) {
                if let Some(newline_pos) = slice.rfind('\n') {
                    let newline_byte_pos = start_byte + newline_pos + 1;
                    if content.is_char_boundary(newline_byte_pos) {
                        actual_end = newline_byte_pos;
                    }
                }
            }
        }

        let chunk_content = content
            .get(start_byte..actual_end)
            .unwrap_or("")
            .to_string();

        let overlap = chunk_config.overlap_size.min((actual_end - start_byte) / 2);
        let next_start = actual_end.saturating_sub(overlap);

        let mut safe_next_start = next_start;
        while safe_next_start > start_byte && !content.is_char_boundary(safe_next_start) {
            safe_next_start -= 1;
        }

        *next_start_byte = if safe_next_start <= start_byte {
            actual_end
        } else {
            safe_next_start
        };

        chunk_content
    }

    fn create_chunk(
        file_path: &Path,
        file_hash: &str,
        chunk_id: usize,
        chunk_content: &str,
        full_content: &str,
        chunk_start_byte: usize,
    ) -> Chunk {
        let start_line = full_content[..chunk_start_byte].matches('\n').count() + 1;
        let end_line = start_line + chunk_content.matches('\n').count();

        Chunk {
            id: format!("{}:{}", file_hash, chunk_id),
            file_path: file_path.to_owned(),
            start_line,
            end_line,
            content: chunk_content.to_string(),
        }
    }
}
