mod claude_config;
mod codex;
mod config;
mod discovery;
mod import;
mod pi;
mod platform;
mod proxy;
mod sync;
mod tui;
mod uninstall;
#[cfg(windows)]
mod windows;

use std::{path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use semver::Version;

use crate::config::AppPaths;

const MIN_CLAUDE_VERSION: &str = "2.1.242";

#[derive(Parser)]
#[command(
    name = "ccsw",
    version,
    about = "Manage Claude Code providers, models, and proxy settings; Codex APIs and subscription accounts; Pi Agent API configuration"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage Pi Agent native API providers and models
    Pi {
        #[command(subcommand)]
        command: pi::Command,
    },
    /// Manage Codex CLI and desktop API settings and subscription accounts
    Codex {
        #[command(subcommand)]
        command: codex::Command,
    },
    /// Remove this user's CCSW configuration and startup entry (binary retained)
    Uninstall {
        /// Execute the displayed cleanup plan
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,
        /// Preview cleanup without changing files (the default)
        #[arg(long)]
        dry_run: bool,
    },
    /// Diagnose Claude, configuration, and gateway connectivity
    Doctor,
    /// Persist a profile and its enabled models to Claude's global settings
    Apply {
        #[arg(long)]
        profile: String,
    },
    /// Manage the local OpenAI-compatible protocol proxy
    Proxy {
        #[command(subcommand)]
        command: ProxyCommand,
    },
    /// Show configuration information
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Import an existing ~/.claude/settings.json profile
    Import {
        /// Save the import without another confirmation
        #[arg(long)]
        yes: bool,
    },
    #[command(hide = true)]
    Internal {
        #[command(subcommand)]
        command: InternalCommand,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the active config path
    Path,
}

#[derive(Subcommand)]
enum ProxyCommand {
    /// Set this user's local proxy port (stop the proxy first)
    Port { port: u16 },
    /// Start the local proxy in the background
    Start {
        #[arg(long)]
        listen: Option<String>,
    },
    /// Show local proxy status
    Status,
    /// Stop the local proxy
    Stop,
    /// Enable the proxy at user login
    Install,
    /// Disable the proxy at user login
    Uninstall,
}

#[derive(Subcommand)]
enum InternalCommand {
    #[cfg(windows)]
    ProxyStart {
        #[arg(long)]
        registry: PathBuf,
    },
    ProxyServe {
        #[arg(long)]
        registry: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::discover()?;
    if let Some(Commands::Uninstall { yes, .. }) = &cli.command {
        return uninstall::run(&paths, *yes);
    }
    let _session = if matches!(
        cli.command,
        Some(Commands::Internal {
            command: InternalCommand::ProxyServe { .. }
        })
    ) {
        None
    } else {
        Some(uninstall::session(&paths)?)
    };
    match cli.command {
        Some(Commands::Uninstall { .. }) => unreachable!(),
        None => {
            let config = config::load(&paths.config)?;
            let import = if config.profiles.is_empty() {
                import::detect().unwrap_or(None)
            } else {
                None
            };
            tui::run(paths, config, import)
        }
        Some(Commands::Doctor) => doctor(&paths),
        Some(Commands::Pi { command }) => pi::run(&paths, command),
        Some(Commands::Codex { command }) => codex::run(&paths, command),
        Some(Commands::Apply { profile }) => apply_to_claude(&paths, &profile),
        Some(Commands::Proxy { command }) => proxy_command(&paths, command),
        Some(Commands::Config {
            command: ConfigCommand::Path,
        }) => {
            println!("{}", paths.config.display());
            Ok(())
        }
        Some(Commands::Import { yes }) => import_existing(&paths, yes),
        #[cfg(windows)]
        Some(Commands::Internal {
            command: InternalCommand::ProxyStart { registry },
        }) => {
            let mut paths = paths;
            paths.state_dir = registry
                .parent()
                .context("registry has no parent")?
                .to_path_buf();
            proxy::start(&paths, None).map(|_| ())
        }
        Some(Commands::Internal {
            command: InternalCommand::ProxyServe { registry },
        }) => tokio::runtime::Runtime::new()?.block_on(proxy::serve(registry)),
    }
}

fn proxy_command(paths: &AppPaths, command: ProxyCommand) -> Result<()> {
    match command {
        ProxyCommand::Port { port } => {
            let status = proxy::set_port(paths, port)?;
            println!(
                "Saved proxy listen address: {}. Start the proxy, then sync with p or ccsw apply --profile <id>.",
                status.listen
            );
        }
        ProxyCommand::Start { listen } => {
            let status = proxy::start(paths, listen.as_deref())?;
            println!(
                "CCSW proxy running at {} ({} routes)",
                status.listen, status.routes
            );
        }
        ProxyCommand::Status => {
            let status = proxy::status(paths)?;
            println!(
                "{} at {} · {} routes{}",
                if status.running { "running" } else { "stopped" },
                status.listen,
                status.routes,
                status
                    .pid
                    .map(|pid| format!(" · pid {pid}"))
                    .unwrap_or_default()
            );
        }
        ProxyCommand::Stop => {
            proxy::stop(paths)?;
            println!("CCSW proxy stopped; models synced to Claude require it to run.");
        }
        ProxyCommand::Install => {
            let path = proxy::install(paths)?;
            println!("Installed CCSW proxy user service at {}", path.display());
        }
        ProxyCommand::Uninstall => match proxy::uninstall()? {
            Some(path) => println!("Removed CCSW proxy user service at {}", path.display()),
            None => println!("CCSW proxy user service was not installed."),
        },
    }
    Ok(())
}

fn apply_to_claude(paths: &AppPaths, profile_id: &str) -> Result<()> {
    let config = config::load(&paths.config)?;
    let profile = config
        .profiles
        .get(profile_id)
        .with_context(|| format!("profile '{profile_id}' does not exist"))?;
    if !profile.enabled {
        bail!("profile '{profile_id}' is disabled");
    }
    let settings = claude_config::settings_path()?;
    let result = sync::apply(paths, &settings, Some(profile_id), true)?;
    println!(
        "Synced {} models from all profiles to {} (default profile: '{profile_id}')",
        result.model_count,
        result.path.display()
    );
    if let Some(backup) = result.backup {
        println!("Previous settings backed up to {}", backup.display());
    }
    Ok(())
}

fn import_existing(paths: &AppPaths, yes: bool) -> Result<()> {
    let Some(candidate) = import::detect()? else {
        println!("No importable ~/.claude/settings.json gateway profile was found.");
        return Ok(());
    };
    for line in candidate.summary() {
        println!("{line}");
    }
    if !yes {
        println!("\nPreview only. Run `ccsw import --yes` to save this profile.");
        return Ok(());
    }
    let config = config::load(&paths.config)?;
    let id = if config.profiles.contains_key("imported") {
        bail!("profile 'imported' already exists; rename or remove it first");
    } else {
        "imported".to_owned()
    };
    config::update(&paths.config, |latest| {
        latest.profiles.insert(id.clone(), candidate.profile);
        Ok(())
    })?;
    println!("Imported as profile '{id}'. The Claude settings file was not changed.");
    Ok(())
}

fn doctor(paths: &AppPaths) -> Result<()> {
    let mut failed = false;
    println!("CCSW doctor\n");
    let claude_bin = std::env::var_os("CCSW_CLAUDE_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("claude"));
    match Command::new(&claude_bin).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version_text = String::from_utf8_lossy(&output.stdout);
            let parsed = version_text.split_whitespace().find_map(|word| {
                Version::parse(word.trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '.')).ok()
            });
            match parsed {
                Some(version) if version >= Version::parse(MIN_CLAUDE_VERSION)? => {
                    println!("✓ Claude Code {version}");
                }
                Some(version) => {
                    failed = true;
                    println!("✗ Claude Code {version}; {MIN_CLAUDE_VERSION}+ is required");
                }
                None => println!("! Claude runs, but its version could not be parsed"),
            }
        }
        Ok(output) => {
            failed = true;
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("✗ Claude could not start: {}", stderr.trim());
        }
        Err(error) => {
            failed = true;
            println!("✗ Claude executable not found: {error}");
        }
    }

    match config::load(&paths.config) {
        Ok(config) => {
            let has_profiles = !config.profiles.is_empty();
            println!(
                "✓ Config: {} ({} profiles)",
                paths.config.display(),
                config.profiles.len()
            );
            #[cfg(unix)]
            if paths.config.exists() {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&paths.config)?.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    failed = true;
                    println!("✗ Config permissions are {mode:o}; expected 600");
                } else {
                    println!("✓ Config permissions are private");
                }
            }
            for (id, profile) in config.profiles {
                if !profile.enabled {
                    println!("○ {id}: provider disabled (network check skipped)");
                    continue;
                }
                match discovery::discover(&profile) {
                    Ok(models) => {
                        println!("✓ {id}: {} models from {}", models.len(), profile.base_url)
                    }
                    Err(error) => {
                        failed = true;
                        println!("✗ {id}: {error:#}");
                    }
                }
            }
            if has_profiles {
                match proxy::status(paths) {
                    Ok(status) if status.running => {
                        println!("✓ CCSW proxy: {}", status.listen)
                    }
                    Ok(status) => {
                        println!(
                            "! CCSW proxy is stopped; it will start when profiles are synced ({})",
                            status.listen
                        )
                    }
                    Err(error) => {
                        failed = true;
                        println!("✗ CCSW proxy: {error:#}");
                    }
                }
            }
        }
        Err(error) => {
            failed = true;
            println!("✗ Config: {error:#}");
        }
    }
    if failed {
        bail!("one or more checks failed");
    }
    println!("\nAll checks passed.");
    Ok(())
}
