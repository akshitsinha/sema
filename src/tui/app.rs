use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::types::FocusedWindow;

use super::engine::{Engine, StateUpdate};
use super::events::{EventHandler, EventResult};
use super::ui::UI;

const POLL_INTERVAL_MS: u64 = 100;
const SPINNER_UPDATE_INTERVAL_MS: u64 = 100;

pub struct App {
    engine: Engine,
    state_rx: mpsc::UnboundedReceiver<StateUpdate>,
    state_tx: mpsc::UnboundedSender<StateUpdate>,
    crawler_handle: Option<tokio::task::JoinHandle<Result<()>>>,
}

impl App {
    pub fn new_with_directory(directory: PathBuf, config: Config) -> Result<Self> {
        let (state_tx, state_rx) = mpsc::unbounded_channel();
        let engine = Engine::new(directory, config);

        Ok(Self {
            engine,
            state_rx,
            state_tx,
            crawler_handle: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        self.start_crawler().await?;
        let result = self.run_main_loop(&mut terminal).await;

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
        let handle = self.engine.start_crawler(self.state_tx.clone()).await?;
        self.crawler_handle = Some(handle);
        Ok(())
    }

    async fn run_main_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        let mut last_tick = Instant::now();
        let mut needs_redraw = true;

        loop {
            self.process_state_updates().await;

            if self.engine.data_changed {
                needs_redraw = true;
                self.engine.data_changed = false;
            }

            if needs_redraw {
                terminal.draw(|f| UI::render(f, &mut self.engine))?;
                needs_redraw = false;
            }

            if crossterm::event::poll(Duration::from_millis(POLL_INTERVAL_MS))? {
                if let Ok(event) = event::read() {
                    needs_redraw = self.handle_event(event).await;
                }
            }

            if self.should_update_spinner()
                && last_tick.elapsed() >= Duration::from_millis(SPINNER_UPDATE_INTERVAL_MS)
            {
                self.engine.spinner_frame = (self.engine.spinner_frame + 1) % 8;
                needs_redraw = true;
                last_tick = Instant::now();
            }

            if self.engine.should_quit {
                break;
            }
        }

        if let Some(handle) = self.crawler_handle.take() {
            handle.abort();
        }

        Ok(())
    }

    async fn process_state_updates(&mut self) {
        while let Ok(update) = self.state_rx.try_recv() {
            match update {
                StateUpdate::FileFound(file) => {
                    self.engine.app_state.crawled_files.push(file);
                    self.engine.data_changed = true;
                }
                StateUpdate::StateChanged(new_state) => {
                    self.engine.app_state.state = new_state;
                    self.engine.data_changed = true;
                }
                StateUpdate::AllFilesCollected(files) => {
                    let state_tx = self.state_tx.clone();
                    let max_file_size = self.engine.crawler_config.max_file_size;
                    tokio::spawn(async move {
                        Engine::start_chunking(state_tx, files, max_file_size).await;
                    });
                }
                StateUpdate::CrawlingCompleted {
                    files_count,
                    duration_secs,
                } => {
                    self.engine.crawling_stats = Some((files_count, duration_secs));
                    self.engine.data_changed = true;
                }
                StateUpdate::ProcessingCompleted {
                    chunks_count,
                    duration_secs,
                } => {
                    self.engine.processing_stats = Some((chunks_count, duration_secs));
                    self.engine.data_changed = true;
                    // Don't set to Ready yet - embedding generation will start
                }
                StateUpdate::EmbeddingStarted => {
                    self.engine.data_changed = true;
                }
                StateUpdate::EmbeddingCompleted {
                    chunks_count,
                    duration_secs,
                } => {
                    self.engine.embedding_stats = Some((chunks_count, duration_secs));
                    self.engine.data_changed = true;
                    self.engine.app_state.state = crate::types::AppState::Ready;
                    self.engine.ui_mode = crate::types::UIMode::SearchInput;
                    self.engine.update_focused_window();
                }
            }
        }
    }

    async fn handle_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                // Clear search error on any key press
                if self.engine.search_error.is_some() {
                    self.engine.search_error = None;
                }

                let result = match self.engine.app_state.state {
                    crate::types::AppState::Ready => {
                        let current_search_result = if !self.engine.search_results.is_empty()
                            && self.engine.selected_search_result < self.engine.search_results.len()
                        {
                            Some(&self.engine.search_results[self.engine.selected_search_result])
                        } else {
                            None
                        };

                        EventHandler::handle_key_input(
                            &key,
                            &mut self.engine.search_input,
                            &mut self.engine.ui_mode,
                            &mut self.engine.selected_search_result,
                            &mut self.engine.search_results_scroll_offset,
                            &mut self.engine.file_preview_scroll_offset,
                            self.engine.search_results.len(),
                            current_search_result,
                        )
                        .await
                    }
                    _ => EventHandler::handle_non_ready_input(&key, &mut self.engine.search_input),
                };

                match result {
                    EventResult::ExecuteSearch(query) => {
                        self.execute_search(&query).await;
                    }
                    EventResult::OpenFile(file_path) => {
                        self.open_file(&file_path).await;
                    }
                    EventResult::ClearFileCache => {
                        self.engine.clear_file_cache();
                        // Reset scroll offset when going back to chunk view
                        if matches!(self.engine.ui_mode, crate::types::UIMode::SearchResults) {
                            self.engine.file_preview_scroll_offset = 0;
                        }
                    }
                    EventResult::Quit => {
                        self.engine.should_quit = true;
                    }
                    EventResult::Continue => {}
                }

                // Auto-load file if we're in file preview mode and selection changed
                self.auto_load_file_preview().await;

                self.engine.update_focused_window();
                true
            }
            Event::Mouse(mouse)
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) =>
            {
                self.engine.focused_window = FocusedWindow::SearchInput;
                if matches!(self.engine.app_state.state, crate::types::AppState::Ready)
                    && !self.engine.search_results.is_empty()
                {
                    self.engine.ui_mode = crate::types::UIMode::SearchInput;
                }
                true
            }
            _ => false,
        }
    }

    async fn execute_search(&mut self, query: &str) {
        // Clear file cache when starting a new search
        self.engine.clear_file_cache();

        if query.trim().is_empty() || query.trim().len() <= 2 {
            self.engine.clear_search();
            return;
        }

        if self.engine.execute_search(query).await.is_err() {
            self.engine.search_error = Some("Search failed".to_string());
            self.engine.clear_search();
        }
    }

    async fn open_file(&mut self, _file_path: &str) {
        // Get the current search result to determine file and scroll position
        let (file_path, scroll_offset) = if let Some(selected_result) = self
            .engine
            .search_results
            .get(self.engine.selected_search_result)
        {
            let file_path = selected_result.chunk.file_path.clone();
            let scroll_offset = self
                .engine
                .calculate_search_result_line_offset(selected_result);
            (Some(file_path), scroll_offset)
        } else {
            (None, 0)
        };

        // Load the full file content if we have a file path
        if let Some(file_path) = file_path {
            if self.engine.load_file_content(&file_path).await.is_ok() {
                self.engine.file_preview_scroll_offset = scroll_offset;
            }
        }

        // Switch to file preview mode
        self.engine.ui_mode = crate::types::UIMode::FilePreview;
        self.engine.update_focused_window();
    }

    async fn auto_load_file_preview(&mut self) {
        // Only auto-load if we're in file preview mode and navigating between results
        if matches!(self.engine.ui_mode, crate::types::UIMode::FilePreview) {
            if let Some(selected_result) = self
                .engine
                .search_results
                .get(self.engine.selected_search_result)
            {
                let file_path = &selected_result.chunk.file_path;

                // Check if we need to load a different file
                let needs_new_file = match &self.engine.cached_file_path {
                    Some(cached_path) => cached_path != file_path,
                    None => true,
                };

                if needs_new_file {
                    // Load the new file content and position at the search result
                    let file_path = file_path.clone();
                    let scroll_offset = self
                        .engine
                        .calculate_search_result_line_offset(selected_result);

                    if self.engine.load_file_content(&file_path).await.is_ok() {
                        self.engine.file_preview_scroll_offset = scroll_offset;
                    }
                }
            }
        }
    }

    fn should_update_spinner(&self) -> bool {
        matches!(
            self.engine.app_state.state,
            crate::types::AppState::Crawling
                | crate::types::AppState::Chunking
                | crate::types::AppState::GeneratingEmbeddings
        )
    }
}
