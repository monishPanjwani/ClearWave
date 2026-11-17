#[cfg(any(target_os = "linux", not(feature = "gui")))]
use anyhow::bail;
#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;
use clap::{Parser, Subcommand};
pub use clearwave::{model, platform};
#[cfg(feature = "gui")]
mod ui;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "ClearWave: Linux playback enhancement and microphone cleanup"
)]
struct Args {
    /// Emit machine-readable JSON (stream commands emit one JSON object per line).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Open the desktop interface (also the default).
    Open,
    /// Run the per-user background engine (never starts capture automatically).
    Daemon,
    /// Apply saved login processing policy once per graphical session, without a window.
    Startup,
    /// Inspect or explicitly change XDG login startup (does not start audio).
    Autostart {
        #[command(subcommand)]
        action: AutostartAction,
    },
    /// Inspect or control the background engine.
    Engine {
        #[command(subcommand)]
        action: EngineAction,
    },
    #[cfg(target_os = "linux")]
    /// Check dependencies and audio-server connectivity without recording.
    Doctor,
    #[cfg(target_os = "linux")]
    /// List microphone and speaker IDs without recording.
    Devices,
    #[cfg(target_os = "linux")]
    /// Run cleanup in the foreground; stop with Ctrl+C or SIGTERM.
    Run {
        /// Source ID from `devices`; defaults to the current system microphone.
        #[arg(long)]
        input: Option<String>,
        /// Headphone sink ID. Required with --monitor.
        #[arg(long)]
        output: Option<String>,
        #[arg(long, default_value_t = 0.0)]
        boost_db: f32,
        #[arg(long)]
        no_denoise: bool,
        /// Listen to the cleaned mic through headphones (off by default).
        #[arg(long)]
        monitor: bool,
        /// Temporarily make Clearwave the default input; restore on normal shutdown.
        #[arg(long)]
        make_default: bool,
    },
    #[cfg(target_os = "linux")]
    /// Change a device's system volume (0–150%).
    Volume {
        #[arg(value_enum)]
        kind: model::DeviceKind,
        id: String,
        #[arg(value_parser = clap::value_parser!(u16).range(0..=150))]
        percent: u16,
    },
    #[cfg(target_os = "linux")]
    /// Explicitly change the system default device (persists after this command).
    Default {
        #[arg(value_enum)]
        kind: model::DeviceKind,
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum AutostartAction {
    Status,
    Enable,
    Disable,
    Repair,
}

#[derive(Debug, Subcommand)]
enum EngineAction {
    /// Apply a validated versioned JSON settings file (never starts capture).
    Configure {
        file: std::path::PathBuf,
    },
    Status,
    Stop {
        #[arg(long)]
        input: bool,
    },
    Start {
        #[arg(long)]
        input: bool,
    },
    StopAll,
    Shutdown,
}

fn main() {
    let json = std::env::args().any(|arg| arg == "--json");
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            if json && error.use_stderr() {
                eprintln!(
                    "{}",
                    serde_json::json!({"ok":false,"error":error.to_string()})
                );
                std::process::exit(2);
            }
            error.exit();
        }
    };
    let json = args.json;
    if let Err(error) = execute(args) {
        if json {
            eprintln!(
                "{}",
                serde_json::json!({"ok":false,"error":format!("{error:#}")})
            );
        } else {
            eprintln!("Clearwave: {error:#}");
        }
        std::process::exit(1);
    }
}

