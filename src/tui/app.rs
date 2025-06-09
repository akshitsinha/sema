use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
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
use crate::types::{AppState, CrawlerConfig, FileEntry};

// Import our new modular components
use super::components::{ColorManager, FileListRenderer, SearchInputRenderer};
use super::handlers::KeyboardHandler;
use super::utils::SpinnerUtils;

#[derive(Clone)]
pub struct SharedAppState {
    pub crawled_files: Vec<FileEntry>,
    pub state: AppState,
    pub data_changed: bool, // Flag to track when data has changed
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
    // Use the new modular color manager
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
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Start crawler automatically
        self.start_crawler().await?;

        // Run the main loop
        let result = self.run_app(&mut terminal).await;

        // Restore terminal
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
        // Update state to crawling
        {
            let mut shared = self.shared_state.lock().unwrap();
            shared.state = AppState::Crawling;
        }

        let (file_tx, mut file_rx) = mpsc::unbounded_channel();

        let crawler = FileCrawler::new(self.crawler_config.clone());
        let root_path = self.root_path.clone();
        let shared_state_for_completion = Arc::clone(&self.shared_state);

        // Spawn crawler task
        let crawler_handle = tokio::spawn(async move {
            let result = crawler.crawl_directory(&root_path, file_tx).await;

            // Mark crawling as complete when done
            {
                let mut shared = shared_state_for_completion.lock().unwrap();
                shared.state = AppState::Ready;
                shared.data_changed = true;
            }

            result
        });

        // Store handle for cleanup
        self.crawler_handle = Some(crawler_handle);

        // Spawn background task to handle file messages
        let shared_state_clone = Arc::clone(&self.shared_state);
        let file_handle = tokio::spawn(async move {
            while let Some(file_entry) = file_rx.recv().await {
                let mut shared = shared_state_clone.lock().unwrap();
                shared.crawled_files.push(file_entry);
                shared.data_changed = true;
            }
        });

        // Store these for later cleanup
        tokio::spawn(file_handle);

        Ok(())
    }

    async fn run_app<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        let mut last_tick = Instant::now();
        let mut needs_redraw = true; // Initial draw needed

        loop {
            // Only redraw when necessary
            if needs_redraw {
                terminal.draw(|f| self.ui(f))?;
                needs_redraw = false;
            }

            // Determine timeout based on current state
            let current_state = {
                let mut shared = self.shared_state.lock().unwrap();
                let state = shared.state.clone();
                if shared.data_changed {
                    shared.data_changed = false; // Reset flag
                    needs_redraw = true;
                }
                state
            };

            let timeout = if current_state == AppState::Crawling {
                // When crawling, check for events more frequently for spinner updates
                Duration::from_millis(100)
            } else {
                // When idle, wait longer to reduce CPU usage
                Duration::from_millis(500)
            };

            if crossterm::event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        needs_redraw = true; // Key press always triggers redraw

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

                            // Handle the restart case separately to avoid borrow issues
                            let should_restart = matches!(key.code, KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) && current_state == AppState::Ready);

                            if should_restart {
                                self.restart_crawler().await?;
                            } else {
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

            // Update spinner animation only when crawling and enough time has passed
            if current_state == AppState::Crawling
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

    async fn restart_crawler(&mut self) -> Result<()> {
        // Clean up existing crawler
        if let Some(handle) = self.crawler_handle.take() {
            handle.abort();
        }

        // Reset state and scroll
        {
            let mut shared = self.shared_state.lock().unwrap();
            shared.crawled_files.clear();
            shared.data_changed = false;
        }
        self.file_list_scroll_offset = 0;
        self.selected_file_index = 0;

        // Start new crawler
        self.start_crawler().await
    }

    fn ui(&self, f: &mut Frame) {
        // Get current state and files
        let (state, files) = {
            let shared = self.shared_state.lock().unwrap();
            (shared.state.clone(), shared.crawled_files.clone())
        };

        // Use the full area for content with default background
        let area = f.area();

        // Clear background with default terminal color
        let background = Block::default().style(Style::default().bg(Color::Reset));
        f.render_widget(background, area);

        // Main content based on state - use the full area
        match state {
            AppState::Crawling | AppState::Ready => self.render_ready(f, area, &files),
        }
    }

    fn render_ready(&self, f: &mut Frame, area: Rect, files: &[FileEntry]) {
        // Get current state for display
        let state = {
            let shared = self.shared_state.lock().unwrap();
            shared.state.clone()
        };

        // Convert files to the format expected by FileListRenderer
        let file_refs: Vec<&FileEntry> = files.iter().collect();

        // Split the area: file list at top, search input at bottom
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // File list (takes most space)
                Constraint::Length(3), // Search input
            ])
            .split(area);

        // Render main file list at the top
        FileListRenderer::render(
            f,
            chunks[0],
            &file_refs,
            &self.root_path,
            self.selected_file_index,
            self.file_list_scroll_offset,
            &self.color_manager,
        );

        // Render search input at the bottom
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
