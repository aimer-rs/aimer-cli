use std::env::{set_current_dir, var_os};
use std::ffi::OsStr;
use std::path::Path;

use anyhow::Context;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;

use crate::commands::version::VersionCommand;
use crate::targets::{MigrateTarget, Targets};

pub mod commands;
pub mod console;
mod consts;
pub mod errors;
pub mod resource_monitor;
pub mod session;
pub mod targets;
pub mod tui;

pub use aimer_config as config;

#[derive(Parser)]
#[command(name = "aimer")]
#[command(about = "Aimer Framework CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Show the version of the CLI
    #[arg(short = 'v', long = "version")]
    version: bool,

    /// Enable verbose (debug) logging
    #[arg(long, global = true)]
    verbose: bool,
}

/// Initialise the tracing subscriber. Honours `RUST_LOG` if set, otherwise
/// defaults to `warn` (or `debug` when `--verbose` is passed).
///
/// Logs go to stderr: stdout carries command output and, for `aimer mcp`, the
/// JSON-RPC stream, which any log line would corrupt.
fn init_logging(verbose: bool) {
    use tracing_subscriber::{EnvFilter, fmt};

    let default_level = if verbose { "debug" } else { "warn" };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    let _ = fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

#[cfg(debug_assertions)]
fn apply_project_dir_override(project_dir: Option<&OsStr>) -> anyhow::Result<()> {
    match project_dir {
        Some(dir) => set_current_dir(Path::new(dir))
            .with_context(|| format!("failed to set current dir to '{}'", dir.to_string_lossy())),
        None => {
            tracing::debug!("MY_PROJECT_DIR is not set");
            Ok(())
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new project
    Create {
        /// Name of the project
        project_name: String,
    },

    /// Turn an existing Cargo project into an Aimer project
    Init {
        /// Cargo project directory (defaults to the current directory)
        path: Option<std::path::PathBuf>,
    },

    /// Read or search crate documentation from docs.rs or installed Rust toolchain docs
    Docs {
        /// Cargo package name or toolchain crate, for example `serde` or `std`
        crate_name: String,
        /// Item path relative to the crate root, for example `de::Deserialize`
        #[arg(conflicts_with = "search")]
        item_path: Option<String>,
        /// Search public item names, paths, and documentation text
        #[arg(long, conflicts_with = "item_path")]
        search: Option<String>,
        /// Exact crate version (defaults to Cargo.lock), or Rust toolchain for std/core/alloc
        #[arg(long)]
        version: Option<String>,
        /// Maximum search results to return (1–100; defaults to 20)
        #[arg(long, requires = "search")]
        limit: Option<usize>,
        /// Refresh the cached docs.rs crate index
        #[arg(long)]
        refresh: bool,
    },

    /// Run the project (interactive picker, or scriptable with
    /// --target/--device)
    Run {
        /// Build/run for this target without showing the picker
        #[arg(short, long, value_enum)]
        target: Option<Targets>,
        /// Run on the device with this id without showing the picker
        #[arg(short, long)]
        device: Option<String>,
        /// Select an Aimer package in the Cargo workspace.
        #[arg(short, long)]
        package: Option<String>,
        /// Print plain logs to stdout/stderr instead of the interactive console.
        /// Useful when running from an IDE or CI where no terminal is
        /// available.
        #[arg(long)]
        no_tui: bool,
        /// Use the full-screen TUI console instead of the default inline one.
        #[arg(long, conflicts_with = "no_tui")]
        tui: bool,
        /// Build, package and launch with the release profile
        #[arg(short, long)]
        release: bool,
        /// Do not start or register the private MCP control session.
        #[arg(long)]
        no_mcp: bool,
    },

    /// Expose a running Aimer session as an MCP server over stdio.
    #[cfg(feature = "mcp")]
    #[command(group(
        clap::ArgGroup::new("config_target")
            .args(["install", "json", "toml"])
            .multiple(true)
    ))]
    Mcp {
        /// Install the Aimer MCP server for one or more supported agents.
        #[arg(
            long,
            value_name = "AGENT",
            num_args = 1..,
            value_delimiter = ',',
            conflicts_with_all = ["attach", "json", "toml"]
        )]
        install: Option<Vec<String>>,
        /// Print the MCP server entry as JSON (Claude Code / Cursor format)
        /// without installing it.
        #[arg(long, conflicts_with_all = ["attach", "toml"])]
        json: bool,
        /// Print the MCP server entry as TOML (Codex format) without
        /// installing it.
        #[arg(long, conflicts_with_all = ["attach", "json"])]
        toml: bool,
        /// Use the user's global agent configuration instead of binding the
        /// entry to the current project (with --install, --json or --toml).
        #[arg(long, requires = "config_target", conflicts_with_all = ["project", "attach"])]
        global: bool,
        /// Project path used for automatic session discovery.
        #[arg(long, conflicts_with = "attach")]
        project: Option<std::path::PathBuf>,
        /// Attach to this exact session ID.
        #[arg(long, conflicts_with = "project")]
        attach: Option<String>,
        /// Return full, unabridged tool output instead of the compact form
        /// (same as AIMER_MCP_FULL_OUTPUT=1).
        #[arg(long, conflicts_with_all = ["install", "json", "toml"])]
        full_output: bool,
        /// Serve this client in its own process instead of sharing one
        /// analysis engine with other agents on the same workspace.
        #[arg(long, conflicts_with_all = ["install", "json", "toml", "host"])]
        standalone: bool,
        /// Run as the shared workspace host (started automatically).
        #[arg(long, hide = true, conflicts_with_all = ["install", "json", "toml", "attach"])]
        host: bool,
    },

    /// Install Aimer's Rust workflow skill for one or more coding agents.
    Skill {
        /// Install the user-level skill for codex, claude, or cursor.
        #[arg(
            long,
            value_name = "AGENT",
            num_args = 1..,
            value_delimiter = ',',
            required = true
        )]
        install: Vec<String>,
    },

    /// Send a click, key or gesture to the app a run session launched.
    Input(commands::input::InputArgs),

    /// Take a picture of the window of the app a run session launched.
    Screenshot(commands::screenshot::ScreenshotArgs),

    /// List live Aimer run sessions.
    Sessions {
        /// Restrict discovery to a project path.
        #[arg(long)]
        project: Option<std::path::PathBuf>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },

    /// Build the project for a target without launching it
    Build {
        /// Target to build for (defaults to aimer.toml's default_target)
        #[arg(short, long, value_enum)]
        target: Option<Targets>,
        /// Select an Aimer package in the Cargo workspace.
        #[arg(short, long)]
        package: Option<String>,
        /// Build in release mode
        #[arg(short, long)]
        release: bool,
    },

    /// Assemble the distributable platform bundle (.app, .apk, .ipa, ...)
    Assemble {
        /// Target platform to bundle for (macos, android, ios, web, ...)
        #[arg(value_enum)]
        platform: Targets,
        /// Select an Aimer package in the Cargo workspace.
        #[arg(short, long)]
        package: Option<String>,
        /// Assemble in release mode
        #[arg(short, long)]
        release: bool,
    },

    /// Package the app: .dmg/.pkg/.tar.gz/.zip (macOS), .ipa (iOS), .aab/.apk (Android),
    /// .AppImage/.tar.gz/.zip (Linux), .exe/.msi/.zip (Windows)
    Package {
        /// Target platform to package for (macos, ios, android, linux or windows)
        #[arg(value_enum)]
        platform: Targets,
        /// Package format (default: dmg on macOS, ipa on iOS, aab on Android, appimage on
        /// Linux, exe on Windows)
        #[arg(short, long, value_enum)]
        format: Option<commands::package::PackageFormat>,
        /// Select an Aimer package in the Cargo workspace.
        #[arg(short, long)]
        package: Option<String>,
        /// Package a release build (the default; kept for explicitness)
        #[arg(short, long, conflicts_with = "debug")]
        release: bool,
        /// Package a debug build instead of the default release build
        #[arg(long)]
        debug: bool,
        /// macOS: code-sign the app and the package (see [macos.signing] in Aimer.toml)
        #[arg(long)]
        sign: bool,
        /// macOS: sign, then have Apple notarize the package and staple the ticket
        #[arg(long)]
        notarize: bool,
        /// iOS: upload the build to App Store Connect (see [ios.signing] in Aimer.toml)
        #[arg(long)]
        upload: bool,
    },

    /// Check that the required toolchains are installed
    Doctor,

    /// Remove build artifacts (builds/ and target/)
    Clean,

    /// Generate a shell completion script
    Completions {
        /// Shell to generate completions for (bash, zsh, fish, ...)
        shell: clap_complete::Shell,
        /// Install the script into the shell's completion directory instead of
        /// printing it to stdout
        #[arg(long)]
        install: bool,
    },

    /// Migrate platform build scaffolds to the latest version
    Migrate {
        /// Target to migrate (macos, windows, linux, android, ios, web, all)
        #[arg(value_enum)]
        target: MigrateTarget,
    },

    /// Manage the tools Aimer installs into ~/.aimer/bin (lists them by default)
    Components {
        #[command(subcommand)]
        action: Option<ComponentsAction>,
    },

    /// Update aimer to the latest release from GitHub
    Update {
        /// Only check whether a newer release exists
        #[arg(long)]
        check: bool,

        /// Install this release (for example `0.9.1`) instead of the latest
        #[arg(long = "to", value_name = "VERSION")]
        version: Option<String>,

        /// Reinstall even when the version is already installed
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum ComponentsAction {
    /// List the available components and whether they are installed
    List,

    /// Install one or more components into ~/.aimer
    Add {
        /// Components to install, for example `trunk` or `ndk` (`ndk@27` picks a version)
        #[arg(
            required = true,
            num_args = 1..,
            add = ArgValueCompleter::new(commands::components::complete_available)
        )]
        names: Vec<String>,

        /// Accept the licences a component requires (such as the Android SDK
        /// licence for the NDK) without asking
        #[arg(long)]
        accept_licenses: bool,
    },

    /// Remove one or more installed components
    Remove {
        /// Components to remove, for example `trunk`
        #[arg(
            required = true,
            num_args = 1..,
            add = ArgValueCompleter::new(commands::components::complete_installed)
        )]
        names: Vec<String>,
    },
}

