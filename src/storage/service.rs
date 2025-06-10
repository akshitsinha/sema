use anyhow::{Context, Result};
use std::path::Path;
use rayon::prelude::*;
use sqlx::Row;
use crate::crawler::FileCrawler;
use crate::storage::ChunkStorage;
use crate::types::{FileEntry, ChunkConfig, TextChunk};

/// Simple service that processes files in parallel and chunks them
pub struct ProcessingService {
    storage: ChunkStorage,
    config: ChunkConfig,
}

impl ProcessingService {
    /// Create a new processing service
    pub async fn new(config_dir: &Path, chunk_config: ChunkConfig) -> Result<Self> {
        let storage = ChunkStorage::new(config_dir).await
            .context("Failed to initialize chunk storage")?;

        Ok(Self { storage, config: chunk_config })
    }

    /// Process files in parallel using all available CPU cores
    pub async fn process_files_parallel(&self, files: Vec<FileEntry>, max_file_size: u64) -> Result<ProcessingStats> {
        if files.is_empty() {
            return Ok(ProcessingStats::default());
        }

        let config = self.config.clone();
        
        // Filter files that need processing using batch database operations
        let files_to_process = self.batch_filter_files(files).await?;
        
        if files_to_process.is_empty() {
            return Ok(ProcessingStats::default());
        }

        // Process all files simultaneously across all CPU cores
        let all_chunks: Vec<Vec<TextChunk>> = tokio::task::spawn_blocking(move || {
            files_to_process
                .into_par_iter()
                .map(|file_entry| {
                    Self::process_single_file(&file_entry, max_file_size, &config)
                })
                .collect()
        }).await?;

        // Collect results and store in bulk
        let mut stats = ProcessingStats::default();
        let mut all_flat_chunks = Vec::new();
        
        for chunk_batch in all_chunks {
            if !chunk_batch.is_empty() {
                stats.files_processed += 1;
                stats.chunks_created += chunk_batch.len();
                all_flat_chunks.extend(chunk_batch);
            }
        }

        // Store all chunks in a single transaction
        if !all_flat_chunks.is_empty() {
            self.storage.store_chunks(&all_flat_chunks).await?;
        }

        Ok(stats)
    }

    /// Batch filter files that need processing with parallel operations
    async fn batch_filter_files(&self, files: Vec<FileEntry>) -> Result<Vec<FileEntry>> {
        let file_paths: Vec<String> = files.iter()
            .map(|f| f.path.to_string_lossy().to_string())
            .collect();
        
        if file_paths.is_empty() {
            return Ok(Vec::new());
        }
        
        // Single query to get all stored file times at once
        let placeholders = file_paths.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let query = format!(
            "SELECT file_path, file_modified_time FROM chunks WHERE file_path IN ({}) GROUP BY file_path",
            placeholders
        );
        
        let mut db_query = sqlx::query(&query);
        for path in &file_paths {
            db_query = db_query.bind(path);
        }
        
        let stored_files = db_query.fetch_all(&self.storage.pool).await?;
        let stored_times: std::collections::HashMap<String, i64> = stored_files
            .into_iter()
            .map(|row| {
                let path: String = row.get("file_path");
                let modified: i64 = row.get("file_modified_time");
                (path, modified)
            })
            .collect();
        
        // Parallel filtering and collect files that need processing
        let needs_processing: Vec<_> = files
            .into_par_iter()
            .filter_map(|file| {
                let file_path_str = file.path.to_string_lossy().to_string();
                let current_timestamp = file.modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                
                match stored_times.get(&file_path_str) {
                    Some(&stored_timestamp) => {
                        if current_timestamp > stored_timestamp {
                            Some((file, true)) // needs deletion
                        } else {
                            None // skip - up to date
                        }
                    }
                    None => Some((file, false)) // new file - no deletion needed
                }
            })
            .collect();

        // Bulk delete operations for modified files only
        let files_needing_deletion: Vec<_> = needs_processing
            .iter()
            .filter_map(|(file, needs_deletion)| {
                if *needs_deletion {
                    Some(file.path.to_string_lossy().to_string())
                } else {
                    None
                }
            })
            .collect();

        if !files_needing_deletion.is_empty() {
            self.bulk_delete_chunks(&files_needing_deletion).await?;
        }

        // Return just the files that need processing
        Ok(needs_processing.into_iter().map(|(file, _)| file).collect())
    }

    /// Bulk delete chunks for multiple files in one operation
    async fn bulk_delete_chunks(&self, file_paths: &[String]) -> Result<()> {
        if file_paths.is_empty() {
            return Ok(());
        }

        // Create SQL IN clause for bulk deletion
        let placeholders = file_paths.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let query = format!("DELETE FROM chunks WHERE file_path IN ({})", placeholders);
        
        let mut db_query = sqlx::query(&query);
        for path in file_paths {
            db_query = db_query.bind(path);
        }
        
        db_query.execute(&self.storage.pool).await?;
        Ok(())
    }

