use anyhow::Result;
use clap::Parser;
use sema::cli::Cli;
use sema::config::{Config, ConfigManager};
use sema::tui::App;
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load configuration (automatically creates if doesn't exist)
    let config = load_config(&cli).await?;

    // Determine target directory
    let target_directory = determine_target_directory(&cli, &config)?;

    // Validate directory
    validate_directory(&target_directory)?;

    // Default: Create and run TUI app
    let mut app = App::new_with_directory(target_directory, config)?;
    app.run().await?;

    Ok(())
}

async fn load_config(cli: &Cli) -> Result<Config> {
    let manager = ConfigManager::new()?;

    // Initialize configuration (creates files if they don't exist)
    manager.init()?;

    let mut config = manager.load_config()?;

    // Apply CLI overrides
    apply_cli_overrides(&mut config, cli);

    Ok(config)
}

fn apply_cli_overrides(config: &mut Config, cli: &Cli) {
    // Override max file size if specified
    if let Some(max_size) = cli.max_file_size {
        config.general.max_file_size = max_size;
    }

    // Override hidden files setting
    if cli.include_hidden {
        config.general.include_hidden = true;
    }

    // Override symlinks setting
    if cli.follow_symlinks {
        config.general.follow_symlinks = true;
    }

    // Override gitignore setting
    if cli.ignore_gitignore {
        config.general.ignore_gitignore = true;
    }

    // Add additional extensions
    if let Some(extensions) = &cli.extensions {
        for ext in extensions {
            if !config.general.file_extensions.contains(ext) {
                config.general.file_extensions.push(ext.clone());
            }
        }
    }

    // Add additional exclude patterns
    if let Some(exclude_patterns) = &cli.exclude {
        for pattern in exclude_patterns {
            if !config.general.exclude_patterns.contains(pattern) {
                config.general.exclude_patterns.push(pattern.clone());
            }
        }
    }

    // Override embedding model
    if let Some(model) = &cli.model {
        config.embeddings.model_name = model.clone();
    }
}

fn determine_target_directory(cli: &Cli, config: &Config) -> Result<PathBuf> {
    let target_directory = if let Some(dir) = &cli.directory {
        dir.canonicalize().unwrap_or_else(|_| {
            env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(dir)
        })
    } else if config.general.default_directory != "." {
        let default_dir = PathBuf::from(&config.general.default_directory);
        if default_dir.is_absolute() {
            default_dir
        } else {
            env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(default_dir)
        }
    } else {
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };

    Ok(target_directory)
}

fn validate_directory(target_directory: &PathBuf) -> Result<()> {
    if !target_directory.exists() {
        eprintln!(
            "❌ Error: Directory '{}' does not exist",
            target_directory.display()
        );
        std::process::exit(1);
    }

    if !target_directory.is_dir() {
        eprintln!(
            "❌ Error: '{}' is not a directory",
            target_directory.display()
        );
        std::process::exit(1);
    }

    Ok(())
}
