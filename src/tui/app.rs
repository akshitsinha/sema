use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::crawler::FileCrawler;
use crate::search::SearchResult;
use crate::storage::service::ProcessingService;
use crate::types::{ChunkConfig, CrawlerConfig, FocusedWindow, UIMode};

use super::components::{
    ColorManager, FileListRenderer, FilePreviewRenderer, SearchInputRenderer, SearchResultsRenderer,
};
use super::handlers::{KeyboardHandler, SearchKeyboardResult};
use super::utils::SpinnerUtils;

#[derive(Clone)]
pub struct AppStateData {
    pub crawled_files: Vec<PathBuf>,
    pub state: crate::types::AppState,
}

#[derive(Debug)]
pub enum StateUpdate {
    FileFound(PathBuf),
    StateChanged(crate::types::AppState),
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

pub struct App {
    should_quit: bool,
    app_state: AppStateData,
    state_rx: mpsc::UnboundedReceiver<StateUpdate>,
    state_tx: mpsc::UnboundedSender<StateUpdate>,
    crawler_handle: Option<tokio::task::JoinHandle<Result<()>>>,
    crawler_config: CrawlerConfig,
    root_path: PathBuf,
    file_list_scroll_offset: usize,
    selected_file_index: usize,
    spinner_frame: usize,
    search_input: String,
    search_mode: bool,
    color_manager: ColorManager,
    data_changed: bool,
    crawling_start_time: Option<Instant>,
    processing_start_time: Option<Instant>,
    crawling_stats: Option<(usize, f64)>, // files_count, duration_secs
    processing_stats: Option<(usize, f64)>, // chunks_count, duration_secs
    // Search functionality
    ui_mode: UIMode,
    search_results: Vec<SearchResult>,
    selected_search_result: usize,
    search_results_scroll_offset: usize,
    file_preview_scroll_offset: usize,
    processing_service: Option<ProcessingService>,
    focused_window: FocusedWindow,
    current_search_query: String, // Store the current search query for highlighting
    search_error: Option<String>, // Store search error messages
    file_preview_renderer: FilePreviewRenderer, // Instance for caching
}

impl App {
    pub fn new_with_directory(directory: PathBuf, config: Config) -> Result<Self> {
        let crawler_config = CrawlerConfig::from(&config.general);
        let (state_tx, state_rx) = mpsc::unbounded_channel();

        Ok(Self {
            should_quit: false,
            app_state: AppStateData {
                crawled_files: Vec::new(),
                state: crate::types::AppState::Crawling,
            },
            state_rx,
            state_tx,
            crawler_handle: None,
            crawler_config,
            root_path: directory,
            file_list_scroll_offset: 0,
            selected_file_index: 0,
            spinner_frame: 0,
            search_input: String::new(),
            search_mode: true,
            color_manager: ColorManager::new(),
            data_changed: false,
            crawling_start_time: None,
            processing_start_time: None,
            crawling_stats: None,
            processing_stats: None,
            ui_mode: UIMode::SearchInput,
            search_results: Vec::new(),
            selected_search_result: 0,
            search_results_scroll_offset: 0,
            file_preview_scroll_offset: 0,
            processing_service: None,
            focused_window: FocusedWindow::SearchInput,
            current_search_query: String::new(),
            search_error: None,
            file_preview_renderer: FilePreviewRenderer::new(),
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        self.start_crawler().await?;

        let result = self.run_app(&mut terminal).await;

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    async fn start_crawler(&mut self) -> Result<()> {
        let _ = self
            .state_tx
            .send(StateUpdate::StateChanged(crate::types::AppState::Crawling));

        let (file_tx, mut file_rx) = mpsc::unbounded_channel();
        let crawler = FileCrawler::new(self.crawler_config.clone());
        let root_path = self.root_path.clone();
        let state_tx = self.state_tx.clone();

        let crawler_handle = tokio::spawn(async move {
            let result = crawler.crawl_directory(&root_path, file_tx).await;
            if result.is_ok() {
                let _ = state_tx.send(StateUpdate::StateChanged(crate::types::AppState::Chunking));
            }
            result
        });

        self.crawler_handle = Some(crawler_handle);

        let state_tx = self.state_tx.clone();
        let crawling_start_time = Instant::now();
        tokio::spawn(async move {
            let mut collected_files = Vec::new();
            while let Some(file_entry) = file_rx.recv().await {
                let _ = state_tx.send(StateUpdate::FileFound(file_entry.clone()));
                collected_files.push(file_entry);
            }
            if !collected_files.is_empty() {
                let crawling_duration = crawling_start_time.elapsed().as_secs_f64();
                let files_count = collected_files.len();

                // Send crawling completed message
                let _ = state_tx.send(StateUpdate::CrawlingCompleted {
                    files_count,
                    duration_secs: crawling_duration,
                });

                let _ = state_tx.send(StateUpdate::AllFilesCollected(collected_files));
            }
        });

        Ok(())
    }

    async fn run_app<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        let mut last_tick = Instant::now();
        let mut needs_redraw = true;

        loop {
            while let Ok(update) = self.state_rx.try_recv() {
                match update {
                    StateUpdate::FileFound(file) => {
                        self.app_state.crawled_files.push(file);
                        self.data_changed = true;
                    }
                    StateUpdate::StateChanged(new_state) => {
                        match new_state {
                            crate::types::AppState::Crawling => {
                                self.crawling_start_time = Some(Instant::now());
                            }
                            crate::types::AppState::Chunking => {
                                self.processing_start_time = Some(Instant::now());
                            }
                            _ => {}
                        }
                        self.app_state.state = new_state;
                        self.data_changed = true;
                    }
                    StateUpdate::AllFilesCollected(files) => {
                        let state_tx = self.state_tx.clone();
                        let max_file_size = self.crawler_config.max_file_size;
                        tokio::spawn(async move {
                            Self::start_chunking(state_tx, files, max_file_size).await;
                        });
                    }
                    StateUpdate::CrawlingCompleted {
                        files_count,
                        duration_secs,
                    } => {
                        self.crawling_stats = Some((files_count, duration_secs));
                        self.data_changed = true;
                    }
                    StateUpdate::ProcessingCompleted {
                        chunks_count,
                        duration_secs,
                    } => {
                        self.processing_stats = Some((chunks_count, duration_secs));
                        self.data_changed = true;

                        // Transition to search mode - processing service will be initialized when needed
                        self.app_state.state = crate::types::AppState::Ready;
                        self.ui_mode = UIMode::SearchInput;
                        self.update_focused_window();
                    }
                }
            }

            if self.data_changed {
                needs_redraw = true;
                self.data_changed = false;
            }

            if needs_redraw {
                terminal.draw(|f| self.ui(f))?;
                needs_redraw = false;
            }

            let current_state = self.app_state.state.clone();

            if crossterm::event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Press {
                            needs_redraw = true;

                            // Clear search error on any key press
                            if self.search_error.is_some() {
                                self.search_error = None;
                            }

                            match current_state {
                                crate::types::AppState::Ready => {
                                    // Get current search result if available
                                    let current_search_result = if !self.search_results.is_empty()
                                        && self.selected_search_result < self.search_results.len()
                                    {
                                        Some(&self.search_results[self.selected_search_result])
                                    } else {
                                        None
                                    };

                                    // Use new search interface keyboard handling
                                    let keyboard_result =
                                        KeyboardHandler::handle_search_interface_key(
                                            &key,
                                            &mut self.search_input,
                                            &mut self.ui_mode,
                                            &mut self.selected_search_result,
                                            &mut self.search_results_scroll_offset,
                                            &mut self.file_preview_scroll_offset,
                                            self.search_results.len(),
                                            &mut self.should_quit,
                                            current_search_result,
                                        )
                                        .await;

                                    // Handle search execution
                                    match keyboard_result {
                                        SearchKeyboardResult::ExecuteSearch(query) => {
                                            // Execute search immediately (for Enter key)
                                            self.execute_search(&query).await;
                                        }
                                        SearchKeyboardResult::NoAction => {
                                            // No action needed
                                        }
                                    }
                                    // Update focused window based on UI mode changes
                                    self.update_focused_window();
                                }
                                _ => {
                                    // Legacy handling for non-ready states
                                    if self.search_mode {
                                        KeyboardHandler::handle_search_mode_key(
                                            &key,
                                            &mut self.search_input,
                                            &mut self.search_mode,
                                            &mut self.selected_file_index,
                                            &mut self.file_list_scroll_offset,
                                            &mut self.should_quit,
                                        )
                                        .await;
                                    } else {
                                        let get_total_count = || self.app_state.crawled_files.len();

                                        KeyboardHandler::handle_normal_mode_key(
                                            &key,
                                            &mut self.should_quit,
                                            &mut self.search_mode,
                                            &mut self.selected_file_index,
                                            &mut self.file_list_scroll_offset,
                                            get_total_count,
                                            &current_state,
                                        )
                                        .await?;
                                    }
                                }
                            }
                        }
                    }
                    Event::Mouse(mouse) => {
                        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                            needs_redraw = true;
                            // Always bring search input into focus when clicking anywhere
                            self.focused_window = FocusedWindow::SearchInput;
                            self.search_mode = true;
                            // If we're in Ready state with search results, switch to SearchInput mode
                            if matches!(current_state, crate::types::AppState::Ready)
                                && !self.search_results.is_empty()
                            {
                                self.ui_mode = UIMode::SearchInput;
                            }
                        }
                    }
                    _ => {}
                }
            }

