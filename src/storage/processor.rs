use anyhow::Result;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::types::Chunk;

const CHUNK_SIZE: usize = 1000;
const OVERLAP_SIZE: usize = 100;
const MIN_CHUNK_SIZE: usize = 50;
const STREAM_THRESHOLD: u64 = 1_048_576; // 1MB

pub struct FileProcessor;

impl FileProcessor {
    pub fn process_files(files: Vec<PathBuf>) -> Result<Vec<Chunk>> {
        use rayon::prelude::*;

        let all_chunks: Vec<Chunk> = files
            .par_iter()
            .filter_map(|file_path| Self::process_file(file_path).ok())
            .flatten()
            .collect();

        Ok(all_chunks)
    }

    fn process_file(file_path: &Path) -> Result<Vec<Chunk>> {
        let metadata = std::fs::metadata(file_path)?;

        if metadata.len() <= STREAM_THRESHOLD {
            let content = std::fs::read_to_string(file_path)?;
            Ok(Self::chunk(file_path, &content))
        } else {
            Self::read_large_file(file_path)
        }
    }

    fn chunk(file_path: &Path, content: &str) -> Vec<Chunk> {
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
                    id: format!("{}:{}", file_path.to_string_lossy(), chunk_id),
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

    fn read_large_file(file_path: &Path) -> Result<Vec<Chunk>> {
        let file = File::open(file_path)?;
        let reader = BufReader::with_capacity(8192, file);

        let mut chunks = Vec::new();
        let mut buffer = String::new();
        let mut chunk_id = 0;
        let mut current_line = 1;

        for line_result in reader.lines() {
            let line = line_result?;
            buffer.push_str(&line);
            buffer.push('\n');

            while buffer.len() >= CHUNK_SIZE + OVERLAP_SIZE {
                let chunk_end = CHUNK_SIZE.min(buffer.len());

                let mut safe_end = chunk_end;
                while safe_end > 0 && !buffer.is_char_boundary(safe_end) {
                    safe_end -= 1;
                }

                if safe_end < buffer.len() {
                    if let Some(newline_pos) = buffer[..safe_end].rfind('\n') {
                        safe_end = newline_pos + 1;
                    }
                }

                let chunk_content = &buffer[..safe_end];

                if chunk_content.len() >= MIN_CHUNK_SIZE {
                    let chunk_line_count = chunk_content.matches('\n').count();

                    chunks.push(Chunk {
                        id: format!("{}:{}", file_path.to_string_lossy(), chunk_id),
                        file_path: file_path.to_owned(),
                        start_line: current_line,
                        end_line: current_line + chunk_line_count,
                        content: chunk_content.to_string(),
                    });

                    chunk_id += 1;
                    current_line += chunk_line_count;
                }

                let overlap_start = safe_end.saturating_sub(OVERLAP_SIZE);
                buffer = buffer[overlap_start..].to_string();

                if overlap_start > 0 {
                    let overlap_lines = buffer[..overlap_start].matches('\n').count();
                    current_line = current_line.saturating_sub(overlap_lines);
                }
            }
        }

        if !buffer.is_empty() && buffer.len() >= MIN_CHUNK_SIZE {
            let chunk_line_count = buffer.matches('\n').count();

            chunks.push(Chunk {
                id: format!("{}:{}", file_path.to_string_lossy(), chunk_id),
                file_path: file_path.to_owned(),
                start_line: current_line,
                end_line: current_line + chunk_line_count,
                content: buffer,
            });
        }

        Ok(chunks)
    }
}
