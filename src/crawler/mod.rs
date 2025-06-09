use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use num_cpus;
use tokio::sync::mpsc as async_mpsc;

use crate::types::{CrawlerConfig, FileEntry};

/// File crawler that recursively discovers and processes text files
pub struct FileCrawler {
    config: CrawlerConfig,
}

impl FileCrawler {
    /// Create a new file crawler with the given configuration
    pub fn new(config: CrawlerConfig) -> Self {
        Self { config }
    }

    /// Crawl directory using all CPU cores for maximum speed
    pub async fn crawl_directory(
        &self,
        root_path: &Path,
        file_tx: async_mpsc::UnboundedSender<FileEntry>,
    ) -> Result<()> {
        let root_path = root_path.to_owned();
        let config = self.config.clone();

        // Spawn blocking task for file system operations
        let handle =
            tokio::task::spawn_blocking(move || Self::crawl_blocking(root_path, config, file_tx));

        handle.await.context("Crawler task failed")?
    }

    /// Crawl using all CPU cores - collect paths only for speed
    fn crawl_blocking(
        root_path: PathBuf,
        config: CrawlerConfig,
        file_tx: async_mpsc::UnboundedSender<FileEntry>,
    ) -> Result<()> {
        // Use all available CPU cores for directory traversal
        let mut walker = WalkBuilder::new(&root_path);
        walker
            .follow_links(config.follow_symlinks)
            .hidden(!config.include_hidden)
            .max_filesize(Some(config.max_file_size))
            .threads(num_cpus::get());

        // Configure gitignore handling
        if config.ignore_gitignore {
            walker
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true);
        } else {
            walker
                .git_ignore(false)
                .git_global(false)
                .git_exclude(false);
        }

        // Add exclude patterns
        for pattern in &config.exclude_patterns {
            walker.add_ignore(&format!("!{}", pattern));
        }

        // Process files directly in parallel walker for maximum efficiency
        let walker = walker.build_parallel();
        let file_tx_ref = &file_tx;
        let config_ref = &config;

