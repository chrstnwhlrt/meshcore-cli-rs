//! meshcore-cli-rs - Rust CLI for `MeshCore` companion radios.

mod cli;
mod commands;
mod config;
mod display;
mod error;
mod interactive;

use clap::Parser;
use meshcore::MeshCore;
use meshcore::transport::serial::SerialConfig;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command};
use commands::CommandContext;
use config::Config;
use display::Display;
use error::{CliError, Result};

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    // Setup logging - respect RUST_LOG if set, otherwise use --debug flag
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if cli.debug {
            EnvFilter::new("debug")
        } else {
            EnvFilter::new("warn")
        }
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    // Handle list command
    if cli.list {
        list_serial_ports()?;
        return Ok(());
    }

    // Determine color setting
    let color = cli.color.unwrap_or(true);

    // Create display
    let display = Display::new(cli.json, color);

    // If no command and no serial port, show help
    if cli.command.is_none() && cli.serial.is_none() {
        // Enter interactive mode with device selection
        println!("No serial port specified. Use -s <port> to specify a serial port.");
        println!("Use -l to list available serial ports.");
        return Ok(());
    }

    // Get serial port
    let port = cli
        .serial
        .ok_or_else(|| CliError::Serial("No serial port specified. Use -s <port>".into()))?;

    // Connect to device
    let ctx = connect_device(&port, cli.baudrate, display).await?;

    // Run init scripts if not in JSON mode
    if !cli.json {
        run_init_scripts(&ctx).await?;
    }

    // Execute command or enter interactive mode
    match cli.command {
        Some(cmd) => execute_command(&ctx, cmd).await?,
        None => {
            // Enter interactive mode
            interactive::run(&ctx).await?;
        }
    }

    Ok(())
}

/// Connects to a device via serial port.
async fn connect_device(port: &str, baudrate: u32, display: Display) -> Result<CommandContext> {
    let config = SerialConfig::new(port).baud_rate(baudrate);

    let mut client = MeshCore::with_serial_config(config);

    // Connect and get self info
    let self_info = client
        .connect()
        .await
        .map_err(|e| CliError::Serial(format!("Failed to connect to {port}: {e}")))?;

    // Preload contacts so they're available for contact-based commands
    if let Err(e) = client.get_contacts().await {
        tracing::debug!("Failed to preload contacts: {e}");
    }

    let ctx = CommandContext::new(client, display, Some(self_info.name.clone()));

    Ok(ctx)
}

/// Lists available serial ports.
fn list_serial_ports() -> Result<()> {
    let ports = meshcore::transport::serial::list_ports()
        .map_err(|e| CliError::Serial(format!("Failed to list ports: {e}")))?;

    if ports.is_empty() {
        println!("No serial ports found");
    } else {
        println!("Available serial ports:");
        for port in ports {
            println!("  {port}");
        }
    }

    Ok(())
}

