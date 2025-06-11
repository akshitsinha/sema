use crate::types::AppState;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

pub struct SearchInputRenderer;

impl SearchInputRenderer {
    fn format_duration(duration_secs: f64) -> String {
        if duration_secs < 1.0 {
            format!("{:.0}ms", duration_secs * 1000.0)
        } else {
            format!("{:.1}s", duration_secs)
        }
    }

    pub fn render(
        f: &mut Frame,
        area: Rect,
        search_input: &str,
        search_mode: bool,
        total_files: usize,
        state: &AppState,
        spinner_char: &str,
        crawling_stats: &Option<(usize, f64)>,
        processing_stats: &Option<(usize, f64)>,
    ) {
        let search_color = if search_mode {
            Color::Magenta // Purple when active
        } else {
            Color::Gray // Gray when inactive
        };

        // Generate title based on indexing state and file count
        let title = match state {
            AppState::Crawling => {
                format!(" {} Crawling files... ", spinner_char)
            }
            AppState::Chunking => {
                let crawling_info = if let Some((files_count, duration)) = crawling_stats {
                    format!(
                        "Crawled {} files in {} - ",
                        files_count,
                        Self::format_duration(*duration)
                    )
                } else {
                    String::new()
                };
                format!(" {}{} Processing files... ", crawling_info, spinner_char)
            }
            AppState::Ready => {
                let mut parts = Vec::new();

                if let Some((files_count, duration)) = crawling_stats {
                    parts.push(format!(
                        "Crawled {} files in {}",
                        files_count,
                        Self::format_duration(*duration)
                    ));
                }

                if let Some((chunks_count, duration)) = processing_stats {
                    parts.push(format!(
                        "Indexed {} chunks in {}",
                        chunks_count,
                        Self::format_duration(*duration)
                    ));
                }

                if parts.is_empty() {
                    format!(" {} files indexed ", total_files)
                } else {
                    format!(" {} ", parts.join(" - "))
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
