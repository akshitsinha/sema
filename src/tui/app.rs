use anyhow::Result;
use ratatui::crossterm::{
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

use crate::config::Config;
use crate::crawler::FileCrawler;
use crate::storage::StorageManager;

use super::engine::Engine;
use super::events::{EventHandler, EventResult};
use super::ui::UI;

const POLL_INTERVAL_MS: u64 = 100;
const SPINNER_UPDATE_INTERVAL_MS: u64 = 100;

pub struct App {
    engine: Engine,
}

impl App {
    pub fn new_with_directory(directory: PathBuf, config: Config) -> Result<Self> {
        let engine = Engine::new(directory, config);

        Ok(Self { engine })
    }

    pub async fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

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

    async fn run_main_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        let mut last_tick = Instant::now();

        let config_dir = match dirs::config_dir() {
            Some(dir) => dir,
            None => match std::env::current_dir() {
                Ok(dir) => dir,
                Err(_) => PathBuf::from("."),
            },
        }
        .join("sema");

        self.engine.state = crate::types::AppState::Crawling;
        terminal.draw(|f| UI::render(f, &mut self.engine))?;

        let crawler = FileCrawler::new(self.engine.crawler_config.clone());
        let files = crawler.crawl_directory(&self.engine.root_path).await?;

        self.engine.state = crate::types::AppState::Chunking;
        terminal.draw(|f| UI::render(f, &mut self.engine))?;

        let mut service = StorageManager::new(&config_dir).await?;
        service.process_and_index_files(files).await?;

        self.engine.processing_service = Some(service);
        self.engine.state = crate::types::AppState::Ready;
        terminal.draw(|f| UI::render(f, &mut self.engine))?;

        loop {
            if ratatui::crossterm::event::poll(Duration::from_millis(POLL_INTERVAL_MS))?
                && let Ok(event) = event::read()
            {
                let terminal_size = terminal.size()?;
                let _ = self.handle_event(event, terminal_size.height).await;
            }

            if last_tick.elapsed() >= Duration::from_millis(SPINNER_UPDATE_INTERVAL_MS) {
                self.engine.spinner_frame = (self.engine.spinner_frame + 1) % 8;
                terminal.draw(|f| UI::render(f, &mut self.engine))?;
                last_tick = Instant::now();
            }

            if self.engine.should_quit {
                break;
            }
        }

        Ok(())
    }

    async fn handle_event(&mut self, event: Event, terminal_height: u16) -> bool {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.engine.search_error = None;
                let prev_selected = self.engine.selected_search_result;

                let result = if matches!(self.engine.state, crate::types::AppState::Ready) {
                    let current_result = self
                        .engine
                        .search_results
                        .get(self.engine.selected_search_result);
                    EventHandler::handle_key_input(
                        &key,
                        &mut self.engine.search_input,
                        &mut self.engine.ui_mode,
                        &mut self.engine.selected_search_result,
                        &mut self.engine.search_results_scroll_offset,
                        &mut self.engine.file_preview_scroll_offset,
                        self.engine.search_results.len(),
                        current_result,
                        terminal_height,
                    )
                    .await
                } else {
                    EventHandler::handle_non_ready_input(&key, &mut self.engine.search_input)
                };

                match result {
                    EventResult::ExecuteSearch(query) => self.execute_search(&query).await,
                    EventResult::OpenFile => self.open_file().await,
                    EventResult::Quit => self.engine.should_quit = true,
                    EventResult::Continue => {}
                }

                if self.engine.selected_search_result != prev_selected {
                    self.sync_file_preview().await;
                }

                true
            }
            Event::Mouse(mouse)
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) =>
            {
                if matches!(self.engine.state, crate::types::AppState::Ready)
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
        if query.trim().len() <= 2 {
            self.engine.clear_search();
            return;
        }

        if self.engine.execute_search(query).await.is_err() {
            self.engine.search_error = Some("Search failed".to_string());
            self.engine.clear_search();
            return;
        }

        if let Some(first) = self.engine.search_results.first().cloned() {
            self.engine
                .update_current_file_content(&first.chunk.file_path)
                .await;
            self.engine.file_preview_scroll_offset = first.chunk.start_line.saturating_sub(1);
        }
    }

    async fn open_file(&mut self) {
        let Some(result) = self
            .engine
            .search_results
            .get(self.engine.selected_search_result)
            .cloned()
        else {
            self.engine.ui_mode = crate::types::UIMode::FilePreview;
            return;
        };

        self.engine
            .update_current_file_content(&result.chunk.file_path)
            .await;
        self.engine.file_preview_scroll_offset = result.chunk.start_line.saturating_sub(1);
        self.engine.ui_mode = crate::types::UIMode::FilePreview;
    }

    async fn sync_file_preview(&mut self) {
        let Some(result) = self
            .engine
            .search_results
            .get(self.engine.selected_search_result)
            .cloned()
        else {
            return;
        };

        let needs_load = self.engine.current_file_path.as_ref() != Some(&result.chunk.file_path);
        if needs_load {
            self.engine
                .update_current_file_content(&result.chunk.file_path)
                .await;
        }

        self.engine.file_preview_scroll_offset = result.chunk.start_line.saturating_sub(1);
    }
}
