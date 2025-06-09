pub struct SpinnerUtils;

impl SpinnerUtils {
    pub fn get_spinner_char(frame: usize) -> &'static str {
        const SPINNER_CHARS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
        SPINNER_CHARS[frame % SPINNER_CHARS.len()]
    }
}
