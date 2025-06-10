use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub qdrant: QdrantConfig,
    pub embeddings: EmbeddingsConfig,
    pub tui: TuiConfig,
}

/// General application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Default directory to index if none specified
    pub default_directory: String,
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

/// Qdrant vector database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantConfig {
    /// Qdrant server URL
    pub url: String,
    /// Collection name for storing document vectors
    pub collection_name: String,
    /// API key for Qdrant (optional)
    pub api_key: Option<String>,
}

/// Embeddings configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsConfig {
    /// Model name or path
    pub model_name: String,
    /// Embedding dimension
    pub embedding_dim: usize,
    /// Maximum sequence length for the model
    pub max_sequence_length: usize,
    /// Text chunk size for processing
    pub chunk_size: usize,
    /// Overlap between chunks
    pub chunk_overlap: usize,
    /// Whether to use GPU acceleration
    pub use_gpu: bool,
    /// Batch size for processing
    pub batch_size: usize,
}

/// TUI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiConfig {
    /// Refresh rate in FPS
    pub refresh_rate: u64,
    /// Search debounce time in milliseconds
    pub search_debounce_ms: u64,
    /// Theme configuration
    pub theme: String,
    /// Key bindings
    pub key_bindings: HashMap<String, String>,
}

/// Configuration manager
pub struct ConfigManager {
    config_dir: PathBuf,
    config_file: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            qdrant: QdrantConfig::default(),
            embeddings: EmbeddingsConfig::default(),
            tui: TuiConfig::default(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_directory: ".".to_string(),
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

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:6334".to_string(),
            collection_name: "sema_documents".to_string(),
            api_key: None,
        }
    }
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            model_name: "sentence-transformers/all-MiniLM-L6-v2".to_string(),
            embedding_dim: 384,
            max_sequence_length: 512,
            chunk_size: 512,
            chunk_overlap: 50,
            use_gpu: false,
            batch_size: 32,
        }
    }
}

impl Default for TuiConfig {
    fn default() -> Self {
        let mut key_bindings = HashMap::new();
        key_bindings.insert("quit".to_string(), "q,Esc".to_string());
        key_bindings.insert("restart".to_string(), "Ctrl+r".to_string());
        key_bindings.insert("scroll_up".to_string(), "Up,k".to_string());
        key_bindings.insert("scroll_down".to_string(), "Down,j".to_string());
        key_bindings.insert("page_up".to_string(), "PageUp".to_string());
        key_bindings.insert("page_down".to_string(), "PageDown".to_string());

        Self {
            refresh_rate: 60,
            search_debounce_ms: 300,
            theme: "light".to_string(),
            key_bindings,
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

/// Get a global configuration instance
pub fn get_config() -> Result<Config> {
    let manager = ConfigManager::new()?;
    manager.init()?;
    manager.load_config()
}

/// Save configuration globally
pub fn save_config(config: &Config) -> Result<()> {
    let manager = ConfigManager::new()?;
    manager.init()?;
    manager.save_config(config)
}
