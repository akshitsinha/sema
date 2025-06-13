use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub general: GeneralConfig,
    pub tui: TuiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Maximum file size to process (in bytes)
    pub max_file_size: u64,
    /// File extensions to include in crawling
    pub file_extensions: Vec<String>,
    /// Patterns to exclude from crawling
    pub exclude_patterns: Vec<String>,
    /// Whether to follow symbolic links
    pub follow_symlinks: bool,
    /// Whether to include hidden files
    pub include_hidden: bool,
    /// Whether to ignore files listed in .gitignore files
    pub ignore_gitignore: bool,
    /// Whether to ignore common lock files (package-lock.json, Cargo.lock, etc.)
    pub ignore_lock_files: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiConfig {
    pub refresh_rate: u64,
    pub theme: String,
}

pub struct ConfigManager {
    config_dir: PathBuf,
    config_file: PathBuf,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            max_file_size: 10_485_760, // 10MB
            file_extensions: vec![
                "txt".to_string(),
                "md".to_string(),
                "rs".to_string(),
                "py".to_string(),
                "js".to_string(),
                "ts".to_string(),
                "go".to_string(),
                "java".to_string(),
                "cpp".to_string(),
                "c".to_string(),
                "json".to_string(),
                "yaml".to_string(),
                "toml".to_string(),
                "xml".to_string(),
                "log".to_string(),
            ],
            exclude_patterns: vec![
                ".git".to_string(),
                "target".to_string(),
                "node_modules".to_string(),
                ".cache".to_string(),
                "*.tmp".to_string(),
                "*.log".to_string(),
            ],
            follow_symlinks: false,
            include_hidden: false,
            ignore_gitignore: true,
            ignore_lock_files: true,
        }
    }
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            refresh_rate: 60,
            theme: "light".to_string(),
        }
    }
}

impl ConfigManager {
    pub fn new() -> Result<Self> {
        let config_dir = Self::get_config_dir()?;
        let config_file = config_dir.join("config.toml");

        Ok(Self {
            config_dir,
            config_file,
        })
    }

    pub fn get_config_dir() -> Result<PathBuf> {
        let home_dir = dirs::home_dir().context("Could not find home directory")?;
        Ok(home_dir.join(".sema"))
    }

    pub fn init(&self) -> Result<()> {
        if !self.config_dir.exists() {
            fs::create_dir_all(&self.config_dir).with_context(|| {
                format!("Failed to create config directory: {:?}", self.config_dir)
            })?;
        }

        if !self.config_file.exists() {
            let default_config = Config::default();
            self.save_config(&default_config)?;
        }

        Ok(())
    }

    pub fn load_config(&self) -> Result<Config> {
        if !self.config_file.exists() {
            let config = Config::default();
            self.save_config(&config)?;
            return Ok(config);
        }

        let config_content = fs::read_to_string(&self.config_file)
            .with_context(|| format!("Failed to read config file: {:?}", self.config_file))?;

        let config: Config = toml::from_str(&config_content)
            .with_context(|| format!("Failed to parse config file: {:?}", self.config_file))?;

        Ok(config)
    }

    pub fn save_config(&self, config: &Config) -> Result<()> {
        let config_content =
            toml::to_string_pretty(config).context("Failed to serialize configuration")?;

        fs::write(&self.config_file, config_content)
            .with_context(|| format!("Failed to write config file: {:?}", self.config_file))?;

        Ok(())
    }

    pub fn config_file_path(&self) -> &Path {
        &self.config_file
    }
}