fn main() -> anyhow::Result<()> {
    // Dynamic, self-updating shell completions. When the shell invokes the
    // binary with `COMPLETE=<shell>` set, this generates completions from the
    // *current* command tree (so newly added subcommands appear automatically)
    // and exits. Must run before anything writes to stdout.
    clap_complete::env::CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();
    init_logging(cli.verbose);

    #[cfg(debug_assertions)]
    apply_project_dir_override(var_os("MY_PROJECT_DIR").as_deref())?;

    commands::components::expose_on_path();

    if cli.version {
        VersionCommand::execute();
        return Ok(());
    }

    match &cli.command {
        Some(Commands::Create { project_name }) => {
            commands::create::execute(project_name)?;
        }
        Some(Commands::Init { path }) => {
            commands::init::execute(path.as_deref())?;
        }
        Some(Commands::Docs {
            crate_name,
            item_path,
            search,
            version,
            limit,
            refresh,
        }) => {
            commands::docs::execute(
                crate_name.clone(),
                item_path.clone(),
                search.clone(),
                version.clone(),
                *limit,
                *refresh,
            )?;
        }
        Some(Commands::Run {
            target,
            device,
            package,
            no_tui,
            tui,
            release,
            no_mcp,
        }) => {
            commands::run::execute_with_package(
                target.map(|t| t.to_string()),
                device.clone(),
                package.clone(),
                *no_tui,
                *release,
                *tui,
                *no_mcp,
            )?;
        }
        #[cfg(feature = "mcp")]
        Some(Commands::Mcp {
            project,
            attach,
            install,
            json,
            toml,
            global,
            full_output,
            standalone,
            host,
        }) => {
            let scope = || -> anyhow::Result<commands::mcp_install::InstallScope> {
                Ok(if *global {
                    commands::mcp_install::InstallScope::Global
                } else {
                    commands::mcp_install::InstallScope::Project(
                        project.clone().unwrap_or(std::env::current_dir()?),
                    )
                })
            };
            if *json || *toml {
                let format = if *json {
                    commands::mcp_install::ConfigFormat::Json
                } else {
                    commands::mcp_install::ConfigFormat::Toml
                };
                println!(
                    "{}",
                    commands::mcp_install::render_config(format, scope()?)?
                );
            } else if let Some(agents) = install {
                let scope = scope()?;
                let reports = commands::mcp_install::install(agents, scope)?;
                for report in reports {
                    let state = if report.changed {
                        "installed"
                    } else {
                        "already installed"
                    };
                    println!(
                        "Aimer MCP {state} for {} in {}",
                        report.agent,
                        report.path.display()
                    );
                }
            } else {
                let mode = if *host {
                    commands::mcp::ServeMode::Host
                } else if *standalone {
                    commands::mcp::ServeMode::Standalone
                } else {
                    commands::mcp::ServeMode::Shared
                };
                commands::mcp::execute(project.clone(), attach.clone(), *full_output, mode)?;
            }
        }
        Some(Commands::Skill { install }) => {
            let reports = commands::skill_install::install(install)?;
            for report in reports {
                let state = if report.changed {
                    "installed"
                } else {
                    "already installed"
                };
                println!(
                    "Aimer Rust skill {state} for {} in {}",
                    report.agent,
                    report.path.display()
                );
            }
        }
        Some(Commands::Input(args)) => commands::input::execute(args)?,
        Some(Commands::Screenshot(args)) => commands::screenshot::execute(args)?,
        Some(Commands::Sessions { project, json }) => {
            commands::sessions::list_sessions(project.clone(), *json)?;
        }
        Some(Commands::Build {
            target,
            package,
            release,
        }) => {
            commands::build::execute_with_package(
                target.map(|t| t.to_string()),
                *release,
                package.clone(),
            )?;
        }
        Some(Commands::Assemble {
            platform,
            package,
            release,
        }) => {
            commands::assemble::execute_with_package(
                platform.to_string(),
                *release,
                package.clone(),
            )?;
        }
        Some(Commands::Package {
            platform,
            format,
            package,
            release: _,
            debug,
            sign,
            notarize,
            upload,
        }) => {
            commands::package::execute_with_package(commands::package::PackageOptions {
                platform: platform.to_string(),
                format: *format,
                // Packaging is for distribution, so it builds release unless
                // `--debug` asks otherwise; `--release` just states the default.
                release: !*debug,
                package: package.clone(),
                sign: *sign,
                notarize: *notarize,
                upload: *upload,
            })?;
        }
        Some(Commands::Doctor) => {
            commands::doctor::execute()?;
        }
        Some(Commands::Clean) => {
            commands::clean::execute()?;
        }
        Some(Commands::Update {
            check,
            version,
            force,
        }) => {
            commands::update::execute(&commands::update::Options {
                check: *check,
                version: version.clone(),
                force: *force,
            })?;
        }
        Some(Commands::Components { action }) => {
            use commands::components::Action;
            commands::components::execute(match action {
                None | Some(ComponentsAction::List) => Action::List,
                Some(ComponentsAction::Add {
                    names,
                    accept_licenses,
                }) => Action::Add {
                    names: names.clone(),
                    accept_licenses: *accept_licenses,
                },
                Some(ComponentsAction::Remove { names }) => Action::Remove(names.clone()),
            })?;
        }
        Some(Commands::Completions { shell, install }) => {
            commands::completions::execute(*shell, *install)?;
        }
        Some(Commands::Migrate { target }) => {
            commands::migrate::execute(target.as_str().to_string())?;
        }
        None => {
            Cli::parse_from(["aimer", "--help"]);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    #[cfg(debug_assertions)]
    use std::env::{current_dir, set_current_dir};

    use super::{Cli, Commands, ComponentsAction};

    #[cfg(debug_assertions)]
    #[test]
    fn runtime_project_dir_override_uses_the_launch_environment() {
        let original = current_dir().unwrap();
        let project = tempfile::tempdir().unwrap();

        super::apply_project_dir_override(Some(project.path().as_os_str())).unwrap();
        // Canonicalize both sides: on Windows `current_dir()` omits the `\\?\`
        // verbatim prefix that `Path::canonicalize()` adds, so comparing the raw
        // and canonical forms directly would mismatch there.
        assert_eq!(
            current_dir().unwrap().canonicalize().unwrap(),
            project.path().canonicalize().unwrap()
        );

        set_current_dir(original).unwrap();
    }

    #[test]
    fn ordinary_run_remains_accepted() {
        assert!(Cli::try_parse_from(["aimer", "run"]).is_ok());
    }

    #[test]
    fn nightly_selector_and_unstable_flag_are_no_longer_accepted() {
        assert!(Cli::try_parse_from(["aimer", "+nightly", "run"]).is_err());
        assert!(Cli::try_parse_from(["aimer", "run", "-Z", "hot-reload"]).is_err());
        assert!(Cli::try_parse_from(["aimer", "run", "--verbose-widget-ir"]).is_err());
    }

    #[test]
    fn run_accepts_the_release_flag() {
        let cli = Cli::try_parse_from(["aimer", "run", "--release"]).unwrap();

        assert!(matches!(
            cli.command,
            Some(Commands::Run { release: true, .. })
        ));
    }

    #[test]
    fn run_build_and_assemble_accept_a_package_selector() {
        let run = Cli::try_parse_from(["aimer", "run", "-p", "moewmoew"]).unwrap();
        assert!(matches!(
            run.command,
            Some(Commands::Run {
                package: Some(package),
                ..
            }) if package == "moewmoew"
        ));

        let build =
            Cli::try_parse_from(["aimer", "build", "--package", "moewmoew", "--target", "web"])
                .unwrap();
        assert!(matches!(
            build.command,
            Some(Commands::Build {
                package: Some(package),
                ..
            }) if package == "moewmoew"
        ));

        let assemble =
            Cli::try_parse_from(["aimer", "assemble", "android", "-p", "moewmoew"]).unwrap();
        assert!(matches!(
            assemble.command,
            Some(Commands::Assemble {
                package: Some(package),
                ..
            }) if package == "moewmoew"
        ));
    }

    #[test]
    fn package_defaults_to_no_explicit_format_and_accepts_short_flags() {
        use crate::commands::package::PackageFormat;

        let default = Cli::try_parse_from(["aimer", "package", "macos"]).unwrap();
        assert!(matches!(
            default.command,
            Some(Commands::Package {
                format: None,
                release: false,
                debug: false,
                ..
            })
        ));

        let debug = Cli::try_parse_from(["aimer", "package", "macos", "--debug"]).unwrap();
        assert!(matches!(
            debug.command,
            Some(Commands::Package { debug: true, .. })
        ));
        assert!(Cli::try_parse_from(["aimer", "package", "macos", "--debug", "-r"]).is_err());

        let pkg =
            Cli::try_parse_from(["aimer", "package", "macos", "-f", "pkg", "-r", "-p", "app"])
                .unwrap();
        assert!(matches!(
            pkg.command,
            Some(Commands::Package {
                format: Some(PackageFormat::Pkg),
                release: true,
                package: Some(package),
                ..
            }) if package == "app"
        ));

        let dmg = Cli::try_parse_from(["aimer", "package", "macos", "--format", "dmg"]).unwrap();
        assert!(matches!(
            dmg.command,
            Some(Commands::Package {
                format: Some(PackageFormat::Dmg),
                ..
            })
        ));

        // Every format is a valid value; whether it suits the platform is
        // decided later, so only a name that is not a format fails to parse.
        assert!(Cli::try_parse_from(["aimer", "package", "macos", "-f", "rar"]).is_err());
        assert!(Cli::try_parse_from(["aimer", "package", "macos", "-f", "zip"]).is_ok());

        let ios = Cli::try_parse_from(["aimer", "package", "ios", "--upload"]).unwrap();
        assert!(matches!(
            ios.command,
            Some(Commands::Package {
                format: None,
                upload: true,
                ..
            })
        ));
        let aab = Cli::try_parse_from(["aimer", "package", "android", "-f", "apk"]).unwrap();
        assert!(matches!(
            aab.command,
            Some(Commands::Package {
                format: Some(PackageFormat::Apk),
                ..
            })
        ));
        for (value, expected) in [
            ("appimage", PackageFormat::Appimage),
            ("tar.gz", PackageFormat::TarGz),
            ("tar-gz", PackageFormat::TarGz),
            ("zip", PackageFormat::Zip),
        ] {
            let linux = Cli::try_parse_from(["aimer", "package", "linux", "-f", value]).unwrap();
            assert!(
                matches!(linux.command, Some(Commands::Package { format: Some(f), .. }) if f == expected),
                "{value}"
            );
        }
        for (value, expected) in [
            ("exe", PackageFormat::Exe),
            ("msi", PackageFormat::Msi),
            ("zip", PackageFormat::Zip),
        ] {
            let windows =
                Cli::try_parse_from(["aimer", "package", "windows", "-f", value]).unwrap();
            assert!(
                matches!(windows.command, Some(Commands::Package { format: Some(f), .. }) if f == expected),
                "{value}"
            );
        }
        let ipa = Cli::try_parse_from(["aimer", "package", "ios", "-f", "ipa"]).unwrap();
        assert!(matches!(
            ipa.command,
            Some(Commands::Package {
                format: Some(PackageFormat::Ipa),
                ..
            })
        ));

        let signed =
            Cli::try_parse_from(["aimer", "package", "macos", "--sign", "--notarize"]).unwrap();
        assert!(matches!(
            signed.command,
            Some(Commands::Package {
                sign: true,
                notarize: true,
                ..
            })
        ));
        assert!(matches!(
            default.command,
            Some(Commands::Package {
                sign: false,
                notarize: false,
                ..
            })
        ));
    }

    #[test]
    fn run_can_disable_automatic_mcp_registration() {
        let cli = Cli::try_parse_from(["aimer", "run", "--no-mcp"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Run { no_mcp: true, .. })
        ));
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_project_and_exact_attach_are_mutually_exclusive() {
        assert!(
            Cli::try_parse_from(["aimer", "mcp", "--project", ".", "--attach", "abc"]).is_err()
        );
        assert!(Cli::try_parse_from(["aimer", "mcp", "--attach", "abc"]).is_ok());
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_global_install_accepts_multiple_agents() {
        let cli = Cli::try_parse_from(["aimer", "mcp", "--install", "codex", "claude", "--global"])
            .unwrap();

        assert!(matches!(
            cli.command,
            Some(Commands::Mcp {
                install: Some(agents),
                global: true,
                ..
            }) if agents == vec!["codex", "claude"]
        ));
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_standalone_and_host_flags_parse_and_conflict() {
        assert!(matches!(
            Cli::try_parse_from(["aimer", "mcp", "--standalone"])
                .unwrap()
                .command,
            Some(Commands::Mcp {
                standalone: true,
                host: false,
                ..
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["aimer", "mcp", "--host", "--project", "."])
                .unwrap()
                .command,
            Some(Commands::Mcp {
                host: true,
                standalone: false,
                ..
            })
        ));
        assert!(Cli::try_parse_from(["aimer", "mcp", "--host", "--standalone"]).is_err());
        assert!(Cli::try_parse_from(["aimer", "mcp", "--host", "--attach", "abc"]).is_err());
        assert!(Cli::try_parse_from(["aimer", "mcp", "--standalone", "--json"]).is_err());
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_global_flag_requires_install() {
        assert!(Cli::try_parse_from(["aimer", "mcp", "--global"]).is_err());
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_json_and_toml_print_without_installing() {
        let cli = Cli::try_parse_from(["aimer", "mcp", "--json", "--global"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Mcp {
                json: true,
                toml: false,
                install: None,
                global: true,
                ..
            })
        ));
        assert!(Cli::try_parse_from(["aimer", "mcp", "--toml"]).is_ok());
        assert!(Cli::try_parse_from(["aimer", "mcp", "--json", "--toml"]).is_err());
        assert!(Cli::try_parse_from(["aimer", "mcp", "--json", "--install", "codex"]).is_err());
        assert!(Cli::try_parse_from(["aimer", "mcp", "--toml", "--attach", "abc"]).is_err());
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_install_rejects_global_project_and_exact_attach_combinations() {
        assert!(
            Cli::try_parse_from([
                "aimer",
                "mcp",
                "--install",
                "codex",
                "--global",
                "--project",
                "."
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from(["aimer", "mcp", "--install", "codex", "--attach", "abc"]).is_err()
        );
    }

    #[test]
    fn run_defaults_to_the_inline_console() {
        let cli = Cli::try_parse_from(["aimer", "run"]).unwrap();

        assert!(matches!(
            cli.command,
            Some(Commands::Run {
                tui: false,
                no_tui: false,
                ..
            })
        ));
    }

    #[test]
    fn tui_flag_selects_the_full_screen_console() {
        let cli = Cli::try_parse_from(["aimer", "run", "--tui"]).unwrap();

        assert!(matches!(cli.command, Some(Commands::Run { tui: true, .. })));
    }

    #[test]
    fn update_takes_check_target_version_and_force() {
        let cli = Cli::try_parse_from(["aimer", "update"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Update {
                check: false,
                version: None,
                force: false
            })
        ));
        let cli =
            Cli::try_parse_from(["aimer", "update", "--check", "--to", "0.9.1", "--force"])
                .unwrap();
        let Some(Commands::Update {
            check,
            version,
            force,
        }) = cli.command
        else {
            panic!("expected update");
        };
        assert!(check && force);
        assert_eq!(version.as_deref(), Some("0.9.1"));
    }

    #[test]
    fn components_defaults_to_listing() {
        let cli = Cli::try_parse_from(["aimer", "components"]).unwrap();

        assert!(matches!(
            cli.command,
            Some(Commands::Components { action: None })
        ));
    }

    #[test]
    fn components_add_and_remove_take_names() {
        let cli =
            Cli::try_parse_from(["aimer", "components", "add", "trunk", "wasm-pack"]).unwrap();
        let Some(Commands::Components {
            action:
                Some(ComponentsAction::Add {
                    names,
                    accept_licenses,
                }),
        }) = cli.command
        else {
            panic!("expected components add");
        };
        assert_eq!(names, ["trunk", "wasm-pack"]);
        assert!(!accept_licenses);

        let cli = Cli::try_parse_from(["aimer", "components", "add", "ndk", "--accept-licenses"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Components {
                action: Some(ComponentsAction::Add {
                    accept_licenses: true,
                    ..
                })
            })
        ));

        let cli = Cli::try_parse_from(["aimer", "components", "remove", "trunk"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Components {
                action: Some(ComponentsAction::Remove { .. })
            })
        ));
    }

    #[test]
    fn components_add_requires_a_name() {
        assert!(Cli::try_parse_from(["aimer", "components", "add"]).is_err());
    }

    #[test]
    fn tui_and_no_tui_are_mutually_exclusive() {
        assert!(Cli::try_parse_from(["aimer", "run", "--tui", "--no-tui"]).is_err());
    }
}
