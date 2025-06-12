use crate::search::SearchResult;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem},
};
use std::path::PathBuf;

pub struct SearchResultsRenderer;

impl SearchResultsRenderer {
    pub fn render(
        f: &mut Frame,
        area: Rect,
        results: &[SearchResult],
        selected_index: usize,
        scroll_offset: usize,
        root_path: &PathBuf,
        is_focused: bool,
    ) {
        let border_color = if is_focused {
            Color::Red // Red when focused/selected
        } else {
            Color::Black // Black when not selected
        };

        let results_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(format!(" Search Results ({}) ", results.len()))
            .title_style(
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Reset));

        if results.is_empty() {
            let empty_message = "No search results";
            let empty_para = ratatui::widgets::Paragraph::new(vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    empty_message,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )]),
            ])
            .alignment(ratatui::layout::Alignment::Center)
            .block(results_block);

            f.render_widget(empty_para, area);
            return;
        }

        // Calculate how many results we can display
        let available_height = area.height.saturating_sub(2); // Account for borders
        let max_display_results = available_height as usize;

        // Adjust selected index for results
        let adjusted_selected = if selected_index < results.len() {
            selected_index
        } else if !results.is_empty() {
            results.len() - 1
        } else {
            0
        };

        // Calculate the range of results to display based on scroll offset
        let start_index = scroll_offset;
        let end_index = std::cmp::min(start_index + max_display_results, results.len());

        let result_items: Vec<ListItem> = results
            .iter()
            .skip(start_index)
            .take(end_index - start_index)
            .enumerate()
            .map(|(display_idx, result)| {
                let actual_idx = start_index + display_idx;
                let is_selected = actual_idx == adjusted_selected;

                // Get the file path relative to root - show only path and name
                let relative_path =
                    if let Ok(rel_path) = result.chunk.file_path.strip_prefix(root_path) {
                        rel_path.to_string_lossy().to_string()
                    } else {
                        result.chunk.file_path.to_string_lossy().to_string()
                    };

                let (text_style, bg_color) = if is_selected {
                    (
                        Style::default()
                            .fg(Color::Black)
                            .add_modifier(Modifier::BOLD),
                        Color::LightBlue,
                    )
                } else {
                    (Style::default().fg(Color::Reset), Color::Reset)
                };

                let line = Line::from(vec![Span::styled(relative_path, text_style)]);
                ListItem::new(line).style(Style::default().bg(bg_color))
            })
            .collect();

        let results_list = List::new(result_items).block(results_block);
        f.render_widget(results_list, area);
    }
}
