use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap},
};
use std::sync::LazyLock;
use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};

use super::engine::Engine;
use crate::types::{AppState as AppStateEnum, FocusedWindow, UIMode};

const LAYOUT_SPLIT_PERCENTAGE: u16 = 30;

// Syntax highlighting setup
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

pub struct UI;

impl UI {
    pub fn render(f: &mut Frame, engine: &mut Engine) {
        let area = f.area();
        let background = Block::default().style(Style::default().bg(Color::Reset));
        f.render_widget(background, area);

        match engine.app_state.state {
            AppStateEnum::Crawling | AppStateEnum::Chunking | AppStateEnum::Ready => {
                Self::render_main_interface(f, area, engine);
            }
        }
    }

    fn render_main_interface(f: &mut Frame, area: Rect, engine: &mut Engine) {
        // If we have search results and we're in Ready state, show search interface
        if !engine.search_results.is_empty()
            && matches!(engine.app_state.state, AppStateEnum::Ready)
        {
            Self::render_search_interface(f, area, engine);
        } else {
            // Show status screen
            Self::render_status_screen(f, area, engine);
        }
    }

    fn render_search_interface(f: &mut Frame, area: Rect, engine: &mut Engine) {
        match engine.ui_mode {
            UIMode::SearchInput => {
                Self::render_status_screen(f, area, engine);
            }
            UIMode::SearchResults | UIMode::FilePreview => {
                Self::render_search_results_split(f, area, engine);
            }
        }
    }

