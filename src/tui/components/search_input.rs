use crate::types::{AppState};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

pub struct SearchInputRenderer;

impl SearchInputRenderer {
    pub fn render(
        f: &mut Frame,
        area: Rect,
        search_input: &str,
        search_mode: bool,
        total_files: usize,
        state: &AppState,
        spinner_char: &str,
        chunks_created: usize,
        crawling_duration: Option<std::time::Duration>,
        chunking_duration: Option<std::time::Duration>,
    ) {
        let search_color = if search_mode {
            Color::Magenta // Purple when active
        } else {
            Color::Gray // Gray when inactive
        };

        // Generate title based on indexing state and file count
        let title = match state {
            AppState::Crawling => {
                format!(" {} Indexing... ", spinner_char)
            }
            AppState::Chunking => {
                if let Some(duration) = crawling_duration {
                    format!(" {} files crawled in {:.1}s {} Chunking ", 
                           total_files, 
                           duration.as_secs_f64(), 
                           spinner_char)
                } else {
                    format!(" {} Chunking ", spinner_char)
                }
            }
            AppState::Ready => {
                if chunks_created > 0 {
                    if let Some(duration) = chunking_duration {
                        format!(" {} chunks processed in {:.1}s ", chunks_created, duration.as_secs_f64())
                    } else {
                        format!(" Chunked {} ", chunks_created)
                    }
                } else {
                    format!(" {} files ", total_files)
                }
            }
        };

        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(search_color))
            .title(title)
            .title_style(
                Style::default()
                    .fg(Color::Reset) // Use default terminal text color
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Reset));

        // Show the search input or a placeholder
        let search_text = if search_input.is_empty() {
            if search_mode {
                "" // Active but empty
            } else {
                "Press '/' to search, 'q' or ESC to exit"
            }
        } else {
            search_input
        };

        let search_line = Line::from(vec![Span::styled(
            search_text,
            if search_mode {
                Style::default()
                    .fg(Color::Reset) // Use default terminal text color
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        )]);

        let search_para = Paragraph::new(vec![search_line]).block(search_block);

        f.render_widget(search_para, area);

        // Show cursor when in search mode
        if search_mode {
            let cursor_x = area.x + 1 + search_input.len() as u16;
            let cursor_y = area.y + 1;
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }
}
