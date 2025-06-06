use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use encoding_rs::Encoding;
use ignore::WalkBuilder;
use mime_guess::{MimeGuess, mime};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc as async_mpsc;

use crate::types::{CrawlError, CrawlProgress, CrawlerConfig, FileEntry};

/// File crawler that recursively discovers and processes text files
pub struct FileCrawler {
    config: CrawlerConfig,
}

impl FileCrawler {
    /// Create a new file crawler with the given configuration
    pub fn new(config: CrawlerConfig) -> Self {
        Self { config }
    }

    /// Crawl files in the given directory and send results through the channel
    pub async fn crawl_directory(
        &self,
        root_path: &Path,
        progress_tx: async_mpsc::UnboundedSender<CrawlProgress>,
        file_tx: async_mpsc::UnboundedSender<FileEntry>,
    ) -> Result<()> {
        let root_path = root_path.to_owned();
        let config = self.config.clone();

        // Spawn blocking task for file system operations
        let handle = tokio::task::spawn_blocking(move || {
            Self::crawl_blocking(root_path, config, progress_tx, file_tx)
        });

        handle.await.context("Crawler task failed")?
    }

    /// Blocking implementation of file crawling
    fn crawl_blocking(
        root_path: PathBuf,
        config: CrawlerConfig,
        progress_tx: async_mpsc::UnboundedSender<CrawlProgress>,
        file_tx: async_mpsc::UnboundedSender<FileEntry>,
    ) -> Result<()> {
        let mut walker = WalkBuilder::new(&root_path);
        walker
            .follow_links(config.follow_symlinks)
            .hidden(!config.include_hidden)
            .max_filesize(Some(config.max_file_size));

        // Add exclude patterns
        for pattern in &config.exclude_patterns {
            walker.add_ignore(&format!("!{}", pattern));
        }

        let walker = walker.build();

        let mut progress = CrawlProgress {
            files_discovered: 0,
            files_processed: 0,
            bytes_processed: 0,
            current_file: None,
            errors: Vec::new(),
        };

        for entry in walker {
            match entry {
                Ok(entry) => {
                    let path = entry.path();

                    // Skip directories
                    if path.is_dir() {
                        continue;
                    }

                    progress.files_discovered += 1;
                    progress.current_file = Some(path.to_owned());

                    // Send progress update
                    if let Err(_) = progress_tx.send(progress.clone()) {
                        // Silently break on send errors
                        break;
                    }

                    // Check if file should be processed
                    if Self::should_process_file(path, &config) {
                        match Self::process_file(path, &config) {
                            Ok(file_entry) => {
                                progress.files_processed += 1;
                                progress.bytes_processed += file_entry.size;

                                if let Err(_) = file_tx.send(file_entry) {
                                    // Silently break on send errors
                                    break;
                                }
                            }
                            Err(e) => {
                                let error = CrawlError {
                                    path: path.to_owned(),
                                    error: e.to_string(),
                                };
                                progress.errors.push(error);
                                // Silently continue on file processing errors
                            }
                        }
                    }
                }
                Err(_) => {
                    // Silently continue on path access errors
                }
            }
        }

        // Send final progress
        progress.current_file = None;
        if let Err(_) = progress_tx.send(progress.clone()) {
            // Silently ignore send errors
        }

        Ok(())
    }

    /// Check if a file should be processed based on configuration
    fn should_process_file(path: &Path, config: &CrawlerConfig) -> bool {
        // Check file extension if specified
        if !config.file_extensions.is_empty() {
            if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
                let extension = extension.to_lowercase();
                if !config
                    .file_extensions
                    .iter()
                    .any(|ext| ext.to_lowercase() == extension)
                {
                    return false;
                }
            } else {
                return false;
            }
        }

        // Check MIME type for text files
        let mime_guess = MimeGuess::from_path(path);
        let is_text = mime_guess.iter().any(|mime| {
            mime.type_() == mime::TEXT
                || mime.essence_str() == "application/json"
                || mime.essence_str() == "application/xml"
                || mime.essence_str() == "application/javascript"
        });

        if !is_text {
            // Silently skip non-text files
            return false;
        }

        true
    }

    /// Process a single file and create a FileEntry
    fn process_file(path: &Path, config: &CrawlerConfig) -> Result<FileEntry> {
        let metadata = fs::metadata(path)
            .context(format!("Failed to read metadata for {}", path.display()))?;

        // Check file size
        if metadata.len() > config.max_file_size {
            return Err(anyhow::anyhow!(
                "File too large: {} bytes (max: {})",
                metadata.len(),
                config.max_file_size
            ));
        }

        // Read file content as bytes
        let bytes = fs::read(path).context(format!("Failed to read file {}", path.display()))?;

        // Detect encoding
        let (content, encoding_name) = Self::decode_content(&bytes)?;

        // Calculate hash
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = format!("{:x}", hasher.finalize());

        // Get MIME type
        let mime_type = MimeGuess::from_path(path)
            .first_or_octet_stream()
            .essence_str()
            .to_string();

        let file_entry = FileEntry {
            path: path.to_owned(),
            content,
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            mime_type,
            encoding: encoding_name,
            hash,
        };

        // File processed successfully
        Ok(file_entry)
    }

    /// Decode file content with encoding detection
    fn decode_content(bytes: &[u8]) -> Result<(String, String)> {
        // Try UTF-8 first
        match std::str::from_utf8(bytes) {
            Ok(content) => {
                return Ok((content.to_string(), "UTF-8".to_string()));
            }
            Err(_) => {
                // Fall back to encoding detection
            }
        }

        // Use encoding_rs for detection and conversion
        let (encoding, _) = Encoding::for_bom(bytes).unwrap_or((encoding_rs::UTF_8, 0));

        let (content, _, _had_errors) = encoding.decode(bytes);
        // Silently continue even if there were decoding errors

        Ok((content.into_owned(), encoding.name().to_string()))
    }
}

impl Default for CrawlerConfig {
    fn default() -> Self {
        Self {
            max_file_size: 10 * 1024 * 1024, // 10MB
            follow_symlinks: false,
            include_hidden: false,
            file_extensions: vec![
                "txt".to_string(),
                "md".to_string(),
                "rs".to_string(),
                "py".to_string(),
                "js".to_string(),
                "ts".to_string(),
                "json".to_string(),
                "toml".to_string(),
                "yaml".to_string(),
                "yml".to_string(),
                "xml".to_string(),
                "html".to_string(),
                "css".to_string(),
                "sql".to_string(),
                "log".to_string(),
                "conf".to_string(),
                "cfg".to_string(),
                "ini".to_string(),
            ],
            exclude_patterns: vec![
                "target/".to_string(),
                "node_modules/".to_string(),
                ".git/".to_string(),
                ".svn/".to_string(),
                ".hg/".to_string(),
                "*.lock".to_string(),
                "*.tmp".to_string(),
                "*.cache".to_string(),
            ],
        }
    }
}
