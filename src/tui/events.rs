use crate::search::SearchResult;
use crate::types::UIMode;
use crossterm::event::{KeyCode, KeyEvent};

pub enum EventResult {
    ExecuteSearch(String),
    OpenFile(String), // File path to open
    ClearFileCache,   // Clear cached file content
    Continue,
    Quit,
}

pub struct EventHandler;

impl EventHandler {
    pub async fn handle_key_input(
        key: &KeyEvent,
        search_input: &mut String,
        ui_mode: &mut UIMode,
        selected_search_result: &mut usize,
        search_results_scroll_offset: &mut usize,
        file_preview_scroll_offset: &mut usize,
        search_results_len: usize,
        current_search_result: Option<&SearchResult>,
    ) -> EventResult {
        // Calculate results per page based on terminal height (approximation)
        let results_per_page = 6; // Each result takes 3 lines, so ~6 results per typical screen

        match key.code {
            KeyCode::Char('q') => EventResult::Quit,
            KeyCode::Enter => match *ui_mode {
                UIMode::SearchInput => {
                    if !search_input.trim().is_empty() {
                        EventResult::ExecuteSearch(search_input.clone())
                    } else {
                        EventResult::Continue
                    }
                }
                UIMode::SearchResults | UIMode::FilePreview => {
                    if let Some(result) = current_search_result {
                        EventResult::OpenFile(result.chunk.file_path.to_string_lossy().to_string())
                    } else {
                        EventResult::Continue
                    }
                }
            },
            KeyCode::Esc => {
                match *ui_mode {
                    UIMode::FilePreview => {
                        *ui_mode = UIMode::SearchResults;
                        EventResult::ClearFileCache
                    }
                    UIMode::SearchResults => {
                        *ui_mode = UIMode::SearchInput;
                        EventResult::ClearFileCache
                    }
                    UIMode::SearchInput => {
                        // Clear search input and results
                        search_input.clear();
                        EventResult::ExecuteSearch(String::new())
                    }
                }
            }
            KeyCode::Tab => {
                if search_results_len > 0 {
                    match *ui_mode {
                        UIMode::SearchInput => *ui_mode = UIMode::SearchResults,
                        UIMode::SearchResults => *ui_mode = UIMode::FilePreview,
                        UIMode::FilePreview => *ui_mode = UIMode::SearchInput,
                    }
                }
                EventResult::Continue
            }
            KeyCode::Up => {
                match *ui_mode {
                    UIMode::SearchResults => {
                        if *selected_search_result > 0 {
                            *selected_search_result -= 1;
                            Self::update_scroll_offset(
                                *selected_search_result,
                                search_results_scroll_offset,
                                results_per_page,
                            );
                        }
                    }
                    UIMode::FilePreview => {
                        if *file_preview_scroll_offset > 0 {
                            *file_preview_scroll_offset -= 1;
                        }
                    }
                    _ => {}
                }
                EventResult::Continue
            }
            KeyCode::Down => {
                match *ui_mode {
                    UIMode::SearchResults => {
                        if *selected_search_result < search_results_len.saturating_sub(1) {
                            *selected_search_result += 1;
                            Self::update_scroll_offset(
                                *selected_search_result,
                                search_results_scroll_offset,
                                results_per_page,
                            );
                        }
                    }
                    UIMode::FilePreview => {
                        if let Some(_result) = current_search_result {
                            *file_preview_scroll_offset += 1;
                        }
                    }
                    _ => {}
                }
                EventResult::Continue
            }
            KeyCode::PageUp => {
                match *ui_mode {
                    UIMode::SearchResults => {
                        *selected_search_result =
                            selected_search_result.saturating_sub(results_per_page);
                        Self::update_scroll_offset(
                            *selected_search_result,
                            search_results_scroll_offset,
                            results_per_page,
                        );
                    }
                    UIMode::FilePreview => {
                        *file_preview_scroll_offset = file_preview_scroll_offset.saturating_sub(10);
                    }
                    _ => {}
                }
                EventResult::Continue
            }
            KeyCode::PageDown => {
                match *ui_mode {
                    UIMode::SearchResults => {
                        *selected_search_result = (*selected_search_result + results_per_page)
                            .min(search_results_len.saturating_sub(1));
                        Self::update_scroll_offset(
                            *selected_search_result,
                            search_results_scroll_offset,
                            results_per_page,
                        );
                    }
                    UIMode::FilePreview => {
                        *file_preview_scroll_offset += 10;
                    }
                    _ => {}
                }
                EventResult::Continue
            }
            KeyCode::Backspace => {
                if matches!(*ui_mode, UIMode::SearchInput) && !search_input.is_empty() {
                    search_input.pop();
                }
                EventResult::Continue
            }
            KeyCode::Char(c) => {
                if matches!(*ui_mode, UIMode::SearchInput) {
                    search_input.push(c);
                }
                EventResult::Continue
            }
            _ => EventResult::Continue,
        }
    }

    pub fn handle_non_ready_input(key: &KeyEvent, search_input: &mut String) -> EventResult {
        match key.code {
            KeyCode::Char('q') => EventResult::Quit,
            KeyCode::Backspace => {
                if !search_input.is_empty() {
                    search_input.pop();
                }
                EventResult::Continue
            }
            KeyCode::Char(c) => {
                search_input.push(c);
                EventResult::Continue
            }
            _ => EventResult::Continue,
        }
    }

    fn update_scroll_offset(
        selected_index: usize,
        scroll_offset: &mut usize,
        visible_height: usize,
    ) {
        if selected_index < *scroll_offset {
            *scroll_offset = selected_index;
        } else if selected_index >= *scroll_offset + visible_height {
            *scroll_offset = selected_index - visible_height + 1;
        }
    }
}
