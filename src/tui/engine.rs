use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::crawler::FileCrawler;
use crate::storage::Processing;
use crate::types::{AppState as AppStateEnum, CrawlerConfig, FocusedWindow, SearchResult, UIMode};

const SEARCH_RESULTS_LIMIT: usize = 50;

#[derive(Clone)]
pub struct AppStateData {
    pub crawled_files: Vec<PathBuf>,
    pub state: AppStateEnum,
}

#[derive(Debug)]
pub enum StateUpdate {
    FileFound(PathBuf),
    StateChanged(AppStateEnum),
    AllFilesCollected(Vec<PathBuf>),
    CrawlingCompleted {
        files_count: usize,
        duration_secs: f64,
    },
    ProcessingCompleted {
        chunks_count: usize,
        duration_secs: f64,
    },
    EmbeddingStarted,
    EmbeddingCompleted {
        chunks_count: usize,
        duration_secs: f64,
    },
}

pub struct Engine {
    // Core state
    pub should_quit: bool,
    pub app_state: AppStateData,
    pub data_changed: bool,

    // UI state
    pub ui_mode: UIMode,
    pub focused_window: FocusedWindow,
    pub spinner_frame: usize,

    // Search state
    pub search_input: String,
    pub search_results: Vec<SearchResult>,
    pub selected_search_result: usize,
    pub search_results_scroll_offset: usize,
    pub file_preview_scroll_offset: usize,
    pub current_search_query: String,
    pub search_error: Option<String>,

    // File preview state
    pub cached_file_content: Option<String>,
    pub cached_file_path: Option<PathBuf>,

    // Stats
    pub crawling_stats: Option<(usize, f64)>,
    pub processing_stats: Option<(usize, f64)>,
    pub embedding_stats: Option<(usize, f64)>,
    pub timing_shown: bool,

    // Services
    pub processing_service: Option<Processing>,

    // Crawler management
    pub crawler_config: CrawlerConfig,
    pub root_path: PathBuf,
}

impl Engine {
    pub fn new(directory: PathBuf, config: Config) -> Self {
        let crawler_config = CrawlerConfig::from(&config.general);

        Self {
            should_quit: false,
            app_state: AppStateData {
                crawled_files: Vec::new(),
                state: AppStateEnum::Crawling,
            },
            data_changed: false,

            ui_mode: UIMode::SearchInput,
            focused_window: FocusedWindow::SearchInput,
            spinner_frame: 0,

            search_input: String::new(),
            search_results: Vec::new(),
            selected_search_result: 0,
            search_results_scroll_offset: 0,
            file_preview_scroll_offset: 0,
            current_search_query: String::new(),
            search_error: None,

            cached_file_content: None,
            cached_file_path: None,

            crawling_stats: None,
            processing_stats: None,
            embedding_stats: None,
            timing_shown: false,
            processing_service: None,

            crawler_config,
            root_path: directory,
        }
    }

    pub fn update_focused_window(&mut self) {
        self.focused_window = match self.ui_mode {
            UIMode::SearchInput => FocusedWindow::SearchInput,
            UIMode::SearchResults => FocusedWindow::SearchResults,
            UIMode::FilePreview => FocusedWindow::FilePreview,
        };
    }

    pub fn clear_search(&mut self) {
        self.search_results.clear();
        self.selected_search_result = 0;
        self.search_results_scroll_offset = 0;
        self.current_search_query.clear();
        self.search_error = None;
        self.ui_mode = UIMode::SearchInput;
        self.update_focused_window();
    }

    pub fn clear_file_cache(&mut self) {
        self.cached_file_content = None;
        self.cached_file_path = None;
    }

