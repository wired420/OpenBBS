//! The OpenBBS server binary.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use openbbs::session::Bbs;
use openbbs::store::Store;
use openbbs::{ssh, theme};
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
    /// Start the BBS and listen for SSH callers.
    Run,
    /// Set a user's password (for when someone, even the sysop, is locked out).
    Passwd {
        /// The user's handle.
        handle: String,
    },
    /// Set a user's access level (90 or more is sysop).
    SetLevel {
        /// The user's handle.
        handle: String,
        /// New level, 0 to 99.
        level: i64,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    if let Err(e) = dispatch(cli) {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
    Ok(())
}

fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Init { force } => init(&cli.config, force),
        Command::Check => check(&cli.config),
        Command::Run => run(&cli.config),
        Command::Passwd { handle } => passwd(&cli.config, &handle),
        Command::SetLevel { handle, level } => set_level(&cli.config, &handle, level),
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
    let theme_files = theme::install_default(&config.theme_dir())?;
    let doors_toml = config.paths.doors.join("doors.toml");
    if !doors_toml.exists() {
        std::fs::write(&doors_toml, openbbs_core::doors::EXAMPLE_DOORS_TOML)?;
    }
    openbbs_core::files::sync(&db::open(&config.database_path())?, &config.paths.files)?;
    ssh::load_or_create_host_key(&config.ssh.host_key)?;
    println!(
        "OpenBBS {VERSION}: wrote {} and created the database.",
        path.display()
    );
    println!(
        "Installed the default theme in {} ({theme_files} files).",
        config.theme_dir().display()
    );
    println!("Edit the config, then start the board with `openbbs run`.");
    println!("The first person to sign up becomes the sysop.");
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

fn run(path: &Path) -> Result<()> {
    let config = Config::load(path)?;
    let store = Store::open(&config.database_path())?;
    {
        let conn = openbbs_core::db::open(&config.database_path())?;
        let report = openbbs_core::files::sync(&conn, &config.paths.files)?;
        if report != Default::default() {
            tracing::info!(?report, "file areas synced with disk");
        }
    }
    let key = ssh::load_or_create_host_key(&config.ssh.host_key)?;
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&config.ssh.listen)
            .await
            .with_context(|| format!("listening on {}", config.ssh.listen))?;
        tracing::info!(
            "OpenBBS {VERSION} \"{}\" listening for SSH callers on {}",
            config.bbs.name,
            config.ssh.listen
        );
        let bbs = Bbs::new(config, store);
        tokio::select! {
            result = ssh::serve(bbs, key, listener) => result,
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                Ok(())
            }
        }
    })
}

fn find_user(conn: &rusqlite::Connection, handle: &str) -> Result<openbbs_core::users::User> {
    openbbs_core::users::find_for_login(conn, handle)?
        .map(|(user, _)| user)
        .with_context(|| format!("there's no user named {handle}"))
}

fn passwd(path: &Path, handle: &str) -> Result<()> {
    use openbbs_core::users;
    let config = Config::load(path)?;
    let conn = db::open(&config.database_path())?;
    let user = find_user(&conn, handle)?;
    eprintln!(
        "New password for {} (it will show as you type):",
        user.handle
    );
    let mut password = String::new();
    std::io::stdin().read_line(&mut password)?;
    let password = password.trim();
    users::validate_password(password)?;
    users::set_password_hash(&conn, user.id, &users::hash_password(password)?)?;
    println!("Password changed for {}.", user.handle);
    Ok(())
}

fn set_level(path: &Path, handle: &str, level: i64) -> Result<()> {
    let config = Config::load(path)?;
    let conn = db::open(&config.database_path())?;
    let user = find_user(&conn, handle)?;
    openbbs_core::users::set_level(&conn, user.id, level)?;
    println!("{} is now access level {level}.", user.handle);
    Ok(())
}
