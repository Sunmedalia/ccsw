mod claude_config;
mod config;
mod discovery;
mod import;
mod proxy;
mod runner;
mod state;
mod tui;

use std::{ffi::OsString, path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use semver::Version;

use crate::{
    config::AppPaths,
    runner::{LaunchRequest, SessionMode},
    state::{ProjectState, SessionRecord},
};

const MIN_CLAUDE_VERSION: &str = "2.1.242";

#[derive(Parser)]
#[command(
    name = "ccsw",
    version,
    about = "Route Claude Code through isolated model profiles"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Arguments forwarded to Claude when launching from the TUI
    #[arg(last = true)]
    claude_args: Vec<OsString>,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch Claude directly with a named profile
    Run(RunArgs),
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

#[derive(Args)]
struct RunArgs {
    #[arg(long)]
    profile: String,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, conflicts_with = "resume")]
    new: bool,
    #[arg(long)]
    resume: Option<String>,
    #[arg(last = true)]
    claude_args: Vec<OsString>,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the active config path
    Path,
}

#[derive(Subcommand)]
enum ProxyCommand {
    /// Start the local proxy in the background
    Start {
        #[arg(long)]
        listen: Option<String>,
    },
    /// Show local proxy status
    Status,
    /// Stop the local proxy
    Stop,
    /// Install a launchd/systemd user service
    Install,
    /// Remove the launchd/systemd user service
    Uninstall,
}

#[derive(Subcommand)]
enum InternalCommand {
    CaptureSession {
        #[arg(long)]
        path: PathBuf,
    },
    ProxyServe {
        #[arg(long)]
        registry: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::discover()?;
    match cli.command {
        None => {
            let config = config::load(&paths.config)?;
            let import = if config.profiles.is_empty() {
                import::detect().unwrap_or(None)
            } else {
                None
            };
            tui::run(paths, config, cli.claude_args, import)
        }
        Some(Commands::Run(args)) => run_direct(paths, args),
        Some(Commands::Doctor) => doctor(&paths),
        Some(Commands::Apply { profile }) => apply_to_claude(&paths, &profile),
        Some(Commands::Proxy { command }) => proxy_command(&paths, command),
        Some(Commands::Config {
            command: ConfigCommand::Path,
        }) => {
            println!("{}", paths.config.display());
            Ok(())
        }
        Some(Commands::Import { yes }) => import_existing(&paths, yes),
        Some(Commands::Internal {
            command: InternalCommand::CaptureSession { path },
        }) => runner::capture_session(&path),
        Some(Commands::Internal {
            command: InternalCommand::ProxyServe { registry },
        }) => tokio::runtime::Runtime::new()?.block_on(proxy::serve(registry)),
    }
}

fn proxy_command(paths: &AppPaths, command: ProxyCommand) -> Result<()> {
    match command {
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
            println!("CCSW proxy stopped; applied OpenAI routes require it to run.");
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
    let cache = discovery::load_cache(&paths.cache);
    let discovered = cache
        .profiles
        .get(profile_id)
        .map(|cached| cached.models.as_slice())
        .unwrap_or_default();
    let models = discovery::active_models(profile, discovered);
    let settings = claude_config::settings_path()?;
    let routed = proxy::routed_profile(paths, profile_id, profile)?;
    let result = claude_config::apply(&settings, &routed, &models)?;
    println!(
        "Applied profile '{profile_id}' with {} models to {}",
        result.model_count,
        result.path.display()
    );
    if let Some(backup) = result.backup {
        println!("Previous settings backed up to {}", backup.display());
    }
    Ok(())
}

fn run_direct(paths: AppPaths, args: RunArgs) -> Result<()> {
    let config = config::load(&paths.config)?;
    let profile = config
        .profiles
        .get(&args.profile)
        .with_context(|| format!("profile '{}' does not exist", args.profile))?;
    let model_id = args.model.as_deref().unwrap_or(&profile.default_model);
    let cache = discovery::load_cache(&paths.cache);
    let discovered = cache
        .profiles
        .get(&args.profile)
        .map(|cached| cached.models.as_slice())
        .unwrap_or_default();
    let models = discovery::active_models(profile, discovered);
    if !models.iter().any(|model| model.id == model_id) {
        bail!(
            "model '{model_id}' is not configured for profile '{}'",
            args.profile
        );
    }
    let mode = args
        .resume
        .map(SessionMode::Resume)
        .unwrap_or(SessionMode::New);
    let routed = proxy::routed_profile(&paths, &args.profile, profile)?;
    let result = runner::launch(
        &paths,
        LaunchRequest {
            profile: &routed,
            model_id,
            models: &models,
            mode,
            forwarded_args: args.claude_args,
        },
    )?;
    let cwd = state::canonical_project(&std::env::current_dir()?);
    state::update(&paths.state, |state| {
        state.projects.insert(
            cwd.clone(),
            ProjectState {
                profile_id: args.profile.clone(),
                model_id: result.capture.model_id.clone(),
            },
        );
        state.sessions.insert(
            result.capture.session_id.clone(),
            SessionRecord {
                session_id: result.capture.session_id.clone(),
                cwd,
                profile_id: args.profile,
                model_id: result.capture.model_id.clone(),
                updated_at: state::now_epoch(),
            },
        );
    })?;
    if !result.status.success() {
        bail!("Claude exited with {}", result.status);
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
            let has_openai = config
                .profiles
                .values()
                .any(|profile| profile.api_format.is_openai());
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
            if has_openai {
                match proxy::status(paths) {
                    Ok(status) if status.running => {
                        println!("✓ OpenAI proxy: {}", status.listen)
                    }
                    Ok(status) => {
                        println!(
                            "! OpenAI proxy is stopped; it will start on Launch/Apply ({})",
                            status.listen
                        )
                    }
                    Err(error) => {
                        failed = true;
                        println!("✗ OpenAI proxy: {error:#}");
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
