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
                    let terminal_size = terminal.size()?;
                    needs_redraw = self.handle_event(event, terminal_size.height).await;
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
                    tokio::spawn(async move {
                        Engine::start_chunking(state_tx, files).await;
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
                }
            }
        }
    }

    async fn handle_event(&mut self, event: Event, terminal_height: u16) -> bool {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if self.engine.search_error.is_some() {
                    self.engine.search_error = None;
                }

                let prev_selected = self.engine.selected_search_result;

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
                            terminal_height,
                        )
                        .await
                    }
                    _ => EventHandler::handle_non_ready_input(&key, &mut self.engine.search_input),
                };

                match result {
                    EventResult::ExecuteSearch(query) => {
                        self.execute_search(&query).await;
                    }
                    EventResult::OpenFile => {
                        self.open_file().await;
                    }
                    EventResult::Quit => {
                        self.engine.should_quit = true;
                    }
                    EventResult::Continue => {}
                }

                // Only sync file preview if selection changed
                if self.engine.selected_search_result != prev_selected {
                    self.sync_file_preview().await;
                }

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
        if query.trim().is_empty() || query.trim().len() <= 2 {
            self.engine.clear_search();
            return;
        }

        if self.engine.execute_search(query).await.is_err() {
            self.engine.search_error = Some("Search failed".to_string());
            self.engine.clear_search();
            return;
        }

        // Load first result for preview
        if !self.engine.search_results.is_empty() {
            let first_result = self.engine.search_results[0].clone();
            let file_path = &first_result.chunk.file_path;

            self.engine.update_current_file_content(file_path).await;

            let scroll_offset = self
                .engine
                .calculate_search_result_line_offset(&first_result);
            self.engine.file_preview_scroll_offset = scroll_offset;
        }
    }

    async fn open_file(&mut self) {
        let selected_result = if let Some(result) = self
            .engine
            .search_results
            .get(self.engine.selected_search_result)
        {
            result.clone()
        } else {
            self.engine.ui_mode = crate::types::UIMode::FilePreview;
            self.engine.update_focused_window();
            return;
        };

        let file_path = &selected_result.chunk.file_path;

        self.engine.update_current_file_content(file_path).await;

        let scroll_offset = self
            .engine
            .calculate_search_result_line_offset(&selected_result);

        self.engine.file_preview_scroll_offset = scroll_offset;
        self.engine.ui_mode = crate::types::UIMode::FilePreview;
        self.engine.update_focused_window();
    }

    async fn sync_file_preview(&mut self) {
        let selected_result = if let Some(result) = self
            .engine
            .search_results
            .get(self.engine.selected_search_result)
        {
            result.clone()
        } else {
            return;
        };

        let file_path = &selected_result.chunk.file_path;

        // Check if we need to load a different file
        let needs_new_file = if let Some(current_path) = &self.engine.current_file_path {
            current_path != file_path
        } else {
            true
        };

        if needs_new_file {
            self.engine.update_current_file_content(file_path).await;
        }

        // Always set the scroll offset to the chunk position
        let scroll_offset = self
            .engine
            .calculate_search_result_line_offset(&selected_result);
        self.engine.file_preview_scroll_offset = scroll_offset;
    }

    fn should_update_spinner(&self) -> bool {
        matches!(
            self.engine.app_state.state,
            crate::types::AppState::Crawling | crate::types::AppState::Chunking
        )
    }
}
