use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
};
use std::env;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::crawler::FileCrawler;
use crate::types::{CrawlProgress, CrawlerConfig, FileEntry};

#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    Welcome,
    Crawling,
    Ready,
    Search,
}

#[derive(Clone)]
pub struct SharedAppState {
    pub crawler_progress: Option<CrawlProgress>,
    pub crawled_files: Vec<FileEntry>,
    pub state: AppState,
}

pub struct App {
    should_quit: bool,
    shared_state: Arc<Mutex<SharedAppState>>,
    crawler_handle: Option<tokio::task::JoinHandle<Result<()>>>,
    last_update: Instant,
    crawler_config: CrawlerConfig,
    root_path: PathBuf,
    file_list_scroll_offset: usize,
    spinner_frame: usize,
}

impl Default for App {
    fn default() -> Self {
        Self {
            should_quit: false,
            shared_state: Arc::new(Mutex::new(SharedAppState {
                crawler_progress: None,
                crawled_files: Vec::new(),
                state: AppState::Welcome,
            })),
            crawler_handle: None,
            last_update: Instant::now(),
            crawler_config: CrawlerConfig::default(),
            root_path: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            file_list_scroll_offset: 0,
            spinner_frame: 0,
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_with_directory(directory: PathBuf) -> Result<Self> {
        Ok(Self {
            should_quit: false,
            shared_state: Arc::new(Mutex::new(SharedAppState {
                crawler_progress: None,
                crawled_files: Vec::new(),
                state: AppState::Welcome,
            })),
            crawler_handle: None,
            last_update: Instant::now(),
            crawler_config: CrawlerConfig::default(),
            root_path: directory,
            file_list_scroll_offset: 0,
            spinner_frame: 0,
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

        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();
        let (file_tx, mut file_rx) = mpsc::unbounded_channel();

        let crawler = FileCrawler::new(self.crawler_config.clone());
        let root_path = self.root_path.clone();

        // Spawn crawler task
        let crawler_handle = tokio::spawn(async move {
            crawler
                .crawl_directory(&root_path, progress_tx, file_tx)
                .await
        });

        // Store handle for cleanup
        self.crawler_handle = Some(crawler_handle);

        // Spawn background tasks to handle messages
        let shared_state_clone = Arc::clone(&self.shared_state);
        let progress_handle = tokio::spawn(async move {
            while let Some(progress) = progress_rx.recv().await {
                let mut shared = shared_state_clone.lock().unwrap();
                shared.crawler_progress = Some(progress.clone());
                if progress.current_file.is_none() {
                    // Crawling finished
                    shared.state = AppState::Ready;
                }
            }
        });

        let shared_state_clone = Arc::clone(&self.shared_state);
        let file_handle = tokio::spawn(async move {
            while let Some(file_entry) = file_rx.recv().await {
                let mut shared = shared_state_clone.lock().unwrap();
                shared.crawled_files.push(file_entry);
            }
        });

        // Store these for later cleanup
        tokio::spawn(progress_handle);
        tokio::spawn(file_handle);

        Ok(())
    }

    async fn run_app<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(100);

        loop {
            terminal.draw(|f| self.ui(f))?;

            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if crossterm::event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                self.should_quit = true;
                            }
                            KeyCode::Char('r')
                                if key
                                    .modifiers
                                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
                            {
                                let current_state = {
                                    let shared = self.shared_state.lock().unwrap();
                                    shared.state.clone()
                                };
                                if current_state == AppState::Ready {
                                    self.restart_crawler().await?;
                                }
                            }
                            KeyCode::Up => {
                                let current_state = {
                                    let shared = self.shared_state.lock().unwrap();
                                    shared.state.clone()
                                };
                                if current_state == AppState::Ready {
                                    if self.file_list_scroll_offset > 0 {
                                        self.file_list_scroll_offset -= 1;
                                    }
                                }
                            }
                            KeyCode::Down => {
                                let current_state = {
                                    let shared = self.shared_state.lock().unwrap();
                                    shared.state.clone()
                                };
                                if current_state == AppState::Ready {
                                    let total_files = {
                                        let shared = self.shared_state.lock().unwrap();
                                        shared.crawled_files.len()
                                    };
                                    if self.file_list_scroll_offset + 1 < total_files {
                                        self.file_list_scroll_offset += 1;
                                    }
                                }
                            }
                            KeyCode::PageUp => {
                                let current_state = {
                                    let shared = self.shared_state.lock().unwrap();
                                    shared.state.clone()
                                };
                                if current_state == AppState::Ready {
                                    if self.file_list_scroll_offset >= 10 {
                                        self.file_list_scroll_offset -= 10;
                                    } else {
                                        self.file_list_scroll_offset = 0;
                                    }
                                }
                            }
                            KeyCode::PageDown => {
                                let current_state = {
                                    let shared = self.shared_state.lock().unwrap();
                                    shared.state.clone()
                                };
                                if current_state == AppState::Ready {
                                    let total_files = {
                                        let shared = self.shared_state.lock().unwrap();
                                        shared.crawled_files.len()
                                    };
                                    if self.file_list_scroll_offset + 10 < total_files {
                                        self.file_list_scroll_offset += 10;
                                    } else if total_files > 0 {
                                        self.file_list_scroll_offset = total_files - 1;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                // Update spinner animation when crawling
                let current_state = {
                    let shared = self.shared_state.lock().unwrap();
                    shared.state.clone()
                };
                if current_state == AppState::Crawling {
                    self.spinner_frame = (self.spinner_frame + 1) % 8;
                }
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
            shared.crawler_progress = None;
        }
        self.file_list_scroll_offset = 0;

        // Start new crawler
        self.start_crawler().await
    }

    fn ui(&self, f: &mut Frame) {
        // Get current state
        let (state, progress, files) = {
            let shared = self.shared_state.lock().unwrap();
            (
                shared.state.clone(),
                shared.crawler_progress.clone(),
                shared.crawled_files.clone(),
            )
        };

        // Use the full area for content with light background
        let area = f.area();

        // Clear background with light color
        let background = Block::default().style(Style::default().bg(Color::Rgb(248, 250, 252))); // Light blue-gray
        f.render_widget(background, area);

        // Main content based on state - use the full area
        match state {
            AppState::Welcome => self.render_crawling(f, area, &progress), // Show crawling view immediately
            AppState::Crawling => self.render_crawling(f, area, &progress),
            AppState::Ready => self.render_ready(f, area, &files),
            AppState::Search => self.render_search(f, area),
        }
    }

    fn render_crawling(&self, f: &mut Frame, area: Rect, progress: &Option<CrawlProgress>) {
        if let Some(progress) = progress {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(10), // Stats panel
                    Constraint::Length(4),  // File counter with spinner
                    Constraint::Min(4),     // Current file and details
                ])
                .split(area);

            // Statistics panel with light theme
            let stats_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(59, 130, 246))) // Blue
                .title(" Crawling Statistics ")
                .title_style(
                    Style::default()
                        .fg(Color::Rgb(17, 24, 39))
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Rgb(255, 255, 255)));

            let mb_processed = progress.bytes_processed as f64 / 1_048_576.0;
            let files_per_sec = if self.last_update.elapsed().as_secs() > 0 {
                progress.files_processed as f64 / self.last_update.elapsed().as_secs() as f64
            } else {
                0.0
            };

            let stats_lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "Files Discovered: ",
                        Style::default().fg(Color::Rgb(107, 114, 128)),
                    ),
                    Span::styled(
                        progress.files_discovered.to_string(),
                        Style::default()
                            .fg(Color::Rgb(14, 165, 233))
                            .add_modifier(Modifier::BOLD), // Sky blue
                    ),
                    Span::styled("  |  ", Style::default().fg(Color::Rgb(156, 163, 175))),
                    Span::styled(
                        "Processed: ",
                        Style::default().fg(Color::Rgb(107, 114, 128)),
                    ),
                    Span::styled(
                        progress.files_processed.to_string(),
                        Style::default()
                            .fg(Color::Rgb(34, 197, 94))
                            .add_modifier(Modifier::BOLD), // Green
                    ),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "Data Processed: ",
                        Style::default().fg(Color::Rgb(107, 114, 128)),
                    ),
                    Span::styled(
                        format!("{:.2} MB", mb_processed),
                        Style::default()
                            .fg(Color::Rgb(245, 158, 11))
                            .add_modifier(Modifier::BOLD), // Amber
                    ),
                    Span::styled("  |  ", Style::default().fg(Color::Rgb(156, 163, 175))),
                    Span::styled("Speed: ", Style::default().fg(Color::Rgb(107, 114, 128))),
                    Span::styled(
                        format!("{:.1} files/s", files_per_sec),
                        Style::default()
                            .fg(Color::Rgb(168, 85, 247))
                            .add_modifier(Modifier::BOLD), // Purple
                    ),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Errors: ", Style::default().fg(Color::Rgb(107, 114, 128))),
                    Span::styled(
                        progress.errors.len().to_string(),
                        if progress.errors.is_empty() {
                            Style::default()
                                .fg(Color::Rgb(34, 197, 94))
                                .add_modifier(Modifier::BOLD) // Green
                        } else {
                            Style::default()
                                .fg(Color::Rgb(239, 68, 68))
                                .add_modifier(Modifier::BOLD) // Red
                        },
                    ),
                ]),
                Line::from(""),
            ];

            let stats = Paragraph::new(stats_lines)
                .alignment(Alignment::Center)
                .block(stats_block);
            f.render_widget(stats, chunks[0]);

            // File counter with spinner
            let counter_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(34, 197, 94))) // Green
                .title(" File Counter ")
                .title_style(
                    Style::default()
                        .fg(Color::Rgb(17, 24, 39))
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Rgb(255, 255, 255)));

            let spinner_char = self.get_spinner_char();
            let counter_text = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        format!("{} ", spinner_char),
                        Style::default()
                            .fg(Color::Rgb(34, 197, 94))
                            .add_modifier(Modifier::BOLD),
                    ), // Green spinner
                    Span::styled(
                        "Files crawled: ",
                        Style::default().fg(Color::Rgb(107, 114, 128)),
                    ),
                    Span::styled(
                        progress.files_processed.to_string(),
                        Style::default()
                            .fg(Color::Rgb(34, 197, 94))
                            .add_modifier(Modifier::BOLD), // Green
                    ),
                    Span::styled(" / ", Style::default().fg(Color::Rgb(156, 163, 175))),
                    Span::styled(
                        progress.files_discovered.to_string(),
                        Style::default()
                            .fg(Color::Rgb(14, 165, 233))
                            .add_modifier(Modifier::BOLD), // Sky blue
                    ),
                ]),
                Line::from(""),
            ];

            let counter = Paragraph::new(counter_text)
                .alignment(Alignment::Center)
                .block(counter_block);
            f.render_widget(counter, chunks[1]);

            // Current file display
            let current_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(168, 85, 247))) // Purple
                .title(" Current Operation ")
                .title_style(
                    Style::default()
                        .fg(Color::Rgb(17, 24, 39))
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Rgb(255, 255, 255)));

            let current_text = if let Some(ref path) = progress.current_file {
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Unknown");
                let dir = path.parent().and_then(|p| p.to_str()).unwrap_or("");

                vec![
                    Line::from(""),
                    Line::from(vec![
                        Span::styled(
                            "Processing: ",
                            Style::default().fg(Color::Rgb(107, 114, 128)),
                        ),
                        Span::styled(
                            filename,
                            Style::default()
                                .fg(Color::Rgb(17, 24, 39))
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Location: ", Style::default().fg(Color::Rgb(107, 114, 128))),
                        Span::styled(dir, Style::default().fg(Color::Rgb(14, 165, 233))), // Sky blue
                    ]),
                ]
            } else {
                vec![
                    Line::from(""),
                    Line::from(vec![
                        Span::styled(
                            "Finalizing crawl operation...",
                            Style::default()
                                .fg(Color::Rgb(245, 158, 11))
                                .add_modifier(Modifier::BOLD),
                        ), // Amber
                    ]),
                    Line::from(""),
                ]
            };

            let current = Paragraph::new(current_text)
                .alignment(Alignment::Center)
                .block(current_block);
            f.render_widget(current, chunks[2]);
        } else {
            // Initial state
            let startup_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(245, 158, 11))) // Amber
                .title(" Starting Crawler ")
                .title_style(
                    Style::default()
                        .fg(Color::Rgb(17, 24, 39))
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Rgb(255, 255, 255)));

            let content = Paragraph::new(vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Initializing file crawler...",
                    Style::default().fg(Color::Rgb(245, 158, 11)),
                )]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Please wait while the system starts up",
                    Style::default().fg(Color::Rgb(107, 114, 128)),
                )]),
                Line::from(""),
            ])
            .alignment(Alignment::Center)
            .block(startup_block);
            f.render_widget(content, area);
        }
    }

    fn render_ready(&self, f: &mut Frame, area: Rect, files: &[FileEntry]) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(12), // Summary with visual elements
                Constraint::Min(0),     // File list
            ])
            .split(area);

        // Enhanced summary with statistics
        let summary_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(34, 197, 94))) // Green
            .title(" Crawling Complete ")
            .title_style(
                Style::default()
                    .fg(Color::Rgb(17, 24, 39))
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Rgb(255, 255, 255)));

        let total_files = files.len();
        let total_size: u64 = files.iter().map(|f| f.size).sum();
        let avg_size = if total_files > 0 {
            total_size / total_files as u64
        } else {
            0
        };

        // Create file type distribution
        let mut extensions = std::collections::HashMap::new();
        for file in files {
            let ext = file
                .path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("no extension");
            *extensions.entry(ext).or_insert(0) += 1;
        }
        let mut ext_vec: Vec<_> = extensions.iter().collect();
        ext_vec.sort_by(|a, b| b.1.cmp(a.1));

        let summary_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::Rgb(107, 114, 128))),
                Span::styled(
                    "READY",
                    Style::default()
                        .fg(Color::Rgb(34, 197, 94))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " for semantic search!",
                    Style::default().fg(Color::Rgb(17, 24, 39)),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "Total Files: ",
                    Style::default().fg(Color::Rgb(107, 114, 128)),
                ),
                Span::styled(
                    total_files.to_string(),
                    Style::default()
                        .fg(Color::Rgb(14, 165, 233))
                        .add_modifier(Modifier::BOLD), // Sky blue
                ),
                Span::styled("  |  ", Style::default().fg(Color::Rgb(156, 163, 175))),
                Span::styled(
                    "Total Size: ",
                    Style::default().fg(Color::Rgb(107, 114, 128)),
                ),
                Span::styled(
                    format!("{:.2} MB", total_size as f64 / 1_048_576.0),
                    Style::default()
                        .fg(Color::Rgb(245, 158, 11))
                        .add_modifier(Modifier::BOLD), // Amber
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "Average Size: ",
                    Style::default().fg(Color::Rgb(107, 114, 128)),
                ),
                Span::styled(
                    format!("{:.2} KB", avg_size as f64 / 1024.0),
                    Style::default()
                        .fg(Color::Rgb(168, 85, 247))
                        .add_modifier(Modifier::BOLD), // Purple
                ),
                Span::styled("  |  ", Style::default().fg(Color::Rgb(156, 163, 175))),
                Span::styled("Top Type: ", Style::default().fg(Color::Rgb(107, 114, 128))),
                Span::styled(
                    ext_vec
                        .first()
                        .map(|(ext, count)| format!("{} ({})", ext, count))
                        .unwrap_or_else(|| "None".to_string()),
                    Style::default()
                        .fg(Color::Rgb(59, 130, 246))
                        .add_modifier(Modifier::BOLD), // Blue
                ),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Ready for semantic search operations",
                Style::default().fg(Color::Rgb(34, 197, 94)),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Use ", Style::default().fg(Color::Rgb(107, 114, 128))),
                Span::styled(
                    "↑↓",
                    Style::default()
                        .fg(Color::Rgb(14, 165, 233))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" or ", Style::default().fg(Color::Rgb(107, 114, 128))),
                Span::styled(
                    "PgUp/PgDn",
                    Style::default()
                        .fg(Color::Rgb(14, 165, 233))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " to scroll files",
                    Style::default().fg(Color::Rgb(107, 114, 128)),
                ),
            ]),
            Line::from(""),
        ];

        let summary = Paragraph::new(summary_lines)
            .alignment(Alignment::Center)
            .block(summary_block);
        f.render_widget(summary, chunks[0]);

        // Enhanced file list with better formatting
        let files_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(59, 130, 246))) // Blue
            .title(format!(
                " Indexed Files ({}/{}) ",
                if files.is_empty() {
                    0
                } else {
                    self.file_list_scroll_offset + 1
                },
                files.len()
            ))
            .title_style(
                Style::default()
                    .fg(Color::Rgb(17, 24, 39))
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Rgb(255, 255, 255)));

        // Calculate how many files we can display
        let available_height = chunks[1].height.saturating_sub(2); // Account for borders
        let max_display_files = available_height as usize;

        // Calculate the range of files to display based on scroll offset
        let start_index = self.file_list_scroll_offset;
        let end_index = std::cmp::min(start_index + max_display_files, files.len());

        let file_items: Vec<ListItem> = files
            .iter()
            .skip(start_index)
            .take(end_index - start_index)
            .enumerate()
            .map(|(display_idx, file)| {
                let actual_idx = start_index + display_idx;
                let size_kb = file.size as f64 / 1024.0;

                // Get the full path relative to root
                let full_path = if let Ok(relative_path) = file.path.strip_prefix(&self.root_path) {
                    relative_path.to_string_lossy().to_string()
                } else {
                    file.path.to_string_lossy().to_string()
                };

                let extension = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");

                // Color code by file type with light theme colors
                let ext_color = match extension {
                    "rs" => Color::Rgb(239, 68, 68),                     // Red
                    "py" => Color::Rgb(245, 158, 11),                    // Amber
                    "js" | "ts" => Color::Rgb(34, 197, 94),              // Green
                    "md" => Color::Rgb(59, 130, 246),                    // Blue
                    "toml" | "yaml" | "yml" => Color::Rgb(168, 85, 247), // Purple
                    "txt" => Color::Rgb(107, 114, 128),                  // Gray
                    _ => Color::Rgb(17, 24, 39),                         // Dark gray
                };

                // Show full path with size
                let line = Line::from(vec![
                    Span::styled(
                        format!("{:3}. ", actual_idx + 1),
                        Style::default().fg(Color::Rgb(156, 163, 175)),
                    ),
                    Span::styled(
                        full_path,
                        Style::default().fg(ext_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" ({:.1} KB)", size_kb),
                        Style::default().fg(Color::Rgb(107, 114, 128)),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect();

        let file_list = List::new(file_items).block(files_block);
        f.render_widget(file_list, chunks[1]);
    }

    fn render_search(&self, f: &mut Frame, area: Rect) {
        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(168, 85, 247))) // Purple
            .title(" Search Interface ")
            .title_style(
                Style::default()
                    .fg(Color::Rgb(17, 24, 39))
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Rgb(255, 255, 255)));

        let content_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "Coming Soon: ",
                    Style::default().fg(Color::Rgb(107, 114, 128)),
                ),
                Span::styled(
                    "Semantic Search Interface",
                    Style::default()
                        .fg(Color::Rgb(168, 85, 247))
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Features in development:",
                Style::default().fg(Color::Rgb(245, 158, 11)),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  • ", Style::default().fg(Color::Rgb(156, 163, 175))),
                Span::styled(
                    "Vector-based semantic search",
                    Style::default().fg(Color::Rgb(17, 24, 39)),
                ),
            ]),
            Line::from(vec![
                Span::styled("  • ", Style::default().fg(Color::Rgb(156, 163, 175))),
                Span::styled(
                    "Natural language queries",
                    Style::default().fg(Color::Rgb(17, 24, 39)),
                ),
            ]),
            Line::from(vec![
                Span::styled("  • ", Style::default().fg(Color::Rgb(156, 163, 175))),
                Span::styled(
                    "Real-time search results",
                    Style::default().fg(Color::Rgb(17, 24, 39)),
                ),
            ]),
            Line::from(vec![
                Span::styled("  • ", Style::default().fg(Color::Rgb(156, 163, 175))),
                Span::styled(
                    "File content preview",
                    Style::default().fg(Color::Rgb(17, 24, 39)),
                ),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "This will be implemented in Milestone 3",
                Style::default().fg(Color::Rgb(14, 165, 233)),
            )]),
            Line::from(""),
        ];

        let content = Paragraph::new(content_lines)
            .alignment(Alignment::Center)
            .block(search_block);
        f.render_widget(content, area);
    }

    fn get_spinner_char(&self) -> &'static str {
        const SPINNER_CHARS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
        SPINNER_CHARS[self.spinner_frame % SPINNER_CHARS.len()]
    }
}
