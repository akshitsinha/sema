use std::path::{Path, PathBuf};
use std::fs::{File, Metadata};
use std::ffi::OsStr;
use memmap2::Mmap;
use std::sync::Arc;

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use num_cpus;
use tokio::sync::mpsc as async_mpsc;

use crate::types::{CrawlerConfig, FileEntry};

/// File crawler that finds text files in directories.
pub struct FileCrawler {
    config: CrawlerConfig,
}

impl FileCrawler {
    /// Creates a new file crawler with the given configuration.
    pub fn new(config: CrawlerConfig) -> Self {
        Self { config }
    }

    /// Crawls a directory and sends found files through the channel.
    pub async fn crawl_directory(
        &self,
        root_path: &Path,
        file_tx: async_mpsc::UnboundedSender<FileEntry>,
    ) -> Result<()> {
        let root_path = root_path.to_owned();
        let config = Arc::new(self.config.clone());

        tokio::task::spawn_blocking(move || {
            Self::crawl_with_batching(root_path, config, file_tx)
        })
        .await
        .context("Crawler task failed")?
    }

    /// Fast parallel crawling with maximum CPU utilization.
    fn crawl_with_batching(
        root_path: PathBuf,
        config: Arc<CrawlerConfig>,
        file_tx: async_mpsc::UnboundedSender<FileEntry>,
    ) -> Result<()> {
        // Use maximum CPU cores for directory traversal
        let thread_count = (num_cpus::get() * 2).max(8);
        let mut walker = WalkBuilder::new(&root_path);
        
        walker
            .follow_links(config.follow_symlinks)
            .hidden(!config.include_hidden)
            .max_filesize(Some(config.max_file_size))
            .threads(thread_count)
            .skip_stdout(true)
            .git_ignore(config.ignore_gitignore)
            .git_global(config.ignore_gitignore)
            .git_exclude(config.ignore_gitignore);

        // Add exclude patterns
        for pattern in &config.exclude_patterns {
            walker.add_ignore(&format!("!{}", pattern));
        }

        let walker = walker.build_parallel();
        
        walker.run(|| {
            let file_tx = file_tx.clone();
            let config = config.clone();
            
            Box::new(move |entry| {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    
                    if path.is_file() && Self::should_include_file(path, &config) {
                        if let Ok(metadata) = path.metadata() {
                            if metadata.len() <= config.max_file_size && Self::is_text_file(path) {
                                let file_entry = Self::create_file_entry(path.to_owned(), metadata);
                                let _ = file_tx.send(file_entry);
                            }
                        }
                    }
                }
                ignore::WalkState::Continue
            })
        });

        Ok(())
    }

    /// Check if file should be included based on configuration.
    fn should_include_file(path: &Path, config: &CrawlerConfig) -> bool {
        // Check lock files first
        if config.ignore_lock_files && Self::is_lock_file(path) {
            return false;
        }

        // Filter by file extension
        if !config.file_extensions.is_empty() {
            if config.file_extensions.iter().any(|ext| ext == "*") {
                return true;
            }

            if let Some(extension) = path.extension() {
                return Self::extension_matches(extension, &config.file_extensions);
            }
            return false;
        }

        // Use default text file extensions
        Self::has_text_extension(path)
    }

    /// Check if extension matches any of the patterns.
    fn extension_matches(ext: &OsStr, patterns: &[String]) -> bool {
        if let Some(ext_str) = ext.to_str() {
            for pattern in patterns {
                if pattern.starts_with("*.") {
                    if ext_str.eq_ignore_ascii_case(&pattern[2..]) {
                        return true;
                    }
                } else if ext_str.eq_ignore_ascii_case(pattern) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if file has a common text extension.
    fn has_text_extension(path: &Path) -> bool {
        if let Some(extension) = path.extension() {
            if let Some(ext_str) = extension.to_str() {
                return Self::is_known_text_extension(ext_str);
            }
        }

        // Check extensionless files
        if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
            Self::is_extensionless_text_file(filename)
        } else {
            false
        }
    }

    /// Check extension by length for faster matching.
    #[inline]
    fn is_known_text_extension(ext: &str) -> bool {
        match ext.len() {
            1 => matches!(ext, "c" | "h" | "r"),
            2 => matches!(ext, "rs" | "py" | "js" | "ts" | "go" | "rb" | "md"),
            3 => matches!(ext, "txt" | "cpp" | "hpp" | "php" | "css" | "xml" | "yml" | "sql" | "tex" | "asm" | "vim" | "cfg" | "ini"),
            4 => matches!(ext, "java" | "html" | "json" | "yaml" | "toml" | "bash"),
            _ => matches!(ext, "gitignore" | "dockerfile" | "makefile"),
        }
    }

    /// Fast extensionless file check.
    #[inline]
    fn is_extensionless_text_file(filename: &str) -> bool {
        matches!(filename.to_ascii_lowercase().as_str(),
            "makefile" | "dockerfile" | "readme" | "license" | "changelog" |
            "manifest" | "requirements" | "todo" | "authors" | "copying"
        )
    }

    /// Check if file is a lock file.
    fn is_lock_file(path: &Path) -> bool {
        if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
            // Check suffix patterns first
            if filename.ends_with(".lock") || filename.ends_with("-lock") {
                return true;
            }
            
            // Check known lock file names
            matches!(filename,
                "Cargo.lock" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" |
                "composer.lock" | "poetry.lock" | "Pipfile.lock" | "Gemfile.lock" |
                "mix.lock" | "go.sum" | "flake.lock" | "deno.lock"
            )
        } else {
            false
        }
    }

    /// Check if file contains text content.
    fn is_text_file(path: &Path) -> bool {
        // Trust known text extensions immediately
        if Self::has_text_extension(path) {
            return true;
        }

        // Check file content for unknown extensions
        Self::is_text_content(path)
    }

    /// Check if file content is text using memory mapping.
    fn is_text_content(path: &Path) -> bool {
        match std::fs::File::open(path) {
            Ok(file) => {
                match unsafe { memmap2::Mmap::map(&file) } {
                    Ok(mmap) => {
                        if mmap.is_empty() {
                            return true;
                        }
                        
                        // Check only first 128 bytes for speed
                        let check_size = mmap.len().min(128);
                        let bytes = &mmap[..check_size];
                        
                        // Check for binary content
                        !bytes.iter().any(|&b| b == 0 || (b < 32 && !matches!(b, 9 | 10 | 13)))
                    }
                    Err(_) => false,
                }
            }
            Err(_) => false,
        }
    }

    /// Create file entry with minimal allocations.
    fn create_file_entry(path: PathBuf, metadata: Metadata) -> FileEntry {
        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_owned())
            .unwrap_or_else(|| "unknown".to_owned());

        FileEntry {
            path,
            filename,
            size: metadata.len(),
            modified: metadata.modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            content: String::new(), // Content loaded separately
            mime_type: String::new(),
            encoding: String::new(),
            hash: String::new(),
        }
    }

    /// Load file content using memory mapping for speed.
    pub fn load_file_content(path: &Path, max_size: u64) -> Result<String> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        
        if metadata.len() > max_size {
            return Err(anyhow::anyhow!("File too large: {} bytes", metadata.len()));
        }
        
        if metadata.len() == 0 {
            return Ok(String::new());
        }
        
        let mmap = unsafe { Mmap::map(&file)? };
        match std::str::from_utf8(&mmap) {
            Ok(content) => Ok(content.to_string()),
            Err(_) => Err(anyhow::anyhow!("File is not valid UTF-8")),
        }
    }
}
