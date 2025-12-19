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
use tokio::sync::mpsc;

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

        let result = self.start(&mut terminal).await;

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    async fn start<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        let config_dir = Self::config_dir();

        self.engine.state = crate::types::AppState::Crawling;
        terminal.draw(|f| UI::render(f, &mut self.engine))?;

        let state_rx = self.init_background(config_dir.clone());
        self.event_loop(terminal, state_rx, config_dir).await
    }

    fn config_dir() -> PathBuf {
        dirs::config_dir()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sema")
    }

    fn init_background(&self, config_dir: PathBuf) -> mpsc::Receiver<crate::types::AppState> {
        let (state_tx, state_rx) = mpsc::channel(10);
        let root_path = self.engine.root_path.clone();
        let crawler_config = self.engine.crawler_config.clone();

        tokio::spawn(async move {
            let _ = Self::initialize(root_path, crawler_config, config_dir, state_tx).await;
        });

        state_rx
    }

    async fn initialize(
        root_path: PathBuf,
        crawler_config: crate::types::CrawlerConfig,
        config_dir: PathBuf,
        state_tx: mpsc::Sender<crate::types::AppState>,
    ) -> Result<()> {
        let crawler = FileCrawler::new(crawler_config);
        let files = crawler.crawl_directory(&root_path).await?;

        state_tx.send(crate::types::AppState::Chunking).await?;

        let mut service = StorageManager::new(&config_dir).await?;
        service.process_and_index_files(files).await?;

        state_tx.send(crate::types::AppState::Ready).await?;
        Ok(())
    }

    async fn event_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        mut state_rx: mpsc::Receiver<crate::types::AppState>,
        config_dir: PathBuf,
    ) -> Result<()> {
        let mut last_tick = Instant::now();

        loop {
            while let Ok(new_state) = state_rx.try_recv() {
                self.engine.state = new_state.clone();

                if matches!(new_state, crate::types::AppState::Ready) {
                    self.engine.processing_service = StorageManager::new(&config_dir).await.ok();
                }
            }

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
                    EventResult::ExecuteSearch(query) => {
                        if query.trim().len() > 2 {
                            if self.engine.execute_search(&query).await.is_ok() {
                                if let Some(first) = self.engine.search_results.first().cloned() {
                                    self.engine
                                        .update_current_file_content(&first.chunk.file_path)
                                        .await;
                                    self.engine.file_preview_scroll_offset =
                                        first.chunk.start_line.saturating_sub(1);
                                }
                            } else {
                                self.engine.search_error = Some("Search failed".to_string());
                                self.engine.clear_search();
                            }
                        } else {
                            self.engine.clear_search();
                        }
                    }
                    EventResult::OpenFile => {
                        if let Some(result) = self
                            .engine
                            .search_results
                            .get(self.engine.selected_search_result)
                            .cloned()
                        {
                            self.engine
                                .update_current_file_content(&result.chunk.file_path)
                                .await;
                            self.engine.file_preview_scroll_offset =
                                result.chunk.start_line.saturating_sub(1);
                        }
                        self.engine.ui_mode = crate::types::UIMode::FilePreview;
                    }
                    EventResult::Quit => self.engine.should_quit = true,
                    EventResult::Continue => {}
                }

                if self.engine.selected_search_result != prev_selected {
                    if let Some(result) = self
                        .engine
                        .search_results
                        .get(self.engine.selected_search_result)
                        .cloned()
                    {
                        if self.engine.current_file_path.as_ref() != Some(&result.chunk.file_path) {
                            self.engine
                                .update_current_file_content(&result.chunk.file_path)
                                .await;
                        }
                        self.engine.file_preview_scroll_offset =
                            result.chunk.start_line.saturating_sub(1);
                    }
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
}