    /// Process a single file (load + chunk)
    fn process_single_file(file_entry: &FileEntry, max_file_size: u64, config: &ChunkConfig) -> Vec<TextChunk> {
        // Use memory-mapped loading for performance
        let content = match FileCrawler::load_file_content(&file_entry.path, max_file_size) {
            Ok(content) => content,
            Err(_) => return Vec::new(),
        };

        if content.trim().is_empty() {
            return Vec::new();
        }

        // Chunk content with pre-allocated capacity
        Self::chunk_content(&file_entry.path, &content, config, file_entry.modified)
    }

    /// Content chunking with minimal allocations
    fn chunk_content(file_path: &Path, content: &str, config: &ChunkConfig, file_modified_time: std::time::SystemTime) -> Vec<TextChunk> {
        // Pre-calculate to avoid repeated work
        let content_len = content.len();
        if content_len <= config.max_chunk_size {
            let line_count = content.lines().count();
            return vec![TextChunk::new(
                file_path.to_path_buf(),
                0,
                content.to_string(),
                1,
                line_count,
                Self::detect_language(file_path),
                file_modified_time,
            )];
        }

        // Pre-allocate with estimated capacity
        let estimated_chunks = (content_len / config.max_chunk_size) + 1;
        let mut chunks = Vec::with_capacity(estimated_chunks);
        
        let lines: Vec<&str> = content.lines().collect();
        let mut current_chunk = String::with_capacity(config.max_chunk_size + 1000);
        let mut chunk_start_line = 1;
        let mut current_line = 1;
        let mut chunk_index = 0;

        for line in lines {
            // Avoid format! allocation - push directly
            current_chunk.push_str(line);
            current_chunk.push('\n');
            
            // Check if chunk is getting too large
            if current_chunk.len() > config.max_chunk_size && current_chunk.len() > line.len() + 1 {
                // Remove the last line we just added
                current_chunk.truncate(current_chunk.len() - line.len() - 1);
                
                // Save current chunk
                chunks.push(TextChunk::new(
                    file_path.to_path_buf(),
                    chunk_index,
                    current_chunk.clone(),
                    chunk_start_line,
                    current_line - 1,
                    Self::detect_language(file_path),
                    file_modified_time,
                ));

                // Start new chunk with overlap if configured
                if config.overlap_size > 0 {
                    current_chunk = Self::create_overlap(&current_chunk, config.overlap_size);
                } else {
                    current_chunk.clear();
                    current_chunk.reserve(config.max_chunk_size + 1000);
                    chunk_start_line = current_line;
                }
                
                // Add the line that didn't fit
                current_chunk.push_str(line);
                current_chunk.push('\n');
                chunk_index += 1;
            }

            current_line += 1;
        }

        // Add the last chunk if it has content
        if !current_chunk.is_empty() {
            chunks.push(TextChunk::new(
                file_path.to_path_buf(),
                chunk_index,
                current_chunk,
                chunk_start_line,
                current_line - 1,
                Self::detect_language(file_path),
                file_modified_time,
            ));
        }

        chunks
    }

    /// Create overlap content from the end of a chunk
    fn create_overlap(chunk: &str, overlap_size: usize) -> String {
        if chunk.len() <= overlap_size {
            return chunk.to_string();
        }

        let chars: Vec<char> = chunk.chars().collect();
        if chars.len() <= overlap_size {
            return chunk.to_string();
        }

        let start_char_idx = chars.len() - overlap_size;
        let overlap_candidate: String = chars[start_char_idx..].iter().collect();

        // Try to find a good break point
        if let Some(last_newline) = overlap_candidate.rfind('\n') {
            overlap_candidate.get(last_newline + 1..).unwrap_or(&overlap_candidate).to_string()
        } else if let Some(last_space) = overlap_candidate.rfind(' ') {
            overlap_candidate.get(last_space + 1..).unwrap_or(&overlap_candidate).to_string()
        } else {
            overlap_candidate
        }
    }

    /// Simple language detection from file extension
    fn detect_language(file_path: &Path) -> Option<String> {
        file_path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                match ext.to_lowercase().as_str() {
                    "rs" => "rust",
                    "py" => "python",
                    "js" => "javascript",
                    "ts" => "typescript",
                    "java" => "java",
                    "c" => "c",
                    "cpp" | "cc" | "cxx" => "cpp",
                    "go" => "go",
                    "rb" => "ruby",
                    "php" => "php",
                    "html" => "html",
                    "css" => "css",
                    "json" => "json",
                    "xml" => "xml",
                    "yaml" | "yml" => "yaml",
                    "md" | "markdown" => "markdown",
                    _ => "text",
                }
                .to_string()
            })
    }

    /// Clear all existing chunks
    pub async fn clear_all_chunks(&self) -> Result<()> {
        self.storage.clear_all_chunks().await
    }

    /// Get processing statistics from the database
    pub async fn get_stats(&self) -> Result<StorageStats> {
        let total_chunks = self.storage.get_chunk_count().await?;
        
        Ok(StorageStats {
            total_chunks: total_chunks as usize,
        })
    }

    /// Close the storage connection
    pub async fn close(self) {
        self.storage.close().await;
    }
}

/// Statistics for file processing
#[derive(Debug, Default)]
pub struct ProcessingStats {
    pub files_processed: usize,
    pub chunks_created: usize,
    pub files_failed: usize,
}

/// Statistics for storage
#[derive(Debug)]
pub struct StorageStats {
    pub total_chunks: usize,
}
