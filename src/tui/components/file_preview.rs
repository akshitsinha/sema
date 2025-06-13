use crate::search::SearchResult;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};
use regex;
use std::{collections::HashMap, fs, path::PathBuf};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

#[derive(Clone)]
struct CachedFileContent {
    file_hash: String,
    highlighted_lines: Vec<Vec<Span<'static>>>,
    search_terms: Vec<String>,
}

pub struct FilePreviewRenderer {
    cache: HashMap<PathBuf, CachedFileContent>,
}

impl Default for FilePreviewRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl FilePreviewRenderer {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Render chunk content (default mode for search results)
    pub fn render(
        &mut self,
        f: &mut Frame,
        area: Rect,
        search_result: &SearchResult,
        _scroll_offset: usize,
        is_focused: bool,
    ) {
        let border_color = if is_focused {
            Color::Red // Red when focused/selected
        } else {
            Color::Black // Black when not selected
        };

        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(format!(
                " {} (Lines {}-{}) ",
                search_result
                    .chunk
                    .file_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                search_result.chunk.start_line,
                search_result.chunk.end_line
            ))
            .title_style(
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Reset));

        // Use highlighted content from search result (just the chunk)
        let content_lines: Vec<Line> = search_result
            .highlighted_content
            .lines()
            .enumerate()
            .map(|(idx, line)| {
                let line_number = search_result.chunk.start_line + idx + 1;
                let line_num_str = format!("{:4} │ ", line_number);

                let mut spans = vec![Span::styled(
                    line_num_str,
                    Style::default().fg(Color::DarkGray),
                )];

                // Parse highlighted content with **term** markers
                let parts: Vec<&str> = line.split("**").collect();
                for (i, part) in parts.iter().enumerate() {
                    if i % 2 == 0 {
                        // Normal text
                        if !part.is_empty() {
                            spans.push(Span::styled(
                                part.to_string(),
                                Style::default().fg(Color::Reset),
                            ));
                        }
                    } else {
                        // Highlighted search term
                        if !part.is_empty() {
                            spans.push(Span::styled(
                                part.to_string(),
                                Style::default()
                                    .fg(Color::Black)
                                    .bg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        }
                    }
                }

                Line::from(spans)
            })
            .collect();

        let preview_para = Paragraph::new(content_lines)
            .block(preview_block)
            .wrap(Wrap { trim: false });

        f.render_widget(preview_para, area);
    }

    /// Render full file content (when user presses Enter to view full file)
    pub fn render_full_file(
        &mut self,
        f: &mut Frame,
        area: Rect,
        search_result: &SearchResult,
        scroll_offset: usize,
        is_focused: bool,
    ) {
        self.render_full_file_with_query(f, area, search_result, scroll_offset, is_focused, None)
    }

    /// Render full file content with custom search query for highlighting
    pub fn render_full_file_with_query(
        &mut self,
        f: &mut Frame,
        area: Rect,
        search_result: &SearchResult,
        scroll_offset: usize,
        is_focused: bool,
        search_query: Option<&str>,
    ) {
        let border_color = if is_focused {
            Color::Red // Red when focused/selected
        } else {
            Color::Black // Black when not selected
        };

        // Try to load the full file content
        let full_content = match fs::read_to_string(&search_result.chunk.file_path) {
            Ok(content) => {
                // Limit file size to 1MB to avoid performance issues
                if content.len() > 1_000_000 {
                    // Show error for very large files
                    let error_block = Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(border_color))
                        .title(" File Too Large ")
                        .title_style(
                            Style::default()
                                .fg(Color::Reset)
                                .add_modifier(Modifier::BOLD),
                        )
                        .style(Style::default().bg(Color::Reset));

                    let error_para = Paragraph::new("File is too large to display (>1MB)")
                        .block(error_block)
                        .style(Style::default().fg(Color::Yellow));
                    f.render_widget(error_para, area);
                    return;
                }
                content
            }
            Err(_) => {
                // Fallback error display
                let error_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border_color))
                    .title(" Error ")
                    .title_style(
                        Style::default()
                            .fg(Color::Reset)
                            .add_modifier(Modifier::BOLD),
                    )
                    .style(Style::default().bg(Color::Reset));

                let error_para = Paragraph::new("Error: Could not read file")
                    .block(error_block)
                    .style(Style::default().fg(Color::Red));
                f.render_widget(error_para, area);
                return;
            }
        };

        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(format!(
                " {} ({} lines) ",
                search_result
                    .chunk
                    .file_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                LinesWithEndings::from(&full_content).count()
            ))
            .title_style(
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Reset));

        // Calculate visible area
        let available_height = area.height.saturating_sub(2) as usize; // Account for borders

        // Get search terms for highlighting
        let search_terms = if let Some(query) = search_query {
            // Parse search query to extract individual terms
            query.split_whitespace().map(|s| s.to_string()).collect()
        } else {
            // Fallback to extracting from highlighted content
            Self::extract_search_terms_from_highlighted_content(&search_result.highlighted_content)
        };

        // Get or generate cached highlighted content
        let highlighted_lines = self.get_cached_highlighted_content(
            &search_result.chunk.file_path,
            &full_content,
            &search_result.chunk.file_hash,
            &search_terms,
        );

        // Ensure scroll_offset doesn't exceed file bounds
        let total_lines = highlighted_lines.len();
        let max_scroll = total_lines.saturating_sub(available_height);
        let effective_scroll = scroll_offset.min(max_scroll);

        // Calculate display range
        let start_line = effective_scroll;
        let end_line = (start_line + available_height).min(highlighted_lines.len());

        // Create display lines with line numbers
        let display_lines: Vec<Line> = highlighted_lines
            .iter()
            .skip(start_line)
            .take(end_line - start_line)
            .enumerate()
            .map(|(idx, line_spans)| {
                let line_number = start_line + idx + 1; // 1-based line numbers
                let line_num_str = format!("{:4} │ ", line_number);

                let mut spans = vec![Span::styled(
                    line_num_str,
                    Style::default().fg(Color::DarkGray),
                )];

                spans.extend(line_spans.clone());
                Line::from(spans)
            })
            .collect();

        let preview_para = Paragraph::new(display_lines)
            .block(preview_block)
            .wrap(Wrap { trim: false });

        f.render_widget(preview_para, area);

        // Clear cache if it gets too large
        self.clear_cache();
    }

    /// Apply syntax highlighting to file content and overlay search term highlighting
    fn highlight_file_content(
        content: &str,
        file_path: &str,
        search_terms: &[String],
    ) -> Vec<Vec<Span<'static>>> {
        // Initialize syntect
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        let theme = &theme_set.themes["base16-ocean.dark"];

        // Determine syntax from file extension
        let syntax = syntax_set
            .find_syntax_for_file(file_path)
            .unwrap_or(None)
            .unwrap_or_else(|| syntax_set.find_syntax_plain_text());

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut result_lines = Vec::new();

        for line in LinesWithEndings::from(content) {
            let ranges = highlighter
                .highlight_line(line, &syntax_set)
                .unwrap_or_default();
            let mut line_spans = Vec::new();

            for (style, text) in ranges {
                // Apply search term highlighting over syntax highlighting
                let highlighted_text = Self::apply_search_highlighting(text, search_terms);

                if highlighted_text.contains("**") && highlighted_text.contains("**") {
                    // Handle search highlighting markers
                    let parts: Vec<&str> = highlighted_text.split("**").collect();
                    for (i, part) in parts.iter().enumerate() {
                        if i % 2 == 0 {
                            // Normal text with syntax highlighting
                            if !part.is_empty() {
                                line_spans.push(Span::styled(
                                    part.to_string(),
                                    Self::syntect_style_to_ratatui_style(style),
                                ));
                            }
                        } else {
                            // Highlighted search term
                            if !part.is_empty() {
                                line_spans.push(Span::styled(
                                    part.to_string(),
                                    Style::default()
                                        .fg(Color::Black)
                                        .bg(Color::Yellow)
                                        .add_modifier(Modifier::BOLD),
                                ));
                            }
                        }
                    }
                } else {
                    // No search highlighting, just syntax highlighting
                    line_spans.push(Span::styled(
                        text.to_string(),
                        Self::syntect_style_to_ratatui_style(style),
                    ));
                }
            }

            result_lines.push(line_spans);
        }

        result_lines
    }

    /// Convert syntect style to ratatui style
    fn syntect_style_to_ratatui_style(syntect_style: SyntectStyle) -> Style {
        let fg_color = Color::Rgb(
            syntect_style.foreground.r,
            syntect_style.foreground.g,
            syntect_style.foreground.b,
        );

        let mut style = Style::default().fg(fg_color);

        if syntect_style
            .font_style
            .contains(syntect::highlighting::FontStyle::BOLD)
        {
            style = style.add_modifier(Modifier::BOLD);
        }
        if syntect_style
            .font_style
            .contains(syntect::highlighting::FontStyle::ITALIC)
        {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if syntect_style
            .font_style
            .contains(syntect::highlighting::FontStyle::UNDERLINE)
        {
            style = style.add_modifier(Modifier::UNDERLINED);
        }

        style
    }

    /// Apply search term highlighting to text
    fn apply_search_highlighting(text: &str, search_terms: &[String]) -> String {
        let mut result = text.to_string();

        for term in search_terms {
            if !term.is_empty() {
                let pattern = regex::Regex::new(&format!(r"(?i)\b{}\b", regex::escape(term)))
                    .unwrap_or_else(|_| regex::Regex::new(&regex::escape(term)).unwrap());

                result = pattern
                    .replace_all(&result, format!("**{}**", term))
                    .to_string();
            }
        }

        result
    }

    /// Extract search terms from highlighted content by looking for **term** patterns
    fn extract_search_terms_from_highlighted_content(highlighted_content: &str) -> Vec<String> {
        let mut terms = Vec::new();
        let parts: Vec<&str> = highlighted_content.split("**").collect();

        for (i, part) in parts.iter().enumerate() {
            if i % 2 == 1 && !part.is_empty() {
                terms.push(part.to_string());
            }
        }

        // Remove duplicates
        terms.sort();
        terms.dedup();
        terms
    }

    /// Get cached highlighted content or generate it if not cached
    fn get_cached_highlighted_content(
        &mut self,
        file_path: &PathBuf,
        content: &str,
        file_hash: &str,
        search_terms: &[String],
    ) -> &Vec<Vec<Span<'static>>> {
        // Check if we have valid cached content
        let should_regenerate = self
            .cache
            .get(file_path)
            .map(|cached| {
                // Regenerate if file hash changed or search terms changed
                cached.file_hash != file_hash || cached.search_terms != search_terms
            })
            .unwrap_or(true);

        if should_regenerate {
            let highlighted_lines =
                Self::highlight_file_content(content, &file_path.to_string_lossy(), search_terms);

            self.cache.insert(
                file_path.clone(),
                CachedFileContent {
                    file_hash: file_hash.to_string(),
                    highlighted_lines,
                    search_terms: search_terms.to_vec(),
                },
            );
        }

        &self.cache.get(file_path).unwrap().highlighted_lines
    }

    /// Clear the cache when it gets too large
    pub fn clear_cache(&mut self) {
        if self.cache.len() > 10 {
            self.cache.clear();
        }
    }

    /// Get the total number of lines in the file
    pub fn get_total_lines_for_file(file_path: &std::path::Path) -> usize {
        match fs::read_to_string(file_path) {
            Ok(content) => content.lines().count(),
            Err(_) => 0,
        }
    }

    /// Get the total number of lines in the content
    pub fn get_total_lines(search_result: &SearchResult) -> usize {
        Self::get_total_lines_for_file(&search_result.chunk.file_path)
    }
}
