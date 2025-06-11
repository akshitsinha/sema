use std::collections::HashSet;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use num_cpus;
use tokio::sync::mpsc as async_mpsc;

use crate::types::{CrawlerConfig, FileEntry};

/// File crawler for discovering and processing files in directories.
pub struct FileCrawler {
    config: CrawlerConfig,
    extension_set: HashSet<String>,
    lock_file_patterns: HashSet<&'static str>,
    extensionless_files: HashSet<&'static str>,
}

impl FileCrawler {
    /// Creates a new file crawler.
    pub fn new(config: CrawlerConfig) -> Self {
        let extension_set = Self::build_extension_set(&config);
        let lock_file_patterns = Self::build_lock_patterns();
        let extensionless_files = Self::build_extensionless_patterns();

        Self {
            config,
            extension_set,
            lock_file_patterns,
            extensionless_files,
        }
    }

    /// Builds extension set for file filtering.
    fn build_extension_set(config: &CrawlerConfig) -> HashSet<String> {
        let mut set = HashSet::with_capacity(64);

        if config.file_extensions.is_empty() {
            // Default text extensions
            for ext in [
                // Programming languages
                "rs",
                "py",
                "js",
                "ts",
                "jsx",
                "tsx",
                "go",
                "rb",
                "php",
                "java",
                "kt",
                "scala",
                "c",
                "cpp",
                "cc",
                "cxx",
                "h",
                "hpp",
                "hxx",
                "cs",
                "vb",
                "fs",
                "ml",
                "hs",
                "elm",
                "clj",
                "cljs",
                "ex",
                "exs",
                "erl",
                "hrl",
                "dart",
                "swift",
                "r",
                "julia",
                "jl",
                "lua",
                "pl",
                "pm",
                "tcl",
                "vhdl",
                "v",
                "sv",
                "verilog",
                "asm",
                "s",
                // Web technologies
                "html",
                "htm",
                "css",
                "scss",
                "sass",
                "less",
                "vue",
                "svelte",
                "astro",
                // Data formats
                "json",
                "yaml",
                "yml",
                "toml",
                "xml",
                "csv",
                "tsv",
                "ini",
                "cfg",
                "conf",
                "properties",
                "env",
                "dotenv",
                // Documentation
                "md",
                "markdown",
                "rst",
                "txt",
                "rtf",
                "tex",
                "latex",
                "adoc",
                "asciidoc",
                // Configuration and scripts
                "sh",
                "bash",
                "zsh",
                "fish",
                "ps1",
                "bat",
                "cmd",
                "dockerfile",
                "makefile",
                "cmake",
                "gradle",
                "sbt",
                "build",
                "ninja",
                // Misc text
                "log",
                "sql",
                "graphql",
                "gql",
                "proto",
                "thrift",
                "avro",
                "vim",
                "emacs",
            ] {
                set.insert(ext.to_string());
            }
        } else {
            for ext in &config.file_extensions {
                let clean_ext = if ext.starts_with("*.") {
                    &ext[2..]
                } else if ext.starts_with('.') {
                    &ext[1..]
                } else {
                    ext
                };
                set.insert(clean_ext.to_lowercase());
            }
        }

        set
    }