/// Runs init scripts.
async fn run_init_scripts(ctx: &CommandContext) -> Result<()> {
    // Run global init script
    if let Ok(lines) = Config::read_init_script() {
        for line in lines {
            if let Some(cmd) = parse_command_line(&line) {
                if let Err(e) = execute_command(ctx, cmd).await {
                    tracing::warn!("Init script error: {e}");
                }
            }
        }
    }

    // Run device-specific init script
    if let Some(name) = &ctx.device_name {
        if let Ok(lines) = Config::read_device_init_script(name) {
            for line in lines {
                if let Some(cmd) = parse_command_line(&line) {
                    if let Err(e) = execute_command(ctx, cmd).await {
                        tracing::warn!("Device init script error: {e}");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Parses a command line string into a Command.
fn parse_command_line(line: &str) -> Option<Command> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let cmd = parts[0].to_lowercase();
    let cmd_str = cmd.as_str();

    match cmd_str {
        // General commands
        "infos" | "i" => Some(Command::Infos),
        "ver" | "v" | "q" | "query" => Some(Command::Ver),
        "battery" => Some(Command::Battery),
        "clock" => Some(Command::Clock { sync: false }),
        "sync_time" | "st" => Some(Command::SyncTime),
        "reboot" => Some(Command::Reboot),
        "wait_key" | "wk" => Some(Command::WaitKey),
        "self_telemetry" | "t" => Some(Command::SelfTelemetry),
        "card" | "e" => Some(Command::Card),

        // Sleep
        "sleep" | "s" if parts.len() > 1 => {
            parts[1].parse().ok().map(|secs| Command::Sleep { secs })
        }

        // Contact commands
        "contacts" | "list" | "lc" => Some(Command::Contacts),
        "reload_contacts" | "rc" => Some(Command::ReloadContacts),
        "advert" | "a" => Some(Command::Advert),
        "floodadv" | "flood_advert" => Some(Command::FloodAdv),
        "pending_contacts" => Some(Command::PendingContacts),
        "flush_pending" => Some(Command::FlushPending),

        "contact_info" | "ci" if parts.len() > 1 => Some(Command::ContactInfo {
            contact: parts[1].to_string(),
        }),
        "contact_timeout" if parts.len() > 2 => {
            parts[2]
                .parse()
                .ok()
                .map(|timeout| Command::ContactTimeout {
                    contact: parts[1].to_string(),
                    timeout,
                })
        }
        "share_contact" | "sc" if parts.len() > 1 => Some(Command::ShareContact {
            contact: parts[1].to_string(),
        }),
        "export_contact" | "ec" => Some(Command::ExportContact {
            contact: parts.get(1).map(|s| (*s).to_string()),
        }),
        "import_contact" | "ic" if parts.len() > 1 => Some(Command::ImportContact {
            uri: parts[1].to_string(),
        }),
        "remove_contact" if parts.len() > 1 => Some(Command::RemoveContact {
            contact: parts[1].to_string(),
        }),
        "add_pending" if parts.len() > 1 => Some(Command::AddPending {
            pending: parts[1].to_string(),
        }),
        "path" if parts.len() > 1 => Some(Command::Path {
            contact: parts[1].to_string(),
        }),
        "disc_path" | "dp" if parts.len() > 1 => Some(Command::DiscPath {
            contact: parts[1].to_string(),
        }),
        "reset_path" | "rp" if parts.len() > 1 => Some(Command::ResetPath {
            contact: parts[1].to_string(),
        }),
        "change_path" | "cp" if parts.len() > 2 => Some(Command::ChangePath {
            contact: parts[1].to_string(),
            path: parts[2].to_string(),
        }),
        "change_flags" | "cf" if parts.len() > 2 => Some(Command::ChangeFlags {
            contact: parts[1].to_string(),
            flags: parts[2].to_string(),
        }),

        // Messaging commands
        "msg" | "m" if parts.len() > 2 => Some(Command::Msg {
            name: parts[1].to_string(),
            message: parts[2..].iter().map(|s| (*s).to_string()).collect(),
            wait: false,
            timeout: 30,
        }),
        "chan" | "ch" if parts.len() > 2 => parts[1].parse().ok().map(|channel| Command::Chan {
            channel,
            message: parts[2..].iter().map(|s| (*s).to_string()).collect(),
        }),
        "public" | "dch" if parts.len() > 1 => Some(Command::Public {
            message: parts[1..].iter().map(|s| (*s).to_string()).collect(),
        }),
        "recv" | "r" => Some(Command::Recv),
        "wait_msg" | "wm" => Some(Command::WaitMsg {
            timeout: parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(30),
        }),
        "trywait_msg" | "wmt" if parts.len() > 1 => parts[1]
            .parse()
            .ok()
            .map(|timeout| Command::TrywaitMsg { timeout }),
        "wmt8" => Some(Command::Wmt8),
        "sync_msgs" | "sm" => Some(Command::SyncMsgs),
        "wait_ack" | "wa" => Some(Command::WaitAck {
            timeout: parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(30),
        }),

        // Channel commands
        "get_channels" | "gc" => Some(Command::GetChannels),
        "get_channel" if parts.len() > 1 => Some(Command::GetChannel {
            channel: parts[1].to_string(),
        }),
        "set_channel" if parts.len() > 2 => Some(Command::SetChannel {
            number: parts[1].parse().ok()?,
            name: parts[2].to_string(),
            key: parts.get(3).map(|s| (*s).to_string()),
        }),
        "add_channel" if parts.len() > 1 => Some(Command::AddChannel {
            name: parts[1].to_string(),
            key: parts.get(2).map(|s| (*s).to_string()),
        }),
        "remove_channel" if parts.len() > 1 => Some(Command::RemoveChannel {
            channel: parts[1].to_string(),
        }),
        "scope" if parts.len() > 1 => Some(Command::Scope {
            scope: parts[1].to_string(),
        }),

        // Get/Set commands
        "get" if parts.len() > 1 => Some(Command::Get {
            param: parts[1].to_string(),
        }),
        "set" if parts.len() > 2 => Some(Command::Set {
            param: parts[1].to_string(),
            value: parts[2..].join(" "),
        }),
        "time" if parts.len() > 1 => parts[1].parse().ok().map(|epoch| Command::Time { epoch }),

        // Repeater commands
        "login" | "l" if parts.len() > 2 => Some(Command::Login {
            name: parts[1].to_string(),
            password: parts[2].to_string(),
        }),
        "logout" if parts.len() > 1 => Some(Command::Logout {
            name: parts[1].to_string(),
        }),
        "cmd" | "c" if parts.len() > 2 => Some(Command::Cmd {
            name: parts[1].to_string(),
            command: parts[2..].iter().map(|s| (*s).to_string()).collect(),
            wait: false,
            timeout: 30,
        }),
        "req_status" | "rs" if parts.len() > 1 => Some(Command::ReqStatus {
            name: parts[1].to_string(),
        }),
        "req_neighbours" | "rn" if parts.len() > 1 => Some(Command::ReqNeighbours {
            name: parts[1].to_string(),
        }),
        "req_telemetry" | "rt" if parts.len() > 1 => Some(Command::ReqTelemetry {
            contact: parts[1].to_string(),
        }),
        "req_mma" | "rm" if parts.len() > 1 => Some(Command::ReqMma {
            contact: parts[1].to_string(),
        }),
        "req_acl" if parts.len() > 1 => Some(Command::ReqAcl {
            contact: parts[1].to_string(),
        }),
        "trace" | "tr" if parts.len() > 1 => Some(Command::Trace {
            path: parts[1].to_string(),
        }),

        // Node discovery
        "node_discover" | "nd" => Some(Command::NodeDiscover {
            filter: parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
        }),

        // Advanced commands
        "export_key" => Some(Command::ExportKey),
        "import_key" if parts.len() > 1 => Some(Command::ImportKey {
            key: parts[1].to_string(),
        }),
        "get_vars" => Some(Command::GetVars),
        "set_var" if parts.len() > 2 => Some(Command::SetVar {
            key: parts[1].to_string(),
            value: parts[2..].join(" "),
        }),
        "stats" => Some(Command::Stats {
            stats_type: match parts.get(1).map(|s| s.to_lowercase()).as_deref() {
                Some("radio") => cli::StatsTypeArg::Radio,
                Some("packets") => cli::StatsTypeArg::Packets,
                _ => cli::StatsTypeArg::Core,
            },
        }),

        _ => None,
    }
}

/// Executes a single command.
async fn execute_command(ctx: &CommandContext, cmd: Command) -> Result<()> {
    match cmd {
        // General commands
        Command::Chat => interactive::run(ctx).await,
        Command::ChatTo { contact } => {
            ctx.state.lock().await.set_contact(Some(contact));
            interactive::run(ctx).await
        }
        Command::Script { filename } => Box::pin(execute_script(ctx, &filename)).await,
        Command::Infos => ctx.cmd_infos().await,
        Command::SelfTelemetry => ctx.cmd_self_telemetry().await,
        Command::Card => ctx.cmd_card().await,
        Command::Ver => ctx.cmd_ver().await,
        Command::Reboot => ctx.cmd_reboot().await,
        Command::Sleep { secs } => ctx.cmd_sleep(secs).await,
        Command::WaitKey => {
            CommandContext::cmd_wait_key();
            Ok(())
        }
        Command::ApplyTo { filter, commands } => ctx.cmd_apply_to(&filter, &commands).await,

        // Messaging commands
        Command::Msg {
            name,
            message,
            wait,
            timeout,
        } => ctx.cmd_msg(&name, &message, wait, timeout).await,
        Command::WaitAck { timeout } => ctx.cmd_wait_ack(timeout).await,
        Command::Chan { channel, message } => ctx.cmd_chan(channel, &message).await,
        Command::Public { message } => ctx.cmd_public(&message).await,
        Command::Recv => ctx.cmd_recv().await,
        Command::WaitMsg { timeout } => ctx.cmd_wait_msg(timeout).await,
        Command::TrywaitMsg { timeout } => ctx.cmd_trywait_msg(timeout).await,
        Command::SyncMsgs => ctx.cmd_sync_msgs().await,
        Command::MsgsSubscribe => ctx.cmd_msgs_subscribe().await,
        Command::GetChannels => ctx.cmd_get_channels().await,
        Command::GetChannel { channel } => ctx.cmd_get_channel(&channel).await,
        Command::SetChannel { number, name, key } => {
            ctx.cmd_set_channel(number, &name, key.as_deref()).await
        }
        Command::RemoveChannel { channel } => ctx.cmd_remove_channel(&channel).await,
        Command::AddChannel { name, key } => ctx.cmd_add_channel(&name, key.as_deref()).await,
        Command::Scope { scope } => ctx.cmd_scope(&scope).await,

        // Management commands
        Command::Advert => ctx.cmd_advert(false).await,
        Command::FloodAdv => ctx.cmd_advert(true).await,
        Command::Get { param } => ctx.cmd_get(&param).await,
        Command::Set { param, value } => ctx.cmd_set(&param, &value).await,
        Command::Time { epoch } => ctx.cmd_set_time(epoch).await,
        Command::Clock { sync } => ctx.cmd_clock(sync).await,
        Command::SyncTime => ctx.cmd_sync_time().await,
        Command::NodeDiscover { filter } => ctx.cmd_node_discover(filter).await,

        // Contact commands
        Command::Contacts | Command::List => ctx.cmd_contacts().await,
        Command::ReloadContacts => ctx.cmd_reload_contacts().await,
        Command::ContactInfo { contact } => ctx.cmd_contact_info(&contact).await,
        Command::ContactTimeout { contact, timeout } => {
            ctx.cmd_contact_timeout(&contact, timeout).await
        }
        Command::ShareContact { contact } => ctx.cmd_share_contact(&contact).await,
        Command::ExportContact { contact } => ctx.cmd_export_contact(contact.as_deref()).await,
        Command::ImportContact { uri } => ctx.cmd_import_contact(&uri).await,
        Command::RemoveContact { contact } => ctx.cmd_remove_contact(&contact).await,
        Command::Path { contact } => ctx.cmd_path(&contact).await,
        Command::DiscPath { contact } => ctx.cmd_disc_path(&contact).await,
        Command::ResetPath { contact } => ctx.cmd_reset_path(&contact).await,
        Command::ChangePath { contact, path } => ctx.cmd_change_path(&contact, &path).await,
        Command::ChangeFlags { contact, flags } => ctx.cmd_change_flags(&contact, &flags).await,
        Command::ReqTelemetry { contact } => ctx.cmd_req_telemetry(&contact).await,
        Command::ReqMma { contact } => ctx.cmd_req_mma(&contact).await,
        Command::ReqAcl { contact } => ctx.cmd_req_acl(&contact).await,
        Command::PendingContacts => ctx.cmd_pending_contacts().await,
        Command::AddPending { pending } => ctx.cmd_add_pending(&pending).await,
        Command::FlushPending => ctx.cmd_flush_pending().await,

        // Repeater commands
        Command::Login { name, password } => ctx.cmd_login(&name, &password).await,
        Command::Logout { name } => ctx.cmd_logout(&name).await,
        Command::Cmd {
            name,
            command,
            wait,
            timeout,
        } => ctx.cmd_cmd(&name, &command, wait, timeout).await,
        Command::Wmt8 => ctx.cmd_wmt8().await,
        Command::ReqStatus { name } => ctx.cmd_req_status(&name).await,
        Command::ReqNeighbours { name } => ctx.cmd_req_neighbours(&name).await,
        Command::ReqBinary { name, data } => ctx.cmd_req_binary(&name, &data).await,
        Command::Trace { path } => ctx.cmd_trace(&path).await,

        // Advanced commands
        Command::Battery => ctx.cmd_battery().await,
        Command::Stats { stats_type } => ctx.cmd_stats(stats_type).await,
        Command::ExportKey => ctx.cmd_export_key().await,
        Command::ImportKey { key } => ctx.cmd_import_key(&key).await,
        Command::GetVars => ctx.cmd_get_vars().await,
        Command::SetVar { key, value } => ctx.cmd_set_var(&key, &value).await,
    }
}

/// Executes a script file.
async fn execute_script(ctx: &CommandContext, filename: &str) -> Result<()> {
    let content = std::fs::read_to_string(filename).map_err(|e| CliError::Script {
        line: 0,
        message: format!("Failed to read script: {e}"),
    })?;

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(cmd) = parse_command_line(line) {
            Box::pin(execute_command(ctx, cmd))
                .await
                .map_err(|e| CliError::Script {
                    line: line_num + 1,
                    message: e.to_string(),
                })?;
        } else {
            return Err(CliError::Script {
                line: line_num + 1,
                message: format!("Unknown command: {line}"),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::commands::parse_time_value;

    #[test]
    fn test_parse_time_value_seconds() {
        assert_eq!(parse_time_value("60s"), 60);
        assert_eq!(parse_time_value("60"), 60);
        assert_eq!(parse_time_value("0s"), 0);
    }

    #[test]
    fn test_parse_time_value_minutes() {
        assert_eq!(parse_time_value("1m"), 60);
        assert_eq!(parse_time_value("30m"), 1800);
        assert_eq!(parse_time_value("0m"), 0);
    }

    #[test]
    fn test_parse_time_value_hours() {
        assert_eq!(parse_time_value("1h"), 3600);
        assert_eq!(parse_time_value("24h"), 86400);
        assert_eq!(parse_time_value("0h"), 0);
    }

    #[test]
    fn test_parse_time_value_days() {
        assert_eq!(parse_time_value("1d"), 86_400);
        assert_eq!(parse_time_value("7d"), 604_800);
        assert_eq!(parse_time_value("0d"), 0);
    }

    #[test]
    fn test_parse_time_value_edge_cases() {
        assert_eq!(parse_time_value(""), 0);
        assert_eq!(parse_time_value("  2h  "), 7200);
        assert_eq!(parse_time_value("invalid"), 0);
    }
}
