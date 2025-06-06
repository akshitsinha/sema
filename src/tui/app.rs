use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame, Terminal,
};
use std::io;
use anyhow::Result;

pub struct App {
    should_quit: bool,
    show_help: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            should_quit: false,
            show_help: false,
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Run the main loop
        let result = self.run_app(&mut terminal);

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

    fn run_app<B: ratatui::backend::Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        loop {
            terminal.draw(|f| self.ui(f))?;

            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            self.should_quit = true;
                        }
                        KeyCode::F(1) => {
                            self.show_help = !self.show_help;
                        }
                        _ => {}
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    fn ui(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Main content
                Constraint::Length(3), // Footer
            ])
            .split(f.size());

        // Header
        let header = Paragraph::new("Sema - Semantic File Search")
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(header, chunks[0]);

        // Main content
        let main_content = Paragraph::new(vec![
            Line::from(""),
            Line::from("Welcome to Sema!"),
            Line::from(""),
            Line::from("This is Milestone 1 - Basic TUI Setup"),
            Line::from(""),
            Line::from(vec![
                Span::raw("Press "),
                Span::styled("F1", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw(" for help"),
            ]),
            Line::from(vec![
                Span::raw("Press "),
                Span::styled("q", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw(" or "),
                Span::styled("Esc", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw(" to quit"),
            ]),
        ])
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Main"));
        f.render_widget(main_content, chunks[1]);

        // Footer
        let footer = Paragraph::new("Ready | Press F1 for help | Press q to quit")
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(footer, chunks[2]);

        // Help popup
        if self.show_help {
            self.render_help_popup(f, f.size());
        }
    }

    fn render_help_popup(&self, f: &mut Frame, area: Rect) {
        let popup_area = self.centered_rect(60, 50, area);
        
        // Clear the background
        f.render_widget(Clear, popup_area);
        
        let help_text = Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Sema Help", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from("Keyboard Shortcuts:"),
            Line::from(""),
            Line::from("  q, Esc    - Quit application"),
            Line::from("  F1        - Toggle this help"),
            Line::from(""),
            Line::from("Coming in future milestones:"),
            Line::from("  ↑/↓       - Navigate file list"),
            Line::from("  Tab       - Switch between panels"),
            Line::from("  Enter     - Open file in editor"),
            Line::from("  Ctrl+R    - Refresh index"),
            Line::from(""),
            Line::from("Press F1 or Esc to close this help"),
        ])
        .alignment(Alignment::Left)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Help")
                .style(Style::default().bg(Color::Black))
        );
        
        f.render_widget(help_text, popup_area);
    }

    fn centered_rect(&self, percent_x: u16, percent_y: u16, r: Rect) -> Rect {
        let popup_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ])
            .split(r);

        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ])
            .split(popup_layout[1])[1]
    }
}