    // Crawler management
    pub async fn start_crawler(
        &self,
        state_tx: mpsc::UnboundedSender<StateUpdate>,
    ) -> Result<tokio::task::JoinHandle<Result<()>>> {
        let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Crawling));

        let (file_tx, mut file_rx) = mpsc::unbounded_channel();
        let crawler = FileCrawler::new(self.crawler_config.clone());
        let root_path = self.root_path.clone();
        let state_tx_clone = state_tx.clone();

        let crawler_handle = tokio::spawn(async move {
            let result = crawler.crawl_directory(&root_path, file_tx).await;
            if result.is_ok() {
                let _ = state_tx_clone.send(StateUpdate::StateChanged(AppStateEnum::Chunking));
            }
            result
        });

        let state_tx_files = state_tx.clone();
        let crawling_start_time = Instant::now();
        tokio::spawn(async move {
            let mut collected_files = Vec::new();
            while let Some(file_entry) = file_rx.recv().await {
                let _ = state_tx_files.send(StateUpdate::FileFound(file_entry.clone()));
                collected_files.push(file_entry);
            }
            if !collected_files.is_empty() {
                let crawling_duration = crawling_start_time.elapsed().as_secs_f64();
                let files_count = collected_files.len();

                let _ = state_tx_files.send(StateUpdate::CrawlingCompleted {
                    files_count,
                    duration_secs: crawling_duration,
                });

                let _ = state_tx_files.send(StateUpdate::AllFilesCollected(collected_files));
            }
        });

        Ok(crawler_handle)
    }

    pub async fn start_chunking(
        state_tx: mpsc::UnboundedSender<StateUpdate>,
        files: Vec<PathBuf>,
        _max_file_size: u64,
    ) {
        let processing_start_time = Instant::now();
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
            .join("sema");

        let mut processing_service = match Processing::new(&config_dir).await {
            Ok(service) => service,
            Err(_e) => {
                let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
                return;
            }
        };

        let should_start_embeddings = match processing_service.process_files(files).await {
            Ok(chunks_count) => {
                let processing_duration = processing_start_time.elapsed().as_secs_f64();

                let _ = state_tx.send(StateUpdate::ProcessingCompleted {
                    chunks_count,
                    duration_secs: processing_duration,
                });
                true
            }
            Err(_e) => {
                let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
                false
            }
        };

        processing_service.close().await;

        // Start embedding generation after processing service is closed
        if should_start_embeddings {
            Self::start_embedding_generation(state_tx.clone(), config_dir.clone()).await;
        }
    }

    pub async fn start_embedding_generation(
        state_tx: mpsc::UnboundedSender<StateUpdate>,
        _config_dir: PathBuf,
    ) {
        // First, set state to downloading model
        let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::DownloadingModel));

        // TODO: Get total number of chunks from LanceDB instead
        let _total_chunks = 0; // Placeholder until we implement chunk counting from LanceDB

        // TODO: Enable when semantic search is fixed
        // let mut semantic_index = match SemanticSearch::new(&config_dir, total_chunks) {
        //     Ok(index) => index,
        //     Err(_) => {
        //         let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
        //         return;
        //     }
        // };

        // After model download, switch to generating embeddings
        let _ = state_tx.send(StateUpdate::StateChanged(
            AppStateEnum::GeneratingEmbeddings,
        ));
        let _ = state_tx.send(StateUpdate::EmbeddingStarted);

        // let embedding_start_time = Instant::now();

        // Process all chunks and store embeddings in usearch index
        // TODO: Fix semantic search processing
        // if let Err(_) = semantic_index.process_all_chunks(chunks).await {
        //     let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
        //     return;
        // }

        // Save the index to disk
        // if semantic_index.save().is_err() {
        //     let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
        //     return;
        // }

        // let embedding_duration = embedding_start_time.elapsed().as_secs_f64();
        // let _ = state_tx.send(StateUpdate::EmbeddingCompleted {
        //     chunks_count: total_chunks,
        //     duration_secs: embedding_duration,
        // });

        let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
    }

    // Search functionality
    pub async fn execute_search(&mut self, query: &str) -> Result<()> {
        self.search_error = None;
        self.current_search_query = query.to_string();

        // Clear timing stats once a search is performed
        self.crawling_stats = None;
        self.processing_stats = None;
        self.embedding_stats = None;

        // Initialize processing service if needed
        if self.processing_service.is_none() {
            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
                .join("sema");

            match Processing::new(&config_dir).await {
                Ok(service) => {
                    self.processing_service = Some(service);
                }
                Err(_) => {
                    self.search_error = Some("Failed to initialize search service".to_string());
                    return Ok(());
                }
            }
        }

        // Execute search using the processing service
        if let Some(ref mut service) = self.processing_service {
            // Initialize semantic search if not already done
            if !service.has_semantic_search() {
                let _ = service.init_semantic_search(); // Try to initialize, ignore errors
            }

            match service.search(query, SEARCH_RESULTS_LIMIT).await {
                Ok(results) => {
                    // Convert to SearchResult format
                    let search_results: Vec<SearchResult> = results
                        .into_iter()
                        .map(|(chunk, score)| SearchResult {
                            chunk: chunk.clone(),
                            score,
                            snippet: String::new(), // Not used in TUI
                            highlighted_content: String::new(), // Not used in TUI
                            total_matches_in_file: 1, // Will be updated in grouping
                        })
                        .collect();

                    // Group results by file, keeping only the first chunk per file
                    self.search_results = Self::group_results_by_file(search_results);
                    self.selected_search_result = 0;
                    self.search_results_scroll_offset = 0;

                    // Auto-switch to SearchResults mode if we have results and are in SearchInput mode
                    if !self.search_results.is_empty()
                        && matches!(self.ui_mode, UIMode::SearchInput)
                    {
                        self.ui_mode = UIMode::SearchResults;
                        self.update_focused_window();
                    }
                }
                Err(e) => {
                    self.search_error = Some(format!("Search failed: {}", e));
                }
            }
        }

        Ok(())
    }

    fn group_results_by_file(results: Vec<SearchResult>) -> Vec<SearchResult> {
        let mut file_groups: HashMap<PathBuf, Vec<SearchResult>> = HashMap::new();

        // Group all results by file
        for result in results {
            let file_path = result.chunk.file_path.clone();
            file_groups.entry(file_path).or_default().push(result);
        }

        // For each file, keep the best result and set the total count
        let mut grouped_results: Vec<SearchResult> = file_groups
            .into_values()
            .map(|mut group| {
                // Sort by score and take the best one
                group.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                let total_count = group.len();
                let mut best_result = group.into_iter().next().unwrap();
                best_result.total_matches_in_file = total_count;
                best_result
            })
            .collect();

        // Sort final results by score
        grouped_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        grouped_results
    }

    // File content management
    pub async fn load_file_content(&mut self, file_path: &std::path::Path) -> Result<&str> {
        // Check if we already have this file cached
        if let Some(ref cached_path) = self.cached_file_path {
            if cached_path == file_path && self.cached_file_content.is_some() {
                return Ok(self.cached_file_content.as_ref().unwrap());
            }
        }

        // Read the file content
        match tokio::fs::read_to_string(file_path).await {
            Ok(content) => {
                self.cached_file_content = Some(content);
                self.cached_file_path = Some(file_path.to_path_buf());
                Ok(self.cached_file_content.as_ref().unwrap())
            }
            Err(e) => Err(anyhow::anyhow!("Failed to read file: {}", e)),
        }
    }

    pub fn calculate_search_result_line_offset(&self, search_result: &SearchResult) -> usize {
        // Position at the start of the chunk
        // chunk.start_line is 1-based, scroll offset is 0-based
        search_result.chunk.start_line.saturating_sub(1)
    }

    // TODO: Reimplement to count chunks from LanceDB
    // fn count_chunks_in_db() -> Result<usize> {
    //     Ok(0)
    // }
}
