use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::mpsc;
use tui_input::Input;

use crate::config::Config;
use crate::crawler::FileCrawler;
use crate::storage::StorageManager;
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
}

pub struct Engine {
    pub should_quit: bool,
    pub app_state: AppStateData,
    pub data_changed: bool,

    pub ui_mode: UIMode,
    pub focused_window: FocusedWindow,
    pub spinner_frame: usize,

    pub search_input: Input,
    pub search_results: Vec<SearchResult>,
    pub selected_search_result: usize,
    pub search_results_scroll_offset: usize,
    pub file_preview_scroll_offset: usize,
    pub current_search_query: String,
    pub search_error: Option<String>,

    pub current_file_content: Option<String>,
    pub current_file_path: Option<PathBuf>,

    pub crawling_stats: Option<(usize, f64)>,
    pub processing_stats: Option<(usize, f64)>,
    pub timing_shown: bool,

    pub processing_service: Option<StorageManager>,

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

            search_input: Input::default(),
            search_results: Vec::new(),
            selected_search_result: 0,
            search_results_scroll_offset: 0,
            file_preview_scroll_offset: 0,
            current_search_query: String::new(),
            search_error: None,

            current_file_content: None,
            current_file_path: None,

            crawling_stats: None,
            processing_stats: None,
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
        self.current_file_content = None;
        self.current_file_path = None;
        self.ui_mode = UIMode::SearchInput;
        self.update_focused_window();
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

    pub async fn start_chunking(state_tx: mpsc::UnboundedSender<StateUpdate>, files: Vec<PathBuf>) {
        let processing_start_time = Instant::now();
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
            .join("sema");

        let mut processing_service = match StorageManager::new(&config_dir).await {
            Ok(service) => service,
            Err(_) => {
                let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
                return;
            }
        };

        match processing_service.process_and_index_files(files).await {
            Ok(chunks_count) => {
                let processing_duration = processing_start_time.elapsed().as_secs_f64();

                let _ = state_tx.send(StateUpdate::ProcessingCompleted {
                    chunks_count,
                    duration_secs: processing_duration,
                });
            }
            Err(_) => {
                let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
                processing_service.close().await;
                return;
            }
        }

        processing_service.close().await;
        let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
    }

    // Search functionality
    pub async fn execute_search(&mut self, query: &str) -> Result<()> {
        self.search_error = None;
        self.current_search_query = query.to_string();

        self.crawling_stats = None;
        self.processing_stats = None;

        // Initialize processing service if needed
        if self.processing_service.is_none() {
            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
                .join("sema");

            match StorageManager::new(&config_dir).await {
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

        // For each file, keep the first occurrence and set the total count
        let mut grouped_results: Vec<SearchResult> = file_groups
            .into_values()
            .map(|mut group| {
                // Sort by start_line to get the first occurrence in the file
                group.sort_by(|a, b| a.chunk.start_line.cmp(&b.chunk.start_line));

                let total_count = group.len();
                let mut first_result = group.into_iter().next().unwrap();
                first_result.total_matches_in_file = total_count;
                first_result
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
    pub async fn load_file_content(&self, file_path: &std::path::Path) -> Result<String> {
        // Check file size first
        match tokio::fs::metadata(file_path).await {
            Ok(metadata) => {
                const MAX_PREVIEW_SIZE: u64 = 1_048_576; // 1MB
                if metadata.len() > MAX_PREVIEW_SIZE {
                    let size_mb = metadata.len() as f64 / 1_048_576.0;
                    return Ok(format!(
                        "File size too large to display ({:.1} MB)",
                        size_mb
                    ));
                }
            }
            Err(e) => {
                return Ok(format!("Failed to read file metadata: {}", e));
            }
        }

        // Read the file content
        match tokio::fs::read_to_string(file_path).await {
            Ok(content) => Ok(content),
            Err(e) => Ok(format!("Failed to read file: {}", e)),
        }
    }

    pub async fn update_current_file_content(&mut self, file_path: &std::path::Path) {
        match self.load_file_content(file_path).await {
            Ok(content) => {
                self.current_file_content = Some(content);
                self.current_file_path = Some(file_path.to_path_buf());
            }
            Err(_) => {
                self.current_file_content = Some("Failed to load file".to_string());
                self.current_file_path = Some(file_path.to_path_buf());
            }
        }
    }

    // Lazy loading - only load visible portion for large files
    pub async fn load_file_window(
        &self,
        file_path: &std::path::Path,
        start_line: usize,
        window_size: usize,
    ) -> Result<String> {
        // Check file size first
        let metadata = tokio::fs::metadata(file_path).await?;
        const LAZY_THRESHOLD: u64 = 262_144; // 256KB - use lazy loading for files larger than this
        const MAX_PREVIEW_SIZE: u64 = 1_048_576; // 1MB

        if metadata.len() > MAX_PREVIEW_SIZE {
            let size_mb = metadata.len() as f64 / 1_048_576.0;
            return Ok(format!(
                "File size too large to display ({:.1} MB)",
                size_mb
            ));
        }

        // For smaller files, use regular loading
        if metadata.len() <= LAZY_THRESHOLD {
            return self.load_file_content(file_path).await;
        }

        // For larger files, read only visible window
        use std::io::{BufRead, BufReader};
        use tokio::fs::File;

        let file = File::open(file_path).await?;
        let mut reader = BufReader::new(file.into_std().await);
        let mut lines = Vec::new();
        let mut current_line = 0;

        // Skip to start line
        let mut line_buf = String::new();
        while current_line < start_line {
            line_buf.clear();
            if reader.read_line(&mut line_buf)? == 0 {
                break; // EOF
            }
            current_line += 1;
        }

        // Read window
        for _ in 0..window_size {
            line_buf.clear();
            if reader.read_line(&mut line_buf)? == 0 {
                break; // EOF
            }
            lines.push(line_buf.trim_end_matches('\n').to_string());
        }

        Ok(lines.join("\n"))
    }

    pub fn calculate_search_result_line_offset(&self, search_result: &SearchResult) -> usize {
        // Position at the start of the chunk
        // chunk.start_line is 1-based, scroll offset is 0-based
        search_result.chunk.start_line.saturating_sub(1)
    }
}
