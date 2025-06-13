use super::colors::ColorManager;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
};
use std::path::PathBuf;

pub struct FileListRenderer;

impl FileListRenderer {
    pub fn render(
        f: &mut Frame,
        area: Rect,
        files: &[&PathBuf],
        root_path: &PathBuf,
        selected_file_index: usize,
        file_list_scroll_offset: usize,
        color_manager: &ColorManager,
        is_focused: bool,
    ) {
        let border_color = if is_focused {
            Color::Red // Red when focused/selected
        } else {
            Color::Black // Black when not selected
        };

        let files_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(Color::Reset));

        // Calculate how many files we can display
        let available_height = area.height.saturating_sub(2); // Account for borders
        let max_display_files = available_height as usize;

        if files.is_empty() {
            let empty_message = "No files found";

            let empty_para = Paragraph::new(vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    empty_message,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )]),
            ])
            .alignment(Alignment::Center)
            .block(files_block);

            f.render_widget(empty_para, area);
            return;
        }

        // Adjust selected index for files
        let adjusted_selected = if selected_file_index < files.len() {
            selected_file_index
        } else if !files.is_empty() {
            files.len() - 1
        } else {
            0
        };

        // Calculate the range of files to display based on scroll offset
        let start_index = file_list_scroll_offset;
        let end_index = std::cmp::min(start_index + max_display_files, files.len());

        let file_items: Vec<ListItem> = files
            .iter()
            .skip(start_index)
            .take(end_index - start_index)
            .enumerate()
            .map(|(display_idx, file)| {
                let actual_idx = start_index + display_idx;
                let is_selected = actual_idx == adjusted_selected;

                // Get the full path relative to root
                let full_path = if let Ok(relative_path) = file.strip_prefix(root_path) {
                    relative_path.to_string_lossy().to_string()
                } else {
                    file.to_string_lossy().to_string()
                };

                let extension = file.extension().and_then(|e| e.to_str()).unwrap_or("");

                // Color code by file type with dynamic assignment
                let base_color = color_manager.get_color_for_extension(extension);

                // Apply selection highlighting
                let (text_style, bg_color) = if is_selected {
                    (
                        Style::default()
                            .fg(Color::White)
                            .bg(Color::Blue)
                            .add_modifier(Modifier::BOLD),
                        Color::Blue,
                    )
                } else {
                    (
                        Style::default().fg(base_color).add_modifier(Modifier::BOLD),
                        Color::Reset,
                    )
                };

                // Show selection indicator and file info
                let prefix = if is_selected { ">" } else { " " };
                let line = Line::from(vec![
                    Span::styled(
                        format!("{} ", prefix),
                        Style::default()
                            .fg(if is_selected {
                                Color::White
                            } else {
                                Color::DarkGray
                            })
                            .bg(bg_color),
                    ),
                    Span::styled(full_path, text_style),
                ]);

                ListItem::new(line).style(Style::default().bg(bg_color))
            })
            .collect();

        let file_list = List::new(file_items).block(files_block);
        f.render_widget(file_list, area);
    }
}
