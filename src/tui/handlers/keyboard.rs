use crate::types::AppState;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

pub struct KeyboardHandler;

impl KeyboardHandler {
    pub async fn handle_search_mode_key(
        key: &KeyEvent,
        search_input: &mut String,
        search_mode: &mut bool,
        selected_file_index: &mut usize,
        file_list_scroll_offset: &mut usize,
        should_quit: &mut bool,
    ) {
        match key.code {
            KeyCode::Esc => {
                *should_quit = true;
            }
            KeyCode::Enter => {
                *search_mode = false;
            }
            KeyCode::Backspace => {
                search_input.pop();
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
        match key.code {
            KeyCode::Char('q') => {
                *should_quit = true;
            }
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
}