    fn render_status_screen(f: &mut Frame, area: Rect, engine: &mut Engine) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);

        // Status message
        let (title, message) = Self::get_status_message(
            &engine.app_state.state,
            engine.spinner_frame,
            engine.search_input.value(),
            &engine.crawling_stats,
            &engine.processing_stats,
        );

        let status_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Blue))
            .title(title)
            .title_style(
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Reset));

        let mut lines = vec![Line::from("")];
        for line in message.lines() {
            lines.push(Line::from(vec![Span::styled(
                line,
                Style::default().fg(Color::DarkGray),
            )]));
        }

        let status_para = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(status_block);

        f.render_widget(status_para, chunks[0]);

        // Search input
        Self::render_search_input(f, chunks[1], engine);
    }

    fn render_search_results_split(f: &mut Frame, area: Rect, engine: &mut Engine) {
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(LAYOUT_SPLIT_PERCENTAGE),
                Constraint::Percentage(100 - LAYOUT_SPLIT_PERCENTAGE),
            ])
            .split(main_chunks[0]);

        // Left: Search results
        Self::render_search_results(f, chunks[0], engine);

        // Right: File preview
        Self::render_file_preview(f, chunks[1], engine);

        // Bottom: Search input
        Self::render_search_input(f, main_chunks[1], engine);
    }

    fn render_search_results(f: &mut Frame, area: Rect, engine: &mut Engine) {
        let is_focused = matches!(engine.focused_window, FocusedWindow::SearchResults);
        let border_color = if is_focused { Color::Red } else { Color::Black };

        // Build title with timing info if available and not yet shown
        let mut title = format!(" Search Results ({}) ", engine.search_results.len());
        if !engine.timing_shown {
            if let (Some((_, crawl_time)), Some((_, process_time))) =
                (&engine.crawling_stats, &engine.processing_stats)
            {
                let total_time = crawl_time + process_time;
                title = format!(
                    " Search Results ({}) - Indexed in {:.2}s ",
                    engine.search_results.len(),
                    total_time
                );
                engine.timing_shown = true; // Mark as shown so it disappears after first display
            }
        }

        let results_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(title)
            .title_style(
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Reset));

        if engine.search_results.is_empty() {
            let empty_para = Paragraph::new("No results found")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray))
                .block(results_block);
            f.render_widget(empty_para, area);
            return;
        }

        let visible_height = area.height.saturating_sub(2) as usize;
        // Each result takes 3 lines (filename, line info, separator), so divide by 3
        let results_per_page = (visible_height / 3).max(1);
        let start_index = engine.search_results_scroll_offset;
        let end_index = (start_index + results_per_page).min(engine.search_results.len());

        let items: Vec<ListItem> = engine.search_results[start_index..end_index]
            .iter()
            .enumerate()
            .map(|(i, result)| {
                let actual_index = start_index + i;
                let is_selected = actual_index == engine.selected_search_result;

                let file_display_path =
                    Self::get_display_path(&result.chunk.file_path, &engine.root_path);

                let (results_count, line_range) = if result.total_matches_in_file > 1 {
                    (
                        format!("+{}", result.total_matches_in_file),
                        format!("L{}-{}", result.chunk.start_line, result.chunk.end_line),
                    )
                } else {
                    (
                        String::new(),
                        format!("L{}-{}", result.chunk.start_line, result.chunk.end_line),
                    )
                };

                // Calculate spacing for right alignment of line numbers
                let available_width = area.width.saturating_sub(4) as usize; // Account for borders
                let results_count_len = results_count.len();
                let line_range_len = line_range.len();
                let middle_padding =
                    available_width.saturating_sub(results_count_len + line_range_len);

                // Only highlight filename when selected, not the entire line
                let filename_style = if is_selected {
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().add_modifier(Modifier::BOLD)
                };

                let info_line = if !results_count.is_empty() {
                    Line::from(vec![
                        Span::styled(results_count, Style::default().fg(Color::Yellow)),
                        Span::styled(" ".repeat(middle_padding), Style::default()),
                        Span::styled(line_range, Style::default().fg(Color::DarkGray)),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled(" ".repeat(middle_padding), Style::default()),
                        Span::styled(line_range, Style::default().fg(Color::DarkGray)),
                    ])
                };

                ListItem::new(vec![
                    Line::from(vec![Span::styled(
                        file_display_path.to_string(),
                        filename_style,
                    )]),
                    info_line,
                    Line::from(vec![Span::styled(
                        "─".repeat(available_width),
                        Style::default().fg(Color::DarkGray),
                    )]), // Border line between results
                ])
            })
            .collect();

        let list = List::new(items)
            .block(results_block)
            .style(Style::default());

        f.render_widget(list, area);
    }

    fn render_file_preview(f: &mut Frame, area: Rect, engine: &Engine) {
        let is_focused = matches!(engine.focused_window, FocusedWindow::FilePreview);
        let border_color = if is_focused { Color::Red } else { Color::Black };

        if let Some(selected_result) = engine.search_results.get(engine.selected_search_result) {
            let file_display_path =
                Self::get_display_path(&selected_result.chunk.file_path, &engine.root_path);

            // Check if content is loaded for the current file
            let content_to_display = if let Some(ref current_content) = engine.current_file_content
            {
                if let Some(ref current_path) = engine.current_file_path {
                    if current_path == &selected_result.chunk.file_path {
                        current_content.as_str()
                    } else {
                        "Loading file..."
                    }
                } else {
                    "Loading file..."
                }
            } else {
                "Loading file..."
            };

            // Always show as full file view
            let title = format!(" {} ", file_display_path);

            let preview_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_color))
                .title(title)
                .title_style(
                    Style::default()
                        .fg(Color::Reset)
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Reset));

            let content_lines: Vec<Line> = Self::highlight_code_content(
                content_to_display,
                &selected_result.chunk.file_path,
                engine.file_preview_scroll_offset,
                area.height.saturating_sub(2) as usize,
                &engine.current_search_query,
            );

            let preview_para = Paragraph::new(content_lines)
                .block(preview_block)
                .wrap(Wrap { trim: false });

            f.render_widget(preview_para, area);
        } else {
            let empty_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_color))
                .title(" Preview ")
                .title_style(
                    Style::default()
                        .fg(Color::Reset)
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Reset));

            let empty_para = Paragraph::new("Select a search result to preview")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray))
                .block(empty_block);

            f.render_widget(empty_para, area);
        }
    }

    fn highlight_code_content(
        content: &str,
        file_path: &std::path::Path,
        scroll_offset: usize,
        visible_lines: usize,
        search_query: &str,
    ) -> Vec<Line<'static>> {
        // Handle empty content case
        if content.is_empty() {
            return vec![Line::from(vec![Span::styled(
                "  1 │ (empty file)",
                Style::default().fg(Color::DarkGray),
            )])];
        }

        let extension = file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");

        let syntax = SYNTAX_SET
            .find_syntax_by_extension(extension)
            .or_else(|| SYNTAX_SET.find_syntax_by_first_line(content))
            .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());

        let theme = &THEME_SET.themes["base16-ocean.dark"];
        let mut highlighter = HighlightLines::new(syntax, theme);

        // Determine if this is semantic search vs text search
        let is_semantic_search = !search_query.trim().starts_with('\'');

        let search_terms: Vec<&str> = if is_semantic_search {
            // For semantic search, don't highlight query terms
            Vec::new()
        } else {
            // For text search, highlight query terms (remove the ' prefix)
            let query = search_query
                .trim()
                .strip_prefix('\'')
                .unwrap_or(search_query);
            query
                .split_whitespace()
                .filter(|term| !term.is_empty())
                .collect()
        };

        let total_lines = content.lines().count();

        // Ensure scroll offset doesn't go beyond content
        let safe_scroll_offset = scroll_offset.min(total_lines.saturating_sub(1));

        let line_number_width = (total_lines + safe_scroll_offset).to_string().len().max(3);

        let result: Vec<Line> = content
            .lines()
            .enumerate()
            .skip(safe_scroll_offset)
            .take(visible_lines)
            .map(|(line_index, line)| {
                let line_number = line_index + 1;
                let line_num_str = format!("{:>width$} │ ", line_number, width = line_number_width);

                // Skip semantic term highlighting for semantic search
                if is_semantic_search {
                    // Use syntax highlighting but no semantic highlighting
                    match highlighter.highlight_line(line, &SYNTAX_SET) {
                        Ok(ranges) => {
                            // Apply syntax highlighting only
                            let mut spans: Vec<Span> = vec![Span::styled(
                                line_num_str,
                                Style::default().fg(Color::DarkGray),
                            )];

                            let content_spans: Vec<Span> = ranges
                                .iter()
                                .map(|(style, text)| {
                                    let fg_color = Color::Rgb(
                                        style.foreground.r,
                                        style.foreground.g,
                                        style.foreground.b,
                                    );
                                    Span::styled(text.to_string(), Style::default().fg(fg_color))
                                })
                                .collect();

                            spans.extend(content_spans);
                            Line::from(spans)
                        }
                        Err(_) => {
                            // Fallback: plain text
                            let spans = vec![
                                Span::styled(line_num_str, Style::default().fg(Color::DarkGray)),
                                Span::styled(line.to_string(), Style::default()),
                            ];
                            Line::from(spans)
                        }
                    }
                } else {
                    // Use syntax highlighting for text search
                    match highlighter.highlight_line(line, &SYNTAX_SET) {
                        Ok(ranges) => {
                            // Apply syntax highlighting first
                            let mut spans: Vec<Span> = vec![Span::styled(
                                line_num_str,
                                Style::default().fg(Color::DarkGray),
                            )];

                            let content_spans: Vec<Span> = ranges
                                .iter()
                                .map(|(style, text)| {
                                    let fg_color = Color::Rgb(
                                        style.foreground.r,
                                        style.foreground.g,
                                        style.foreground.b,
                                    );
                                    Span::styled(text.to_string(), Style::default().fg(fg_color))
                                })
                                .collect();

                            spans.extend(content_spans);

                            // Apply highlighting based on search type
                            if !search_terms.is_empty() {
                                // Text search: highlight query terms
                                let (line_num_span, content_spans) = spans.split_first().unwrap();
                                let highlighted_content = Self::highlight_search_terms(
                                    content_spans.to_vec(),
                                    &search_terms,
                                );
                                let mut final_spans = vec![line_num_span.clone()];
                                final_spans.extend(highlighted_content);
                                Line::from(final_spans)
                            } else {
                                Line::from(spans)
                            }
                        }
                        Err(_) => {
                            // Fallback: plain text with highlighting
                            let spans = vec![
                                Span::styled(line_num_str, Style::default().fg(Color::DarkGray)),
                                Span::styled(line.to_string(), Style::default()),
                            ];

                            if !search_terms.is_empty() {
                                // Text search: highlight query terms
                                let (line_num_span, content_spans) = spans.split_first().unwrap();
                                let highlighted_content = Self::highlight_search_terms(
                                    content_spans.to_vec(),
                                    &search_terms,
                                );
                                let mut final_spans = vec![line_num_span.clone()];
                                final_spans.extend(highlighted_content);
                                Line::from(final_spans)
                            } else {
                                Line::from(spans)
                            }
                        }
                    }
                }
            })
            .collect();

        result
    }

    fn render_search_input(f: &mut Frame, area: Rect, engine: &Engine) {
        let is_focused = matches!(engine.focused_window, FocusedWindow::SearchInput);
        let border_color = if is_focused { Color::Red } else { Color::Black };

        let mut title = " Search ".to_string();
        if let Some(ref error) = engine.search_error {
            title = format!(" Search - {} ", error);
        } else if !engine.search_results.is_empty()
            && !engine.search_input.value().trim().is_empty()
            && matches!(engine.focused_window, FocusedWindow::SearchInput)
        {
            // Only show results count when search input is focused and has content
            title = format!(" Search - {} results ", engine.search_results.len());
        }

        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(title)
            .title_style(
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Reset));

        // Calculate width for visual scrolling (accounting for borders)
        let width = area.width.max(3) - 3;
        let scroll = engine.search_input.visual_scroll(width as usize);

        let input_widget = Paragraph::new(engine.search_input.value())
            .scroll((0, scroll as u16))
            .block(search_block);

        f.render_widget(input_widget, area);

        // Set cursor position if the search input is focused
        if is_focused {
            // Position the cursor past the end of the input text and one line down from the border
            let x = engine.search_input.visual_cursor().max(scroll) - scroll + 1;
            f.set_cursor_position((area.x + x as u16, area.y + 1));
        }
    }

    fn get_status_message(
        state: &AppStateEnum,
        spinner_frame: usize,
        search_input: &str,
        crawling_stats: &Option<(usize, f64)>,
        processing_stats: &Option<(usize, f64)>,
    ) -> (String, &'static str) {
        match state {
            AppStateEnum::Crawling => {
                let spinner_char = Self::get_spinner_char(spinner_frame);
                (
                    format!(" {} Crawling files... ", spinner_char),
                    "Discovering files in the directory.\nYou can type your search query now.",
                )
            }
            AppStateEnum::Chunking => {
                let spinner_char = Self::get_spinner_char(spinner_frame);
                (
                    format!(" {} Processing files... ", spinner_char),
                    "Breaking files into searchable chunks.\nAlmost ready for search!",
                )
            }
            AppStateEnum::Ready => {
                // Show completion stats if available and no search has been performed
                if search_input.is_empty() {
                    let mut title = " Ready to Search ".to_string();
                    let mut message =
                        "Type your search query and press Enter\nto search through indexed files.";

                    if let Some((files_count, duration)) = crawling_stats {
                        let time_unit = if *duration < 1.0 { "ms" } else { "seconds" };
                        let time_value = if *duration < 1.0 {
                            duration * 1000.0
                        } else {
                            *duration
                        };
                        title = format!(
                            " Crawled {} files in {:.1} {} ",
                            files_count, time_value, time_unit
                        );

                        if let Some((chunks_count, proc_duration)) = processing_stats {
                            let proc_time_unit = if *proc_duration < 1.0 {
                                "ms"
                            } else {
                                "seconds"
                            };
                            let proc_time_value = if *proc_duration < 1.0 {
                                proc_duration * 1000.0
                            } else {
                                *proc_duration
                            };
                            title = format!(
                                "{} - Processed {} chunks in {:.1} {}",
                                title.trim_end(),
                                chunks_count,
                                proc_time_value,
                                proc_time_unit
                            );
                        }

                        message = "Processing completed! Semantic search ready.\nType your search query and press Enter to search.";
                    }

                    (title, message)
                } else {
                    (
                        " Ready to Search ".to_string(),
                        "Press Enter to execute search, or\ncontinue typing to refine your query.",
                    )
                }
            }
        }
    }

    fn get_spinner_char(frame: usize) -> char {
        const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];
        SPINNER_CHARS[frame % SPINNER_CHARS.len()]
    }

    fn highlight_search_terms(spans: Vec<Span>, search_terms: &[&str]) -> Vec<Span<'static>> {
        let mut result = Vec::new();

        for span in spans {
            let text = span.content.to_string();
            let style = span.style;
            let mut current_pos = 0;

            // Find all search term matches in this span
            let mut matches = Vec::new();
            let text_lower = text.to_lowercase();

            for term in search_terms {
                let term_lower = term.to_lowercase();
                let mut search_pos = 0;

                while let Some(pos) = text_lower[search_pos..].find(&term_lower) {
                    let abs_pos = search_pos + pos;
                    matches.push((abs_pos, abs_pos + term.len()));
                    search_pos = abs_pos + 1;
                }
            }

            // Sort matches by position and merge overlapping ones
            matches.sort_by_key(|&(start, _)| start);
            let mut merged_matches = Vec::new();

            for (start, end) in matches {
                if let Some(&mut (_, ref mut last_end)) = merged_matches.last_mut() {
                    if start <= *last_end {
                        *last_end = (*last_end).max(end);
                        continue;
                    }
                }
                merged_matches.push((start, end));
            }

            // Split the span into highlighted and non-highlighted parts
            if merged_matches.is_empty() {
                // No matches, keep the original span
                result.push(Span::styled(text, style));
            } else {
                for (start, end) in merged_matches {
                    // Add text before the match
                    if start > current_pos {
                        result.push(Span::styled(text[current_pos..start].to_string(), style));
                    }

                    // Add the highlighted match
                    result.push(Span::styled(
                        text[start..end].to_string(),
                        Style::default()
                            .bg(Color::Yellow)
                            .fg(Color::Black)
                            .add_modifier(Modifier::BOLD),
                    ));

                    current_pos = end;
                }

                // Add remaining text
                if current_pos < text.len() {
                    result.push(Span::styled(text[current_pos..].to_string(), style));
                }
            }
        }

        result
    }

    fn get_display_path(file_path: &std::path::Path, base_dir: &std::path::Path) -> String {
        // Try to get a relative path from base directory
        if let Ok(relative) = file_path.strip_prefix(base_dir) {
            relative.to_string_lossy().to_string()
        } else {
            // Fall back to showing the last 2 components if possible and not too long
            let components: Vec<_> = file_path.components().collect();
            if components.len() >= 2 {
                let parent = components[components.len() - 2]
                    .as_os_str()
                    .to_string_lossy();
                let filename = components[components.len() - 1]
                    .as_os_str()
                    .to_string_lossy();
                let display_path = format!("{}/{}", parent, filename);

                // Check length, truncate if necessary
                if display_path.len() > 50 {
                    format!("...{}", &display_path[display_path.len() - 47..])
                } else {
                    display_path
                }
            } else {
                file_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            }
        }
    }
}
