use crate::types::{AppState, UIMode};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug)]
pub enum SearchKeyboardResult {
    ExecuteSearch(String), // Execute search immediately
    NoAction,              // No search-related action
}

pub struct KeyboardHandler;

impl KeyboardHandler {
    /// Check if the key combination should quit the application
    fn should_quit(key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => true,
            _ => false,
        }
    }
    pub async fn handle_search_mode_key(
        key: &KeyEvent,
        search_input: &mut String,
        search_mode: &mut bool,
        selected_file_index: &mut usize,
        file_list_scroll_offset: &mut usize,
        should_quit: &mut bool,
    ) {
        // Check for quit commands first
        if Self::should_quit(key) {
            *should_quit = true;
            return;
        }

        match key.code {
            KeyCode::Enter => {
                *search_mode = false;
            }
            KeyCode::Backspace => {
                if !search_input.is_empty() {
                    search_input.pop();
                }
                // Reset selection when search changes (even though it doesn't filter)
                *selected_file_index = 0;
                *file_list_scroll_offset = 0;
            }
            KeyCode::Char(c) => {
                search_input.push(c);
                // Reset selection when search changes (even though it doesn't filter)
                *selected_file_index = 0;
                *file_list_scroll_offset = 0;
            }
            _ => {}
        }
    }

    pub async fn handle_normal_mode_key<F>(
        key: &KeyEvent,
        should_quit: &mut bool,
        search_mode: &mut bool,
        selected_file_index: &mut usize,
        file_list_scroll_offset: &mut usize,
        get_total_count: F,
        _current_state: &AppState,
    ) -> Result<()>
    where
        F: Fn() -> usize,
    {
        // Check for quit commands first
        if Self::should_quit(key) {
            *should_quit = true;
            return Ok(());
        }

        match key.code {
            KeyCode::Esc => {
                *search_mode = true; // Return to search mode instead of quitting
            }
            KeyCode::Char('/') => {
                *search_mode = !*search_mode;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let total_count = get_total_count();
                if total_count > 0 && *selected_file_index > 0 {
                    *selected_file_index -= 1;
                    if *selected_file_index < *file_list_scroll_offset {
                        *file_list_scroll_offset = *selected_file_index;
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let total_count = get_total_count();
                if total_count > 0 && *selected_file_index + 1 < total_count {
                    *selected_file_index += 1;
                    // Auto-scroll if needed
                    let visible_height = 20; // Approximate visible items
                    if *selected_file_index >= *file_list_scroll_offset + visible_height {
                        *file_list_scroll_offset = *selected_file_index - visible_height + 1;
                    }
                }
            }
            KeyCode::PageUp => {
                let total_count = get_total_count();
                if total_count > 0 {
                    if *selected_file_index >= 10 {
                        *selected_file_index -= 10;
                    } else {
                        *selected_file_index = 0;
                    }
                    if *selected_file_index < *file_list_scroll_offset {
                        *file_list_scroll_offset = *selected_file_index;
                    }
                }
            }
            KeyCode::PageDown => {
                let total_count = get_total_count();
                if total_count > 0 {
                    if *selected_file_index + 10 < total_count {
                        *selected_file_index += 10;
                    } else {
                        *selected_file_index = total_count - 1;
                    }
                    let visible_height = 20;
                    if *selected_file_index >= *file_list_scroll_offset + visible_height {
                        *file_list_scroll_offset = *selected_file_index - visible_height + 1;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn handle_search_interface_key(
        key: &KeyEvent,
        search_input: &mut String,
        ui_mode: &mut UIMode,
        selected_search_result: &mut usize,
        search_results_scroll_offset: &mut usize,
        file_preview_scroll_offset: &mut usize,
        search_results_len: usize,
        should_quit: &mut bool,
        current_search_result: Option<&crate::search::SearchResult>,
    ) -> SearchKeyboardResult {
        // Check for quit commands first across all modes
        if Self::should_quit(key) {
            *should_quit = true;
            return SearchKeyboardResult::NoAction;
        }

        match ui_mode {
            UIMode::SearchInput => match key.code {
                KeyCode::Enter => {
                    if !search_input.trim().is_empty() {
                        *ui_mode = UIMode::SearchResults;
                        *selected_search_result = 0;
                        *search_results_scroll_offset = 0;
                        SearchKeyboardResult::ExecuteSearch(search_input.clone())
                    } else {
                        SearchKeyboardResult::NoAction
                    }
                }
                KeyCode::Backspace => {
                    if !search_input.is_empty() {
                        search_input.pop();
                    }
                    SearchKeyboardResult::NoAction
                }
                KeyCode::Char(c) => {
                    search_input.push(c);
                    SearchKeyboardResult::NoAction
                }
                _ => SearchKeyboardResult::NoAction,
            },
            UIMode::SearchResults => {
                match key.code {
                    KeyCode::Esc => {
                        *ui_mode = UIMode::SearchInput;
                        SearchKeyboardResult::NoAction
                    }
                    KeyCode::Enter => {
                        if search_results_len > 0 {
                            *ui_mode = UIMode::FilePreview;
                            // Initialize scroll to center on the search result
                            if let Some(search_result) = current_search_result {
                                let available_height = 20; // Approximate, should be passed from render context
                                *file_preview_scroll_offset = search_result
                                    .chunk
                                    .start_line
                                    .saturating_sub(available_height / 2);
                            }
                        }
                        SearchKeyboardResult::NoAction
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if *selected_search_result > 0 {
                            *selected_search_result -= 1;
                            if *selected_search_result < *search_results_scroll_offset {
                                *search_results_scroll_offset = *selected_search_result;
                            }
                        }
                        SearchKeyboardResult::NoAction
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if search_results_len > 0
                            && *selected_search_result + 1 < search_results_len
                        {
                            *selected_search_result += 1;
                            let visible_height = 20; // Approximate
                            if *selected_search_result
                                >= *search_results_scroll_offset + visible_height
                            {
                                *search_results_scroll_offset =
                                    *selected_search_result - visible_height + 1;
                            }
                        }
                        SearchKeyboardResult::NoAction
                    }
                    KeyCode::Char('/') => {
                        *ui_mode = UIMode::SearchInput;
                        SearchKeyboardResult::NoAction
                    }
                    _ => SearchKeyboardResult::NoAction,
                }
            }
            UIMode::FilePreview => match key.code {
                KeyCode::Esc => {
                    *ui_mode = UIMode::SearchResults;
                    SearchKeyboardResult::NoAction
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if *file_preview_scroll_offset > 0 {
                        *file_preview_scroll_offset -= 1;
                    }
                    SearchKeyboardResult::NoAction
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    // Simple increment for now - let the renderer handle bounds
                    *file_preview_scroll_offset += 1;
                    SearchKeyboardResult::NoAction
                }
                KeyCode::PageUp => {
                    *file_preview_scroll_offset = file_preview_scroll_offset.saturating_sub(10);
                    SearchKeyboardResult::NoAction
                }
                KeyCode::PageDown => {
                    *file_preview_scroll_offset += 10;
                    SearchKeyboardResult::NoAction
                }
                KeyCode::Char('/') => {
                    *ui_mode = UIMode::SearchInput;
                    SearchKeyboardResult::NoAction
                }
                _ => SearchKeyboardResult::NoAction,
            },
        }
    }
}