    /// Builds lock file patterns.
    fn build_lock_patterns() -> HashSet<&'static str> {
        [
            // Rust
            "Cargo.lock",
            // Node.js
            "package-lock.json",
            "yarn.lock",
            "pnpm-lock.yaml",
            "npm-shrinkwrap.json",
            // PHP
            "composer.lock",
            // Python
            "poetry.lock",
            "Pipfile.lock",
            "pdm.lock",
            // Ruby
            "Gemfile.lock",
            // Elixir
            "mix.lock",
            // Go
            "go.sum",
            // Nix
            "flake.lock",
            // Deno
            "deno.lock",
            // Swift
            "Package.resolved",
            // Other
            "pubspec.lock",
            "packages.lock.json",
        ]
        .into_iter()
        .collect()
    }

    /// Builds extensionless file patterns.
    fn build_extensionless_patterns() -> HashSet<&'static str> {
        [
            // Documentation
            "readme",
            "license",
            "changelog",
            "authors",
            "copying",
            "install",
            "news",
            "history",
            "contributors",
            "acknowledgments",
            "notice",
            "thanks",
            // Configuration
            "makefile",
            "dockerfile",
            "rakefile",
            "gemfile",
            "procfile",
            "requirements",
            "manifest",
            "justfile",
            "vagrantfile",
            "berksfile",
            "guardfile",
            // Version control
            "gitignore",
            "gitattributes",
            "gitmodules",
            "gitkeep",
            // CI/CD
            "jenkinsfile",
            "buildkite",
            "circleci",
            "travis",
            "appveyor",
            // Other
            "todo",
            "fixme",
            "notes",
            "codeowners",
            "editorconfig",
        ]
        .into_iter()
        .collect()
    }

    /// Crawl directory for files.
    pub async fn crawl_directory(
        &self,
        root_path: &Path,
        file_tx: async_mpsc::UnboundedSender<FileEntry>,
    ) -> Result<()> {
        let root_path = root_path.to_owned();
        let extension_set = self.extension_set.clone();
        let lock_file_patterns = self.lock_file_patterns.clone();
        let extensionless_files = self.extensionless_files.clone();
        let config = Arc::new(self.config.clone());

        tokio::task::spawn_blocking(move || {
            Self::crawl(
                root_path,
                config,
                file_tx,
                extension_set,
                lock_file_patterns,
                extensionless_files,
            )
        })
        .await
        .context("Crawler task failed")?
    }

    /// Parallel crawling implementation.
    fn crawl(
        root_path: PathBuf,
        config: Arc<CrawlerConfig>,
        file_tx: async_mpsc::UnboundedSender<FileEntry>,
        extension_set: HashSet<String>,
        lock_file_patterns: HashSet<&'static str>,
        extensionless_files: HashSet<&'static str>,
    ) -> Result<()> {
        // Thread count for IO parallelism
        let cpu_count = num_cpus::get();
        let thread_count = (cpu_count * 6).max(24).min(128);

        let mut walker = WalkBuilder::new(&root_path);

        // Configure walker
        walker
            .follow_links(config.follow_symlinks)
            .hidden(!config.include_hidden)
            .max_filesize(Some(config.max_file_size))
            .threads(thread_count)
            .skip_stdout(true)
            .git_ignore(config.ignore_gitignore)
            .git_global(config.ignore_gitignore)
            .git_exclude(config.ignore_gitignore)
            .same_file_system(true);

        // Add ignore patterns
        for pattern in &config.exclude_patterns {
            walker.add_ignore(&format!("!{}", pattern));
        }

        let walker = walker.build_parallel();

        // Batching system
        let (batch_tx, batch_rx) = std::sync::mpsc::channel::<Vec<FileEntry>>();
        let batch_size = 1024;

        // Sender thread
        let file_tx_clone = file_tx.clone();
        std::thread::spawn(move || {
            while let Ok(batch) = batch_rx.recv() {
                for entry in batch {
                    if file_tx_clone.send(entry).is_err() {
                        return; // Channel closed
                    }
                }
            }
        });

        // File processing
        walker.run(|| {
            let batch_tx = batch_tx.clone();
            let config = config.clone();
            let extension_set = extension_set.clone();
            let lock_file_patterns = lock_file_patterns.clone();
            let extensionless_files = extensionless_files.clone();
            let mut batch = Vec::with_capacity(batch_size);

            Box::new(move |entry| {
                if let Ok(entry) = entry {
                    let path = entry.path();

                    // Check metadata
                    if let Ok(metadata) = entry.metadata() {
                        if metadata.is_file()
                            && metadata.len() <= config.max_file_size
                            && metadata.len() > 0
                            && Self::should_include_file(
                                path,
                                &config,
                                &extension_set,
                                &lock_file_patterns,
                                &extensionless_files,
                            )
                        {
                            let file_entry = Self::create_file_entry(path.to_owned(), metadata);
                            batch.push(file_entry);

                            // Send batch when full
                            if batch.len() >= batch_size {
                                let _ = batch_tx.send(std::mem::take(&mut batch));
                                batch = Vec::with_capacity(batch_size);
                            }
                        }
                    }
                }
                ignore::WalkState::Continue
            })
        });

        Ok(())
    }

    /// File filtering logic.
    #[inline]
    fn should_include_file(
        path: &Path,
        config: &CrawlerConfig,
        extension_set: &HashSet<String>,
        lock_file_patterns: &HashSet<&'static str>,
        extensionless_files: &HashSet<&'static str>,
    ) -> bool {
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };

        // Lock file check
        if config.ignore_lock_files
            && (filename.ends_with(".lock")
                || filename.ends_with("-lock")
                || lock_file_patterns.contains(filename))
        {
            return false;
        }

        // Check for wildcard acceptance
        if config.file_extensions.iter().any(|ext| ext == "*") {
            return true;
        }

        // Extension check
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            return extension_set.contains(&ext.to_lowercase());
        }

        // Extensionless file check
        extensionless_files.contains(&filename.to_lowercase().as_str())
    }

    /// Create file entry from path and metadata.
    #[inline]
    fn create_file_entry(path: PathBuf, metadata: Metadata) -> FileEntry {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_owned())
            .unwrap_or_else(|| "unknown".to_owned());

        FileEntry {
            path,
            filename,
            size: metadata.len(),
            modified: metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            content: String::new(),
            mime_type: String::new(),
            encoding: String::new(),
            hash: String::new(),
        }
    }

    /// Load file content with size limits.
    pub fn load_file_content(path: &Path, max_size: u64) -> Result<String> {
        let metadata = std::fs::metadata(path)?;

        if metadata.len() > max_size {
            return Err(anyhow::anyhow!("File too large: {} bytes", metadata.len()));
        }

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

                if content.len() > max_size as usize {
                    return Err(anyhow::anyhow!("File content exceeds size limit"));
                }
            }
            Ok(content)
        } else {
            // For small files
            std::fs::read_to_string(path).context("Failed to read file")
        }
    }
}
