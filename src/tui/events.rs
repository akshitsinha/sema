use crate::types::{SearchResult, UIMode};
use crossterm::event::{KeyCode, KeyEvent};
use tui_input::{Input, backend::crossterm::EventHandler as InputEventHandler};

pub enum EventResult {
    ExecuteSearch(String),
    OpenFile,
    Continue,
    Quit,
}

pub struct EventHandler;

impl EventHandler {
    pub async fn handle_key_input(
        key: &KeyEvent,
        search_input: &mut Input,
        ui_mode: &mut UIMode,
        selected_search_result: &mut usize,
        search_results_scroll_offset: &mut usize,
        file_preview_scroll_offset: &mut usize,
        search_results_len: usize,
        current_search_result: Option<&SearchResult>,
        terminal_height: u16,
    ) -> EventResult {
        // Calculate results per page based on terminal height
        let results_per_page = ((terminal_height.saturating_sub(2)) / 3).max(1) as usize;

        match key.code {
            KeyCode::Char('q') => EventResult::Quit,
            KeyCode::Enter => match *ui_mode {
                UIMode::SearchInput => {
                    if !search_input.value().trim().is_empty() {
                        EventResult::ExecuteSearch(search_input.value().to_string())
                    } else {
                        EventResult::Continue
                    }
                }
                UIMode::SearchResults | UIMode::FilePreview => {
                    if current_search_result.is_some() {
                        EventResult::OpenFile
                    } else {
                        EventResult::Continue
                    }
                }
            },
            KeyCode::Esc => {
                match *ui_mode {
                    UIMode::FilePreview => {
                        *ui_mode = UIMode::SearchResults;
                        EventResult::Continue
                    }
                    UIMode::SearchResults => {
                        *ui_mode = UIMode::SearchInput;
                        EventResult::Continue
                    }
                    UIMode::SearchInput => {
                        // Clear search input and results
                        search_input.reset();
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
            KeyCode::Backspace | KeyCode::Delete => {
                if matches!(*ui_mode, UIMode::SearchInput) {
                    search_input.handle_event(&crossterm::event::Event::Key(*key));
                }
                EventResult::Continue
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                if matches!(*ui_mode, UIMode::SearchInput) {
                    search_input.handle_event(&crossterm::event::Event::Key(*key));
                }
                EventResult::Continue
            }
            KeyCode::Char(c) => {
                if matches!(*ui_mode, UIMode::SearchInput) {
                    // Handle Ctrl+A, Ctrl+W, etc.
                    search_input.handle_event(&crossterm::event::Event::Key(*key));
                } else if c == 'c'
                    && key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL)
                {
                    // Ctrl+C to quit
                    return EventResult::Quit;
                }
                EventResult::Continue
            }
            _ => EventResult::Continue,
        }
    }

    pub fn handle_non_ready_input(key: &KeyEvent, search_input: &mut Input) -> EventResult {
        match key.code {
            KeyCode::Char('q') => EventResult::Quit,
            KeyCode::Char('c')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                EventResult::Quit
            }
            KeyCode::Backspace | KeyCode::Delete => {
                search_input.handle_event(&crossterm::event::Event::Key(*key));
                EventResult::Continue
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                search_input.handle_event(&crossterm::event::Event::Key(*key));
                EventResult::Continue
            }
            KeyCode::Char(_) => {
                search_input.handle_event(&crossterm::event::Event::Key(*key));
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
