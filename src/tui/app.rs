use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::Block,
};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::crawler::FileCrawler;
use crate::storage::service::ProcessingService;
use crate::types::{ChunkConfig, CrawlerConfig, FileEntry};

use super::components::{ColorManager, FileListRenderer, SearchInputRenderer};
use super::handlers::KeyboardHandler;
use super::utils::SpinnerUtils;

#[derive(Clone)]
pub struct AppStateData {
    pub crawled_files: Vec<FileEntry>,
    pub state: crate::types::AppState,
}

#[derive(Debug)]
pub enum StateUpdate {
    FileFound(FileEntry),
    StateChanged(crate::types::AppState),
    AllFilesCollected(Vec<FileEntry>),
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
        tokio::spawn(async move {
            let mut collected_files = Vec::new();
            while let Some(file_entry) = file_rx.recv().await {
                let _ = state_tx.send(StateUpdate::FileFound(file_entry.clone()));
                collected_files.push(file_entry);
            }
            if !collected_files.is_empty() {
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
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        needs_redraw = true;

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
        files: Vec<FileEntry>,
        max_file_size: u64,
    ) {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
            .join("sema");

        let processing_service =
            match ProcessingService::new(&config_dir, ChunkConfig::default()).await {
                Ok(service) => service,
                Err(e) => {
                    eprintln!("Failed to initialize processing service: {}", e);
                    let _ = state_tx.send(StateUpdate::StateChanged(crate::types::AppState::Ready));
                    return;
                }
            };

        match processing_service.process_files(files, max_file_size).await {
            Ok(()) => {
                let _ = state_tx.send(StateUpdate::StateChanged(crate::types::AppState::Ready));
            }
            Err(e) => {
                eprintln!("Failed to process files: {}", e);
                let _ = state_tx.send(StateUpdate::StateChanged(crate::types::AppState::Ready));
            }
        }

        processing_service.close().await;
    }

    fn ui(&self, f: &mut Frame) {
        let state = self.app_state.state.clone();
        let files = &self.app_state.crawled_files;

        let area = f.area();
        let background = Block::default().style(Style::default().bg(Color::Reset));
        f.render_widget(background, area);

        match state {
            crate::types::AppState::Crawling
            | crate::types::AppState::Chunking
            | crate::types::AppState::Ready => self.render_ready(f, area, files),
        }
    }

    fn render_ready(&self, f: &mut Frame, area: Rect, files: &[FileEntry]) {
        let state = self.app_state.state.clone();

        let file_refs: Vec<&FileEntry> = files.iter().collect();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);

        FileListRenderer::render(
            f,
            chunks[0],
            &file_refs,
            &self.root_path,
            self.selected_file_index,
            self.file_list_scroll_offset,
            &self.color_manager,
        );

        let spinner_char = SpinnerUtils::get_spinner_char(self.spinner_frame);
        SearchInputRenderer::render(
            f,
            chunks[1],
            &self.search_input,
            self.search_mode,
            files.len(),
            &state,
            spinner_char,
        );
    }
}
