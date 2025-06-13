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
    let config = load_config(&cli).await?;
    let target_directory = resolve_directory(&cli)?;

    let mut app = App::new_with_directory(target_directory, config)?;
    app.run().await?;

    Ok(())
}

async fn load_config(cli: &Cli) -> Result<Config> {
    let manager = ConfigManager::new()?;
    manager.init()?;

    let mut config = manager.load_config()?;
    apply_cli_overrides(&mut config, cli);

    Ok(config)
}

fn apply_cli_overrides(config: &mut Config, cli: &Cli) {
    if let Some(max_size) = cli.max_file_size {
        config.general.max_file_size = max_size;
    }

    if cli.include_hidden {
        config.general.include_hidden = true;
    }

    if cli.follow_symlinks {
        config.general.follow_symlinks = true;
    }

    if cli.ignore_gitignore {
        config.general.ignore_gitignore = true;
    }

    // Override file extensions if --extensions is provided
    if let Some(extensions) = &cli.extensions {
        config.general.file_extensions = extensions.clone();
    }

    if let Some(exclude_patterns) = &cli.exclude {
        for pattern in exclude_patterns {
            if !config.general.exclude_patterns.contains(pattern) {
                config.general.exclude_patterns.push(pattern.clone());
            }
        }
    }
}

fn resolve_directory(cli: &Cli) -> Result<PathBuf> {
    let target_directory = if let Some(dir) = &cli.directory {
        dir.clone()
    } else {
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };

    let canonical_path = target_directory.canonicalize().map_err(|_| {
        anyhow::anyhow!(
            "Error: Directory '{}' does not exist or cannot be accessed",
            target_directory.display()
        )
    })?;

    if !canonical_path.is_dir() {
        return Err(anyhow::anyhow!(
            "Error: '{}' is not a directory",
            canonical_path.display()
        ));
    }

    Ok(canonical_path)
}