        walker.run(|| {
            let file_tx = file_tx_ref.clone();

            Box::new(move |entry| {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file() && Self::should_include_file(path, config_ref) {
                        // Check if file is text/UTF-8 before including it
                        if Self::is_text_file(path, config_ref.max_file_size) {
                            // Create and send file entry immediately - no intermediate collection
                            let file_entry = FileEntry {
                                path: path.to_owned(),
                                filename: path
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("unknown")
                                    .to_string(),
                                ..Default::default()
                            };

                            // Send directly from the parallel walker
                            let _ = file_tx.send(file_entry);
                        }
                    }
                }
                ignore::WalkState::Continue
            })
        });

        Ok(())
    }

    /// File filtering with minimal checks for performance
    fn should_include_file(path: &Path, config: &CrawlerConfig) -> bool {
        // If specific extensions are provided, use pattern matching
        if !config.file_extensions.is_empty() {
            // Check for wildcard pattern that includes all files
            if config.file_extensions.iter().any(|ext| ext == "*") {
                return true;
            }

            if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
                let extension = extension.to_lowercase();
                return config.file_extensions.iter().any(|ext| {
                    let ext_lower = ext.to_lowercase();
                    // Support wildcard patterns like "*.rs" or just "rs"
                    if ext_lower.starts_with("*.") {
                        extension == ext_lower.trim_start_matches("*.")
                    } else {
                        extension == ext_lower
                    }
                });
            } else {
                // When extensions are specified but file has no extension,
                // only include if there's a pattern that matches extensionless files
                return config
                    .file_extensions
                    .iter()
                    .any(|ext| ext == "*" || ext == "");
            }
        }

        // If no extensions specified, use sensible defaults for text files
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            let ext = extension.to_lowercase();
            return matches!(
                ext.as_str(),
                "txt"
                    | "rs"
                    | "py"
                    | "js"
                    | "ts"
                    | "jsx"
                    | "tsx"
                    | "html"
                    | "css"
                    | "scss"
                    | "json"
                    | "xml"
                    | "yaml"
                    | "yml"
                    | "toml"
                    | "md"
                    | "markdown"
                    | "rst"
                    | "c"
                    | "cpp"
                    | "h"
                    | "hpp"
                    | "java"
                    | "kt"
                    | "go"
                    | "rb"
                    | "php"
                    | "sh"
                    | "bash"
                    | "zsh"
                    | "fish"
                    | "ps1"
                    | "bat"
                    | "dockerfile"
                    | "vue"
                    | "svelte"
                    | "sass"
                    | "less"
                    | "ini"
                    | "conf"
                    | "cfg"
                    | "log"
                    | "tex"
                    | "sql"
                    | "r"
                    | "swift"
                    | "scala"
                    | "clj"
                    | "elm"
                    | "hs"
                    | "lua"
                    | "pl"
                    | "pm"
                    | "vim"
            );
        }

        // Include files without extension (often config files) when using defaults
        true
    }

    /// Check if a file is a text file (UTF-8 encoded, non-binary)
    fn is_text_file(path: &Path, max_size: u64) -> bool {
        // Check file size first to avoid reading large files
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() > max_size {
                return false;
            }
        } else {
            return false;
        }

        // Quick check: if it has a known text file extension, assume it's text
        // This avoids unnecessary file reading for common text files
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            let ext = extension.to_lowercase();
            if matches!(
                ext.as_str(),
                "txt" | "md" | "markdown" | "rst" | "json" | "xml" | "yaml" | "yml" 
                | "toml" | "ini" | "conf" | "cfg" | "log" | "csv" | "tsv"
                | "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "html" | "css" | "scss"
                | "c" | "cpp" | "h" | "hpp" | "java" | "kt" | "go" | "rb" | "php"
                | "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "dockerfile"
                | "vue" | "svelte" | "sass" | "less" | "tex" | "sql" | "r"
                | "swift" | "scala" | "clj" | "elm" | "hs" | "lua" | "pl" | "pm" | "vim"
            ) {
                return true;
            }
        }

        // For files without known text extensions or unknown extensions,
        // read a small sample to detect if it's binary
        const SAMPLE_SIZE: usize = 8192; // Read first 8KB
        
        match std::fs::read(path) {
            Ok(bytes) => {
                // If file is empty, consider it text
                if bytes.is_empty() {
                    return true;
                }

                // Take a sample from the beginning of the file
                let sample = if bytes.len() <= SAMPLE_SIZE {
                    &bytes
                } else {
                    &bytes[..SAMPLE_SIZE]
                };

                // Check for null bytes (common indicator of binary files)
                if sample.contains(&0) {
                    return false;
                }

                // Try to decode as UTF-8
                match std::str::from_utf8(sample) {
                    Ok(_) => {
                        // If it's valid UTF-8, check if it contains mostly printable characters
                        Self::is_mostly_printable(sample)
                    }
                    Err(_) => {
                        // If it's not valid UTF-8, it's likely binary
                        false
                    }
                }
            }
            Err(_) => false, // If we can't read the file, skip it
        }
    }

    /// Check if the byte content contains mostly printable characters
    fn is_mostly_printable(bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }

        let mut printable_count = 0;
        let mut total_count = 0;

        for &byte in bytes {
            total_count += 1;
            
            // Consider printable: ASCII printable chars and whitespace
            // Note: is_ascii_whitespace() already covers \t, \n, \r and more
            // Note: is_ascii_graphic() covers 0x21-0x7E, which overlaps with 0x20-0x7E
            if byte.is_ascii_graphic() || byte.is_ascii_whitespace() {
                printable_count += 1;
            }
        }

        // If at least 85% of characters are printable, consider it text
        let printable_ratio = printable_count as f64 / total_count as f64;
        printable_ratio >= 0.85
    }
}
