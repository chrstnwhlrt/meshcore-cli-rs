//! meshcore-cli-rs: command line interface to `MeshCore` companion radios,
//! a port of the Python meshcore-cli.

mod app;
mod args;
mod commands;
mod connect;
mod display;
mod editor;
mod handlers;
mod help;
mod history;
mod interactive;
mod output;
mod paths;
mod repeater;
mod signals;
mod util;

use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use log::LevelFilter;

use crate::app::{App, Settings, to_value, vint, vstr};
use crate::output::{Logger, Sink};

/// How long the program waits for the connection to close and the handlers
/// to finish when it ends. Over BLE, `BlueZ` confirms a disconnect only once
/// the link is down, about 2 s later; the request is out long before and
/// `BlueZ` completes it alone.
const CLOSE_WAIT: Duration = Duration::from_millis(500);

fn main() -> ExitCode {
    Logger::init();
    let mut argv = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.into_string() {
            Ok(arg) => argv.push(arg),
            Err(arg) => {
                let arg = arg.display();
                return python_error(&format!("the argument '{arg}' is not UTF-8 text"));
            }
        }
    }
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => return python_error(&e.to_string()),
    };
    let code = runtime.block_on(run_until_done(argv));
    // a prompt still waiting for input after Ctrl-C must not keep the
    // program alive; the reference exits at once
    runtime.shutdown_background();
    code
}

/// Ends the program after a signal it does not see, as while it waits for a
/// terminal or a pipe to take its output.
async fn end_blocked(session: Arc<OnceLock<Arc<App>>>) {
    let number = signals::terminated().await;
    tokio::time::sleep(2 * CLOSE_WAIT).await;
    if let Some(app) = session.get() {
        let _ = tokio::time::timeout(CLOSE_WAIT, app.mc.disconnect()).await;
    }
    std::process::exit(128 + i32::from(number));
}

/// Runs the program until it is done or a signal ends it, then closes the
/// connection.
async fn run_until_done(argv: Vec<String>) -> ExitCode {
    let session: Arc<OnceLock<Arc<App>>> = Arc::default();
    tokio::spawn(end_blocked(Arc::clone(&session)));
    let mut signal = None;
    let code = tokio::select! {
        // a signal that came while the program did not wait counts first
        biased;
        number = signals::ending() => {
            signal = Some(number);
            ExitCode::SUCCESS
        }
        code = run(argv, &session) => code,
    };
    if signal == Some(signals::INTERRUPT) {
        // another signal ends the wait
        tokio::select! {
            () = interactive::processes_ended() => {}
            _ = signals::ending() => {}
        }
        if interactive::IN_LOOP.load(Ordering::Relaxed) {
            // the chat loop catches the cancellation in the reference
            output::println("Exiting cli");
        } else {
            output::println("\nExited cleanly");
        }
    }
    // closed however the program ends: BlueZ keeps a BLE link up after the
    // program is gone, and the device does not advertise while connected
    if let Some(app) = session.get() {
        let close = async { tokio::join!(app.handlers.close(), app.mc.disconnect()) };
        let _ = tokio::time::timeout(CLOSE_WAIT, close).await;
    } else if signal.is_some() {
        // a BLE link still being set up is closed by the library in the
        // background, which needs the runtime for a moment
        tokio::time::sleep(CLOSE_WAIT).await;
    }
    match (signal, output::stdout_error()) {
        // what a shell reports for a program killed by the signal, as the
        // reference is
        (Some(number), _) if number != signals::INTERRUPT => ExitCode::from(128 + number),
        // the reference ends with this error
        (_, Some(error)) => {
            output::eprintln(&format!("Error: {error}"));
            ExitCode::FAILURE
        }
        _ => code,
    }
}

/// Reads an option value with `int()`, like the reference; whether the
/// number suits is found out when connecting.
fn int_option(value: &str) -> Result<i64, ExitCode> {
    util::py::int(value, 10).map_err(|e| python_error(&e))
}

/// Reads an option value with `float()`, like the reference.
fn float_option(value: &str) -> Result<f64, ExitCode> {
    util::py::float(value).map_err(|e| python_error(&e))
}

/// Ends the program like the reference after an exception: `Error:` and
/// the message on stdout (the reference adds a traceback and exits with 0).
fn python_error(message: &str) -> ExitCode {
    output::println(&format!("Error: {message}"));
    ExitCode::FAILURE
}

/// Everything given on the command line.
#[allow(clippy::struct_excessive_bools)] // independent command line flags
struct Options {
    settings: Settings,
    json: bool,
    debug: bool,
    quiet: bool,
    force_interactive: bool,
    repeater_mode: bool,
    target: connect::Target,
    commands: Vec<String>,
}