            if matches!(
                current_state,
                crate::types::AppState::Crawling | crate::types::AppState::Chunking
            ) && last_tick.elapsed() >= Duration::from_millis(100)
            {
                self.spinner_frame = (self.spinner_frame + 1) % 8;
                needs_redraw = true;
                last_tick = Instant::now();
            }

            if self.should_quit {
                break;
            }
        }

        if let Some(handle) = self.crawler_handle.take() {
            handle.abort();
        }

        Ok(())
    }

    async fn start_chunking(
        state_tx: mpsc::UnboundedSender<StateUpdate>,
        files: Vec<PathBuf>,
        max_file_size: u64,
    ) {
        let processing_start_time = Instant::now();
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
            .join("sema");

        let processing_service =
            match ProcessingService::new(&config_dir, ChunkConfig::default()).await {
                Ok(service) => service,
                Err(_e) => {
                    // Failed to initialize processing service - just transition to Ready state
                    let _ = state_tx.send(StateUpdate::StateChanged(crate::types::AppState::Ready));
                    return;
                }
            };

        match processing_service.process_files(files, max_file_size).await {
            Ok(chunks_count) => {
                let processing_duration = processing_start_time.elapsed().as_secs_f64();

                // Send processing completed message
                let _ = state_tx.send(StateUpdate::ProcessingCompleted {
                    chunks_count,
                    duration_secs: processing_duration,
                });

                let _ = state_tx.send(StateUpdate::StateChanged(crate::types::AppState::Ready));
            }
            Err(_e) => {
                // Failed to process files - just transition to Ready state
                let _ = state_tx.send(StateUpdate::StateChanged(crate::types::AppState::Ready));
            }
        }

        processing_service.close().await;
    }

    fn ui(&mut self, f: &mut Frame) {
        let state = self.app_state.state.clone();

        let area = f.area();
        let background = Block::default().style(Style::default().bg(Color::Reset));
        f.render_widget(background, area);

        match state {
            crate::types::AppState::Crawling
            | crate::types::AppState::Chunking
            | crate::types::AppState::Ready => {
                // Clone files to avoid borrow checker issues
                let files = self.app_state.crawled_files.clone();
                self.render_ready(f, area, &files);
            }
            crate::types::AppState::Searching => self.render_search_interface(f, area),
        }
    }

    fn render_ready(&mut self, f: &mut Frame, area: Rect, files: &[PathBuf]) {
        let state = self.app_state.state.clone();

        // If we have search results and we're in Ready state, use the search interface rendering
        if !self.search_results.is_empty() && matches!(state, crate::types::AppState::Ready) {
            self.render_search_interface(f, area);
        } else {
            // Always use vertical layout with search input at bottom for file browsing
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(3)])
                .split(area);

            // Show file list in main area
            let file_refs: Vec<&PathBuf> = files.iter().collect();
            FileListRenderer::render(
                f,
                chunks[0],
                &file_refs,
                &self.root_path,
                self.selected_file_index,
                self.file_list_scroll_offset,
                &self.color_manager,
                matches!(self.focused_window, FocusedWindow::FileList),
            );

            // Always show search input at the bottom
            let spinner_char = SpinnerUtils::get_spinner_char(self.spinner_frame);
            SearchInputRenderer::render(
                f,
                chunks[1],
                &self.search_input,
                self.search_mode,
                files.len(),
                &state,
                spinner_char,
                &self.crawling_stats,
                &self.processing_stats,
                matches!(self.focused_window, FocusedWindow::SearchInput),
                &self.search_error,
                None, // No search results count in file browsing mode
            );
        }
    }

    fn render_search_interface(&mut self, f: &mut Frame, area: Rect) {
        match self.ui_mode {
            UIMode::SearchInput => {
                self.render_search_input_only(f, area);
            }
            UIMode::SearchResults => {
                self.render_search_results_split(f, area);
            }
            UIMode::FilePreview => {
                self.render_file_preview_split(f, area);
            }
        }
    }

    fn render_search_input_only(&self, f: &mut Frame, area: Rect) {
        // Full screen search input
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);

        // Show welcome message
        let welcome_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Blue))
            .title(" Ready to Search ")
            .title_style(
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Reset));

        let welcome_text = if self.search_input.is_empty() {
            "Type your search query and press Enter to search through indexed files."
        } else {
            "Press Enter to execute search, or continue typing to refine your query."
        };

        let welcome_para = Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                welcome_text,
                Style::default().fg(Color::DarkGray),
            )]),
        ])
        .alignment(Alignment::Center)
        .block(welcome_block);

        f.render_widget(welcome_para, chunks[0]);

        // Search input at the bottom
        SearchInputRenderer::render(
            f,
            chunks[1],
            &self.search_input,
            true, // Always active in search mode
            0,    // No file count needed
            &crate::types::AppState::Ready,
            "", // No spinner
            &None,
            &None,
            matches!(self.focused_window, FocusedWindow::SearchInput),
            &self.search_error,
            Some(self.search_results.len()), // Show search results count
        );
    }

    fn render_search_results_split(&mut self, f: &mut Frame, area: Rect) {
        // Vertical layout with search input at bottom
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);

        // Horizontal split for search results and preview
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(main_chunks[0]);

        // Left side: Search results list
        SearchResultsRenderer::render(
            f,
            chunks[0],
            &self.search_results,
            self.selected_search_result,
            self.search_results_scroll_offset,
            &self.root_path,
            matches!(self.focused_window, FocusedWindow::SearchResults),
        );

        // Right side: File preview (chunk only)
        if let Some(selected_result) = self.search_results.get(self.selected_search_result) {
            self.file_preview_renderer.render(
                f,
                chunks[1],
                selected_result,
                self.file_preview_scroll_offset,
                matches!(self.focused_window, FocusedWindow::FilePreview),
            );
        }

        // Search input at the bottom
        SearchInputRenderer::render(
            f,
            main_chunks[1],
            &self.search_input,
            true, // Always active in search mode
            0,    // No file count needed
            &crate::types::AppState::Ready,
            "", // No spinner
            &None,
            &None,
            matches!(self.focused_window, FocusedWindow::SearchInput),
            &self.search_error,
            Some(self.search_results.len()), // Show search results count
        );
    }

    fn render_file_preview_split(&mut self, f: &mut Frame, area: Rect) {
        // Vertical layout with search input at bottom
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);

        // Horizontal split for search results and preview
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(main_chunks[0]);

        // Left side: Search results list
        SearchResultsRenderer::render(
            f,
            chunks[0],
            &self.search_results,
            self.selected_search_result,
            self.search_results_scroll_offset,
            &self.root_path,
            matches!(self.focused_window, FocusedWindow::SearchResults),
        );

        // Right side: Full file preview with syntax highlighting
        if let Some(selected_result) = self.search_results.get(self.selected_search_result) {
            self.file_preview_renderer.render_full_file_with_query(
                f,
                chunks[1],
                selected_result,
                self.file_preview_scroll_offset,
                matches!(self.focused_window, FocusedWindow::FilePreview),
                Some(&self.current_search_query),
            );
        }

        // Search input at the bottom
        SearchInputRenderer::render(
            f,
            main_chunks[1],
            &self.search_input,
            true, // Always active in search mode
            0,    // No file count needed
            &crate::types::AppState::Ready,
            "", // No spinner
            &None,
            &None,
            matches!(self.focused_window, FocusedWindow::SearchInput),
            &self.search_error,
            Some(self.search_results.len()), // Show search results count
        );
    }

    async fn execute_search(&mut self, query: &str) {
        if query.trim().is_empty() || query.trim().len() <= 2 {
            self.search_results.clear();
            self.current_search_query.clear();
            self.search_error = None;

            // Switch back to SearchInput mode when clearing results
            if !matches!(self.ui_mode, UIMode::SearchInput) {
                self.ui_mode = UIMode::SearchInput;
                self.update_focused_window();
            }
            return;
        }

        // Clear any previous error
        self.search_error = None;

        // Store the current search query for highlighting
        self.current_search_query = query.to_string();

        // Initialize processing service if not already done
        if self.processing_service.is_none() {
            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
                .join("sema");

            if let Ok(service) = ProcessingService::new(&config_dir, ChunkConfig::default()).await {
                self.processing_service = Some(service);
            } else {
                self.search_error = Some("Failed to initialize search service".to_string());
                return;
            }
        }

        // Execute search using the processing service
        if let Some(ref service) = self.processing_service {
            match service.search(query, 50).await {
                Ok(results) => {
                    // Group results by file, keeping only the first chunk per file
                    self.search_results = self.group_results_by_file(results);
                    self.selected_search_result = 0;
                    self.search_results_scroll_offset = 0;

                    // Auto-switch to SearchResults mode if we have results and are in SearchInput mode
                    if !self.search_results.is_empty()
                        && matches!(self.ui_mode, UIMode::SearchInput)
                    {
                        self.ui_mode = UIMode::SearchResults;
                        self.update_focused_window();
                    }

                    self.data_changed = true;
                }
                Err(e) => {
                    self.search_error = Some(format!("Search failed: {}", e));
                    self.search_results.clear();

                    // Switch back to SearchInput mode on error
                    if !matches!(self.ui_mode, UIMode::SearchInput) {
                        self.ui_mode = UIMode::SearchInput;
                        self.update_focused_window();
                    }
                }
            }
        }
    }

    /// Group search results by file, keeping the first (highest scoring) chunk per file
    fn group_results_by_file(&self, results: Vec<SearchResult>) -> Vec<SearchResult> {
        use std::collections::HashMap;

        let mut file_results: HashMap<PathBuf, SearchResult> = HashMap::new();

        for result in results {
            let file_path = result.chunk.file_path.clone();

            // Only keep the first result per file (highest score since results are sorted)
            file_results.entry(file_path).or_insert(result);
        }

        // Convert back to vector and sort by score
        let mut grouped_results: Vec<SearchResult> = file_results.into_values().collect();
        grouped_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        grouped_results
    }

    /// Update focused window based on current UI mode
    fn update_focused_window(&mut self) {
        self.focused_window = match self.ui_mode {
            UIMode::SearchInput => FocusedWindow::SearchInput,
            UIMode::SearchResults => FocusedWindow::SearchResults,
            UIMode::FilePreview => FocusedWindow::FilePreview,
        };
    }
}
