use ratatui::style::Color;
use std::collections::HashMap;

pub struct ColorManager {
    extension_colors: HashMap<String, Color>,
    default_color: Color,
}

impl ColorManager {
    pub fn new() -> Self {
        // Fixed colors for common file extensions
        let mut extension_colors = HashMap::new();

        // Programming languages
        extension_colors.insert("rs".to_string(), Color::LightRed);
        extension_colors.insert("py".to_string(), Color::LightBlue);
        extension_colors.insert("js".to_string(), Color::LightYellow);
        extension_colors.insert("ts".to_string(), Color::Blue);
        extension_colors.insert("jsx".to_string(), Color::LightYellow);
        extension_colors.insert("tsx".to_string(), Color::Blue);
        extension_colors.insert("java".to_string(), Color::Red);
        extension_colors.insert("c".to_string(), Color::LightCyan);
        extension_colors.insert("cpp".to_string(), Color::LightCyan);
        extension_colors.insert("go".to_string(), Color::Cyan);
        extension_colors.insert("rb".to_string(), Color::Red);
        extension_colors.insert("php".to_string(), Color::Magenta);
        extension_colors.insert("swift".to_string(), Color::LightRed);

        // Web technologies
        extension_colors.insert("html".to_string(), Color::LightRed);
        extension_colors.insert("css".to_string(), Color::LightBlue);
        extension_colors.insert("scss".to_string(), Color::LightMagenta);
        extension_colors.insert("sass".to_string(), Color::LightMagenta);
        extension_colors.insert("less".to_string(), Color::LightBlue);
        extension_colors.insert("vue".to_string(), Color::Green);
        extension_colors.insert("svelte".to_string(), Color::LightRed);

        // Configuration and data
        extension_colors.insert("json".to_string(), Color::Yellow);
        extension_colors.insert("xml".to_string(), Color::LightGreen);
        extension_colors.insert("yaml".to_string(), Color::LightYellow);
        extension_colors.insert("yml".to_string(), Color::LightYellow);
        extension_colors.insert("toml".to_string(), Color::Yellow);
        extension_colors.insert("ini".to_string(), Color::LightCyan);
        extension_colors.insert("conf".to_string(), Color::LightCyan);
        extension_colors.insert("cfg".to_string(), Color::LightCyan);

        // Documentation
        extension_colors.insert("md".to_string(), Color::LightGreen);
        extension_colors.insert("markdown".to_string(), Color::LightGreen);
        extension_colors.insert("rst".to_string(), Color::LightGreen);
        extension_colors.insert("txt".to_string(), Color::LightBlue);

        // Shell scripts
        extension_colors.insert("sh".to_string(), Color::Green);
        extension_colors.insert("bash".to_string(), Color::Green);
        extension_colors.insert("zsh".to_string(), Color::Green);
        extension_colors.insert("fish".to_string(), Color::Green);
        extension_colors.insert("ps1".to_string(), Color::Blue);
        extension_colors.insert("bat".to_string(), Color::Blue);

        Self {
            extension_colors,
            default_color: Color::LightCyan, // Use a visible color for unknown extensions
        }
    }

    /// Get the color for a file extension
    pub fn get_color_for_extension(&self, extension: &str) -> Color {
        // Return default color for files without extension
        if extension.is_empty() {
            return self.default_color;
        }

        // Return predefined color or default
        self.extension_colors
            .get(&extension.to_lowercase())
            .copied()
            .unwrap_or(self.default_color)
    }
}
