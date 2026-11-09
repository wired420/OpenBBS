//! The OpenBBS server binary.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use openbbs_core::{VERSION, config::Config, db};

#[derive(Parser)]
#[command(name = "openbbs", version = VERSION, about = "An open source bulletin board system")]
struct Cli {
    /// Path to the config file.
    #[arg(short, long, global = true, default_value = "openbbs.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a default config file and create the database.
    Init {
        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,
    },
    /// Validate the config file and database, then exit.
    Check,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Init { force } => init(&cli.config, force),
        Command::Check => check(&cli.config),
    }
}

fn init(path: &Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "{} already exists; pass --force to overwrite it",
            path.display()
        );
    }
    let config = Config::default();
    config.save(path)?;
    for dir in [
        &config.paths.data,
        &config.paths.themes,
        &config.paths.files,
        &config.paths.doors,
    ] {
        std::fs::create_dir_all(dir)?;
    }
    db::open(&config.database_path())?;
    println!(
        "OpenBBS {VERSION}: wrote {} and created the database.",
        path.display()
    );
    println!("Edit the config, then run `openbbs check`.");
    Ok(())
}

fn check(path: &Path) -> Result<()> {
    let config = Config::load(path)?;
    let conn = db::open(&config.database_path())?;
    println!("OpenBBS {VERSION}");
    println!("  BBS name:  {}", config.bbs.name);
    println!("  SSH:       {}", config.ssh.listen);
    println!(
        "  Database:  {} (schema v{})",
        config.database_path().display(),
        db::schema_version(&conn)?
    );
    println!("Config and database look good.");
    Ok(())
}