/// Evaluates the options in order, like the reference does with getopt.
///
/// Returns `Err` with the exit code when the program is done (help,
/// version, listing, errors).
async fn parse_options(argv: &[String]) -> Result<Options, ExitCode> {
    let Ok((opts, commands)) = args::getopt(argv) else {
        output::println("Unrecognized option, use -h to get more help");
        output::print(&help::command_usage());
        return Err(ExitCode::FAILURE);
    };
    let mut o = Options {
        settings: Settings::default(),
        json: false,
        debug: false,
        quiet: false,
        force_interactive: false,
        repeater_mode: false,
        target: connect::Target {
            address: connect::default_address(),
            port: 5000,
            baudrate: 115_200,
            scan_timeout: 2.0,
            ..connect::Target::default()
        },
        commands,
    };
    for (opt, value) in opts {
        match opt {
            'i' => o.force_interactive = true,
            'c' => {
                if value == "off" {
                    o.settings.color = false;
                }
            }
            'C' => o.settings.classic = !o.settings.classic,
            'r' => o.repeater_mode = true,
            'a' | 'd' => o.target.address = value,
            'P' => o.target.pair = true,
            's' => {
                let spec = args::parse_serial_spec(&value);
                for error in &spec.errors {
                    log::error!("{error}");
                }
                if let Some(baud) = spec.baudrate {
                    o.target.baudrate = baud;
                }
                // levels from an earlier -s stay unless given again
                if spec.dtr.is_some() {
                    o.target.dtr = spec.dtr;
                }
                if spec.rts.is_some() {
                    o.target.rts = spec.rts;
                }
                o.target.serial_port = Some(spec.port);
            }
            'b' => o.target.baudrate = int_option(&value)?,
            't' => o.target.hostname = Some(value),
            'p' => o.target.port = int_option(&value)?,
            'j' => {
                o.json = true;
                o.settings.json_msgs = true;
            }
            'D' => o.debug = true,
            'h' => {
                output::println(&help::usage());
                return Err(ExitCode::SUCCESS);
            }
            'T' => o.target.scan_timeout = float_option(&value)?,
            'v' => {
                output::println(&help::version());
                return Err(ExitCode::SUCCESS);
            }
            'f' => o.target.first_device = true,
            'q' => o.quiet = true,
            'l' => {
                connect::list_devices(o.target.scan_timeout, o.repeater_mode).await;
                return Err(ExitCode::SUCCESS);
            }
            'S' => match connect::select_device(o.target.scan_timeout, o.repeater_mode).await {
                Some(connect::Selection::Ble(address)) => o.target.address = address,
                Some(connect::Selection::Serial(port)) => o.target.serial_port = Some(port),
                None => return Err(ExitCode::SUCCESS),
            },
            _ => {}
        }
    }
    Ok(o)
}

/// Runs an init script and prints its output.
///
/// # Errors
///
/// A script that cannot be read, or whose output cannot be written, ends the
/// program, as the exception does in the reference.
async fn run_init_script(
    app: &Arc<App>,
    path: &std::path::Path,
    json: bool,
) -> Result<(), ExitCode> {
    let mut sink = Sink::Stdout;
    let output = commands::process_script(app, &path.display().to_string(), json, &mut sink)
        .await
        .map_err(|e| python_error(&e))?;
    output::print(&output);
    if output::stdout_error().is_some() {
        return Err(ExitCode::FAILURE);
    }
    Ok(())
}

/// Runs the program; what is to be closed at its end goes to `session`.
async fn run(argv: Vec<String>, session: &OnceLock<Arc<App>>) -> ExitCode {
    let o = match parse_options(&argv).await {
        Ok(o) => o,
        Err(code) => return code,
    };

    if o.debug {
        Logger::set_level(LevelFilter::Debug);
    } else if o.json || o.quiet {
        Logger::set_level(LevelFilter::Error);
    }

    if o.repeater_mode {
        let Some(port) = o.target.serial_port.clone() else {
            log::error!("Repeater mode (-r) requires serial port (-s)");
            output::print(&help::command_usage());
            return ExitCode::FAILURE;
        };
        let (baudrate, color, commands) = (o.target.baudrate, o.settings.color, o.commands.clone());
        // a port that cannot be opened or stops working is a failed
        // connection (exit code 1)
        let connected =
            tokio::task::spawn_blocking(move || repeater::run(&port, baudrate, color, &commands))
                .await
                .unwrap_or(false);
        return if connected {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    let Some(mc) = connect::connect(&o.target).await else {
        return ExitCode::FAILURE;
    };

    let app = Arc::new(App::new(mc, o.settings.clone()));
    let _ = session.set(app.clone());
    let _events = display::spawn_event_task(app.clone());
    app.mc.set_auto_update_contacts(true);
    app.mc.set_decrypt_channel_logs(true);

    let device = match app.mc.send_device_query().await {
        Ok(device) => to_value(&device),
        Err(e) => {
            log::error!("Error while querying device: {e}");
            return ExitCode::FAILURE;
        }
    };

    let name = app.self_name();
    if paths::existing_config_dir().is_some() {
        app.state().message_log = Some(paths::message_log_file(&name));
    }
    let fw_ver = vint(&device, "fw ver");
    app.has_cli_cmd.store(fw_ver >= 14, Ordering::Relaxed);

    if o.json || o.quiet {
        Logger::set_level(if o.debug {
            LevelFilter::Debug
        } else {
            LevelFilter::Error
        });
    } else if fw_ver > 2 {
        log::info!(
            "Connected to {name} running on a {} ({fw_ver}) fw.",
            vstr(&device, "ver")
        );
    } else {
        log::info!("Connected to {name}.");
    }

    let init = paths::init_script();
    if !o.json && init.exists() {
        log::debug!("Executing init script : {}", init.display());
        if let Err(code) = run_init_script(&app, &init, o.json).await {
            return code;
        }
    }
    let init = paths::device_init_script(&name);
    if init.exists() {
        log::info!("Executing device init script : {}", init.display());
        if let Err(code) = run_init_script(&app, &init, o.json).await {
            return code;
        }
    } else {
        log::debug!("No device init script for {name}");
    }

    let mut force_interactive = o.force_interactive;
    if !o.commands.is_empty() {
        if o.commands
            .last()
            .is_some_and(|c| matches!(c.as_str(), "interactive" | "chat" | "im"))
        {
            force_interactive = true;
        }
        let mut sink = Sink::Stdout;
        if !commands::process_cmds(&app, &o.commands, o.json, &mut sink, "\n").await {
            log::error!(
                "Problem while executing {}",
                util::json::py_str_list(&o.commands)
            );
        }
    }

    if (o.commands.is_empty() || force_interactive) && output::stdout_error().is_none() {
        interactive::interactive_loop(&app, o.json).await;
    }
    ExitCode::SUCCESS
}
