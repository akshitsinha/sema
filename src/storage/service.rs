use crate::search::{SearchResult, SearchService};
use crate::storage::ChunkStorage;
use crate::types::{ChunkConfig, TextChunk};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

pub struct ProcessingService {
    storage: ChunkStorage,
    search: SearchService,
    config: ChunkConfig,
}

impl ProcessingService {
    pub async fn new(config_dir: &Path, chunk_config: ChunkConfig) -> Result<Self> {
        let storage = ChunkStorage::new(config_dir)
            .await
            .context("Failed to initialize chunk storage")?;

        let search = SearchService::new(config_dir)
            .await
            .context("Failed to initialize search service")?;

        Ok(Self {
            storage,
            search,
            config: chunk_config,
        })
    }

    pub async fn process_files(&self, files: Vec<PathBuf>, max_file_size: u64) -> Result<usize> {
        if files.is_empty() {
            return Ok(0);
        }

        let config = self.config.clone();
        let files_to_process = self.filter_files(files).await?;

        if files_to_process.is_empty() {
            return Ok(0);
        }

        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        let batch_size = (files_to_process.len() / num_cpus).clamp(1, 64);

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let handles: Vec<_> = files_to_process
            .chunks(batch_size)
            .map(|batch| {
                let batch = batch.to_vec();
                let config = config.clone();
                let tx = tx.clone();
                tokio::task::spawn_blocking(move || {
                    let chunks: Vec<TextChunk> = batch
                        .into_par_iter()
                        .flat_map(|file_path| {
                            Self::process_file(&file_path, max_file_size, &config)
                        })
                        .collect();
                    if !chunks.is_empty() {
                        let _ = tx.send(chunks);
                    }
                })
            })
            .collect();

        drop(tx);

        let mut all_chunks = Vec::new();
        while let Some(chunks) = rx.recv().await {
            all_chunks.extend(chunks);
        }

        for handle in handles {
            handle.await?;
        }

        if !all_chunks.is_empty() {
            self.storage.store_chunks(&all_chunks).await?;
            self.search.index_chunks(&all_chunks).await?;
        }

        Ok(all_chunks.len())
    }

    async fn filter_files(&self, files: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
        let file_paths: Vec<String> = files
            .par_iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect();

        if file_paths.is_empty() {
            return Ok(Vec::new());
        }

        let stored_hashes = self.storage.get_file_hashes(&file_paths)?;

        let (to_process, to_delete): (Vec<_>, Vec<_>) = files
            .into_par_iter()
            .filter_map(|file_path| {
                let file_path_str = file_path.to_string_lossy().to_string();

                // Compute current hash for the file
                let current_hash = match Self::compute_file_hash(&file_path) {
                    Ok(hash) => hash,
                    Err(_) => return None, // Skip files we can't read
                };

                match stored_hashes.get(&file_path_str) {
                    Some(stored_hash) => {
                        if &current_hash != stored_hash {
                            Some((Some(file_path), Some(file_path_str)))
                        } else {
                            None
                        }
                    }
                    None => Some((Some(file_path), None)),
                }
            })
            .unzip();

        let files_to_delete: Vec<String> = to_delete.into_iter().flatten().collect();
        if !files_to_delete.is_empty() {
            self.delete_chunks(&files_to_delete).await?;
        }

        Ok(to_process.into_iter().flatten().collect())
    }

    async fn delete_chunks(&self, file_paths: &[String]) -> Result<()> {
        if file_paths.is_empty() {
            return Ok(());
        }

        self.storage.delete_chunks_for_files(file_paths)?;
        self.search.remove_chunks_for_files(file_paths).await?;
        Ok(())
    }

    fn process_file(
        file_path: &PathBuf,
        max_file_size: u64,
        config: &ChunkConfig,
    ) -> Vec<TextChunk> {
        let content = match Self::load_file_content(file_path, max_file_size) {
            Ok(content) => content,
            Err(_) => return Vec::new(),
        };

        if content.trim().is_empty() {
            return Vec::new();
        }

        // Compute file hash
        let file_hash = match Self::compute_file_hash(file_path) {
            Ok(hash) => hash,
            Err(_) => return Vec::new(),
        };

        let content_len = content.len();
        if content_len <= config.max_chunk_size {
            let line_count = content.matches('\n').count() + 1;
            return vec![TextChunk::new(
                file_path.clone(),
                0,
                content,
                1,
                line_count,
                Self::detect_language(file_path),
                file_hash,
            )];
        }

        let mut chunks = Vec::new();
        let mut chunk_index = 0;
        let mut current_chunk = String::with_capacity(config.max_chunk_size);
        let mut chunk_start_line = 1;
        let mut current_line = 1;
        let language = Self::detect_language(file_path);

        let lines = content.lines();
        for line in lines {
            let line_with_newline = format!("{}\n", line);

            if current_chunk.len() + line_with_newline.len() > config.max_chunk_size
                && !current_chunk.is_empty()
            {
                chunks.push(TextChunk::new(
                    file_path.clone(),
                    chunk_index,
                    current_chunk,
                    chunk_start_line,
                    current_line - 1,
                    language.clone(),
                    file_hash.clone(),
                ));

                chunk_index += 1;
                current_chunk = String::with_capacity(config.max_chunk_size);

                if config.overlap_size > 0 {
                    let overlap_lines = (config.overlap_size / 50).clamp(1, 5);
                    current_chunk.push_str(&line_with_newline);
                    chunk_start_line = current_line.saturating_sub(overlap_lines - 1);
                } else {
                    chunk_start_line = current_line;
                }
            }

            if current_chunk.len() + line_with_newline.len() <= config.max_chunk_size {
                current_chunk.push_str(&line_with_newline);
            }

            current_line += 1;
        }

        if !current_chunk.trim().is_empty() {
            chunks.push(TextChunk::new(
                file_path.clone(),
                chunk_index,
                current_chunk,
                chunk_start_line,
                current_line - 1,
                language,
                file_hash,
            ));
        }

        chunks
    }

    fn detect_language(file_path: &Path) -> Option<String> {
        file_path
            .extension()
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

    pub async fn clear_all_chunks(&self) -> Result<()> {
        self.storage.clear_all_chunks().await?;
        self.search.clear_index().await?;
        Ok(())
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.search.search(query, limit).await
    }

    pub async fn close(self) {
        self.storage.close().await;
    }

    /// Compute a hash for a file based on its content
    fn compute_file_hash(file_path: &Path) -> Result<String> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let content = std::fs::read_to_string(file_path)?;
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        Ok(format!("{:x}", hasher.finish()))
    }

    /// Load file content with optimized reading for large files
    fn load_file_content(path: &Path, max_size: u64) -> Result<String> {
        let metadata = std::fs::metadata(path)?;

        if metadata.len() == 0 {
            return Ok(String::new());
        }

        // For large files, use buffered reading
        if metadata.len() > 2 * 1024 * 1024 {
            use std::io::BufRead;
            let file = std::fs::File::open(path)?;
            let mut reader = std::io::BufReader::with_capacity(256 * 1024, file);

            let mut content =
                String::with_capacity((metadata.len() as usize).min(max_size as usize));
            let mut buffer = String::with_capacity(16384);

            while reader.read_line(&mut buffer)? > 0 {
                content.push_str(&buffer);
                buffer.clear();
            }
            Ok(content)
        } else {
            // For small files
            std::fs::read_to_string(path).context("Failed to read file")
        }
    }
}