fn execute(args: Args) -> Result<()> {
    match args.command.unwrap_or(Command::Open) {
        Command::Autostart { action } => {
            match action {
                AutostartAction::Status => {}
                AutostartAction::Enable => clearwave::autostart::set(true)?,
                AutostartAction::Disable => clearwave::autostart::set(false)?,
                AutostartAction::Repair => clearwave::autostart::repair()?,
            }
            print_value(&clearwave::autostart::inspect(), args.json)
        }
        Command::Startup => {
            clearwave::engine::ensure_running()?;
            clearwave::engine::request(&clearwave::engine::Request::Startup)?;
            Ok(())
        }
        Command::Daemon => clearwave::engine::run_daemon(),
        Command::Engine { action } => {
            use clearwave::engine::{self, Request};
            let request = match action {
                EngineAction::Status => Request::Status,
                EngineAction::Configure { file } => Request::Configure {
                    config: Box::new(clearwave::settings::Config::load(&file)?),
                },
                EngineAction::Start { input } => Request::Start { input },
                EngineAction::Stop { input } => Request::Stop { input },
                EngineAction::StopAll => Request::StopAll,
                EngineAction::Shutdown => Request::Shutdown,
            };
            print_value(&engine::request(&request)?, args.json)
        }
        Command::Open => {
            #[cfg(feature = "gui")]
            {
                ui::run()
            }
            #[cfg(not(feature = "gui"))]
            {
                bail!("This executable was built without the GUI. Use `run` or rebuild with default features.")
            }
        }
        #[cfg(target_os = "linux")]
        Command::Doctor => {
            let report = platform::doctor();
            print_value(&report, args.json)?;
            if report.get("ok").and_then(|x| x.as_bool()) == Some(false) {
                bail!("Audio setup is incomplete; see the diagnostic report.");
            }
            Ok(())
        }
        #[cfg(target_os = "linux")]
        Command::Devices => print_value(&platform::enumerate()?, args.json),
        #[cfg(target_os = "linux")]
        Command::Volume { kind, id, percent } => {
            platform::set_volume(&id, kind, percent)?;
            print_value(
                &serde_json::json!({"ok":true,"id":id,"percent":percent}),
                args.json,
            )
        }
        #[cfg(target_os = "linux")]
        Command::Default { kind, id } => {
            platform::set_default(&id, kind)?;
            print_value(
                &serde_json::json!({"ok":true,"id":id,"kind":kind}),
                args.json,
            )
        }
        #[cfg(target_os = "linux")]
        Command::Run {
            input,
            output,
            boost_db,
            no_denoise,
            monitor,
            make_default,
        } => {
            use std::sync::{
                atomic::{AtomicBool, Ordering},
                Arc, Mutex,
            };
            let inventory = platform::enumerate()?;
            let input = input
                .or_else(|| {
                    inventory
                        .devices
                        .iter()
                        .find(|d| d.kind == model::DeviceKind::Input && d.is_default)
                        .map(|d| d.id.clone())
                })
                .context(
                    "No default microphone found. Choose a source from `devices` with --input ID.",
                )?;
            let settings = model::Settings {
                input,
                output,
                boost_db,
                denoise: !no_denoise,
                monitor,
            };
            settings.validate()?;
            let stop = Arc::new(AtomicBool::new(false));
            let signal = stop.clone();
            ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))?;
            let telemetry = Arc::new(Mutex::new(model::Telemetry::default()));
            let mut session = platform::Session::start(settings.clone(), telemetry.clone())?;
            if make_default {
                session.make_default()?;
            }
            print_value(
                &serde_json::json!({"event":"started","ok":true,
                "virtual_source":session.virtual_source(),"settings":settings}),
                args.json,
            )?;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let state = telemetry
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Audio state unavailable"))?
                    .clone();
                if let Some(error) = &state.error {
                    bail!("{error}");
                }
                if !state.running {
                    bail!("Audio processing stopped unexpectedly.");
                }
                if args.json {
                    print_value(&serde_json::json!({"event":"levels","state":state}), true)?;
                }
            }
            session.stop();
            print_value(&serde_json::json!({"event":"stopped","ok":true}), args.json)
        }
    }
}

#[cfg(target_os = "linux")]
fn print_value(value: &impl serde::Serialize, json: bool) -> Result<()> {
    use std::io::Write;
    let text = if json {
        serde_json::to_string(value)?
    } else {
        serde_json::to_string_pretty(value)?
    };
    let mut out = std::io::stdout().lock();
    writeln!(out, "{text}")?;
    out.flush()?;
    Ok(())
}
