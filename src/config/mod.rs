use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub general: GeneralConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub max_file_size: u64,
    pub file_extensions: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub follow_symlinks: bool,
    pub include_hidden: bool,
    pub ignore_gitignore: bool,
}

pub struct ConfigManager {
    config_dir: PathBuf,
    config_file: PathBuf,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            max_file_size: 10_485_760,
            file_extensions: vec![
                "rs".to_string(),
                "py".to_string(),
                "js".to_string(),
                "ts".to_string(),
                "jsx".to_string(),
                "tsx".to_string(),
                "go".to_string(),
                "java".to_string(),
                "kt".to_string(),
                "scala".to_string(),
                "c".to_string(),
                "cpp".to_string(),
                "cc".to_string(),
                "cxx".to_string(),
                "h".to_string(),
                "hpp".to_string(),
                "cs".to_string(),
                "rb".to_string(),
                "php".to_string(),
                "swift".to_string(),
                "dart".to_string(),
                "lua".to_string(),
                "pl".to_string(),
                "sh".to_string(),
                "bash".to_string(),
                "zsh".to_string(),
                "fish".to_string(),
                "ps1".to_string(),
                "bat".to_string(),
                "r".to_string(),
                "jl".to_string(), // Julia
                "hs".to_string(), // Haskell
                "elm".to_string(),
                "clj".to_string(), // Clojure
                "ex".to_string(),  // Elixir
                "erl".to_string(), // Erlang
                "vim".to_string(),
                "asm".to_string(),
                "s".to_string(),
                // Web technologies
                "html".to_string(),
                "htm".to_string(),
                "css".to_string(),
                "scss".to_string(),
                "sass".to_string(),
                "less".to_string(),
                "vue".to_string(),
                "svelte".to_string(),
                // Configuration & data formats
                "json".to_string(),
                "yaml".to_string(),
                "yml".to_string(),
                "toml".to_string(),
                "xml".to_string(),
                "ini".to_string(),
                "cfg".to_string(),
                "conf".to_string(),
                "properties".to_string(),
                "env".to_string(),
                // Documentation & text
                "md".to_string(),
                "markdown".to_string(),
                "txt".to_string(),
                "rst".to_string(),
                "adoc".to_string(),
                "asciidoc".to_string(),
                "tex".to_string(),
                "rtf".to_string(),
                // Database & query languages
                "sql".to_string(),
                "graphql".to_string(),
                "gql".to_string(),
                // Misc
                "log".to_string(),
                "csv".to_string(),
                "tsv".to_string(),
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
