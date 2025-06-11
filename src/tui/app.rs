use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
    },
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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::crawler::FileCrawler;
use crate::storage::service::ProcessingService;
use crate::types::{AppState, CrawlerConfig, FileEntry, ChunkConfig};

use super::components::{ColorManager, FileListRenderer, SearchInputRenderer};
use super::handlers::KeyboardHandler;
use super::utils::SpinnerUtils;

#[derive(Clone)]
pub struct SharedAppState {
    pub crawled_files: Vec<FileEntry>,
    pub state: AppState,
    pub data_changed: bool,
    pub chunks_created: usize,
    pub crawling_duration: Option<std::time::Duration>,
    pub chunking_duration: Option<std::time::Duration>,
}

pub struct App {
    should_quit: bool,
    shared_state: Arc<Mutex<SharedAppState>>,
    crawler_handle: Option<tokio::task::JoinHandle<Result<()>>>,
    crawler_config: CrawlerConfig,
    root_path: PathBuf,
    file_list_scroll_offset: usize,
    selected_file_index: usize,
    spinner_frame: usize,
    search_input: String,
    search_mode: bool,
    color_manager: ColorManager,
}

impl App {
    pub fn new_with_directory(directory: PathBuf, config: Config) -> Result<Self> {
        let crawler_config = CrawlerConfig::from(&config.general);

        Ok(Self {
            should_quit: false,
            shared_state: Arc::new(Mutex::new(SharedAppState {
                crawled_files: Vec::new(),
                state: AppState::Crawling,
                data_changed: false,
                chunks_created: 0,
                crawling_duration: None,
                chunking_duration: None,
            })),
            crawler_handle: None,
            crawler_config,
            root_path: directory,
            file_list_scroll_offset: 0,
            selected_file_index: 0,
            spinner_frame: 0,
            search_input: String::new(),
            search_mode: true,
            color_manager: ColorManager::new(),
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
        {
            let mut shared = self.shared_state.lock().unwrap();
            shared.state = AppState::Crawling;
        }

        let (file_tx, mut file_rx) = mpsc::unbounded_channel();
        let crawler = FileCrawler::new(self.crawler_config.clone());
        let root_path = self.root_path.clone();
        let shared_state_for_completion = Arc::clone(&self.shared_state);
        let max_file_size = self.crawler_config.max_file_size;

        let crawler_handle = tokio::spawn(async move {
            let crawl_start = std::time::Instant::now();
            let result = crawler.crawl_directory(&root_path, file_tx).await;

            if result.is_ok() {
                {
                    let mut shared = shared_state_for_completion.lock().unwrap();
                    shared.crawling_duration = Some(crawl_start.elapsed());
                    shared.state = AppState::Chunking;
                    shared.data_changed = true;
                }

                Self::start_chunking_process(shared_state_for_completion, max_file_size).await;
            }

            result
        });

        self.crawler_handle = Some(crawler_handle);

        let shared_state_clone = Arc::clone(&self.shared_state);
        let file_handle = tokio::spawn(async move {
            while let Some(file_entry) = file_rx.recv().await {
                let mut shared = shared_state_clone.lock().unwrap();
                shared.crawled_files.push(file_entry);
                shared.data_changed = true;
            }
        });

        tokio::spawn(file_handle);
        Ok(())
    }

    async fn run_app<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        let mut last_tick = Instant::now();
        let mut needs_redraw = true;

        loop {
            if needs_redraw {
                terminal.draw(|f| self.ui(f))?;
                needs_redraw = false;
            }

            let current_state = {
                let mut shared = self.shared_state.lock().unwrap();
                let state = shared.state.clone();
                if shared.data_changed {
                    shared.data_changed = false;
                    needs_redraw = true;
                }
                state
            };

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
                            let get_total_count = || {
                                let shared = self.shared_state.lock().unwrap();
                                shared.crawled_files.len()
                            };

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

            // Update spinner animation only when processing and enough time has passed
            if matches!(current_state, AppState::Crawling | AppState::Chunking)
                && last_tick.elapsed() >= Duration::from_millis(100)
            {
                self.spinner_frame = (self.spinner_frame + 1) % 8;
                needs_redraw = true;
                last_tick = Instant::now();
            }

            if self.should_quit {
                break;
            }
        }

        // Clean up crawler if running
        if let Some(handle) = self.crawler_handle.take() {
            handle.abort();
        }

        Ok(())
    }

    async fn start_chunking_process(shared_state: Arc<Mutex<SharedAppState>>, max_file_size: u64) {
        let files = {
            let shared = shared_state.lock().unwrap();
            shared.crawled_files.clone()
        };

        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
            .join("sema");

        let processing_service = match ProcessingService::new(&config_dir, ChunkConfig::default()).await {
            Ok(service) => service,
            Err(e) => {
                eprintln!("Failed to initialize processing service: {}", e);
                let mut shared = shared_state.lock().unwrap();
                shared.state = AppState::Ready;
                shared.data_changed = true;
                return;
            }
        };

        let chunk_start = std::time::Instant::now();
        match processing_service.process_files_parallel(files, max_file_size).await {
            Ok(stats) => {
                let mut shared = shared_state.lock().unwrap();
                shared.chunks_created = stats.chunks_created;
                shared.chunking_duration = Some(chunk_start.elapsed());
                shared.state = AppState::Ready;
                shared.data_changed = true;
            }
            Err(e) => {
                eprintln!("Failed to process files in parallel: {}", e);
                let mut shared = shared_state.lock().unwrap();
                shared.state = AppState::Ready;
                shared.data_changed = true;
            }
        }

        processing_service.close().await;
    }

    fn ui(&self, f: &mut Frame) {
        let (state, files) = {
            let shared = self.shared_state.lock().unwrap();
            (shared.state.clone(), shared.crawled_files.clone())
        };

        let area = f.area();
        let background = Block::default().style(Style::default().bg(Color::Reset));
        f.render_widget(background, area);

        match state {
            AppState::Crawling | AppState::Chunking | AppState::Ready => self.render_ready(f, area, &files),
        }
    }

    fn render_ready(&self, f: &mut Frame, area: Rect, files: &[FileEntry]) {
        let (state, chunks_created, crawling_duration, chunking_duration) = {
            let shared = self.shared_state.lock().unwrap();
            (shared.state.clone(), shared.chunks_created, shared.crawling_duration, shared.chunking_duration)
        };

        let file_refs: Vec<&FileEntry> = files.iter().collect();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(3),
            ])
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
            chunks_created,
            crawling_duration,
            chunking_duration,
        );
    }
}
