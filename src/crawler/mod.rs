use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::WalkBuilder;

use crate::types::CrawlerConfig;

pub struct FileCrawler {
    config: CrawlerConfig,
}

impl FileCrawler {
    pub fn new(config: CrawlerConfig) -> Self {
        Self { config }
    }

    pub async fn crawl_directory(&self, root_path: &Path) -> Result<Vec<PathBuf>> {
        let root_path = root_path.to_owned();
        let config = self.config.clone();

        tokio::task::spawn_blocking(move || Self::crawl(root_path, config))
            .await
            .context("Crawler task failed")?
    }

    fn crawl(root_path: PathBuf, config: CrawlerConfig) -> Result<Vec<PathBuf>> {
        let allowed_extensions: Option<HashSet<String>> = if !config.file_extensions.is_empty() {
            Some(
                config
                    .file_extensions
                    .iter()
                    .map(|ext| {
                        ext.trim_start_matches("*.")
                            .trim_start_matches('.')
                            .to_lowercase()
                    })
                    .collect(),
            )
        } else {
            None
        };

        let mut walker = WalkBuilder::new(&root_path);
        walker
            .follow_links(config.follow_symlinks)
            .hidden(!config.include_hidden)
            .max_filesize(Some(config.max_file_size))
            .skip_stdout(true)
            .git_ignore(config.ignore_gitignore)
            .same_file_system(true);

        for pattern in &config.exclude_patterns {
            walker.add_ignore(format!("!{}", pattern));
        }

        let walk_results = walker.build();
        let mut files = Vec::new();

        for entry_result in walk_results {
            if let Ok(entry) = entry_result {
                if let Some(file_path) =
                    Self::process_entry(&entry, &allowed_extensions, config.max_file_size)
                {
                    files.push(file_path);
                }
            }
        }

        Ok(files)
    }

    fn process_entry(
        entry: &ignore::DirEntry,
        allowed_extensions: &Option<HashSet<String>>,
        max_size: u64,
    ) -> Option<PathBuf> {
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => return None,
        };

        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > max_size {
            return None;
        }

        if let Some(ext_set) = allowed_extensions {
            let extension_os = match path.extension() {
                Some(ext) => ext,
                None => return None,
            };
            let extension = match extension_os.to_str() {
                Some(s) => s.to_lowercase(),
                None => return None,
            };
            if !ext_set.contains(&extension) {
                return None;
            }
        }

        Some(path.to_owned())
    }
}
