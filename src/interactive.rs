//! Interactive (chat) mode.

use std::borrow::Cow;

use crossterm::ExecutableCommand;
use crossterm::style::{Color, ResetColor, SetForegroundColor};
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Editor, Helper};

use crate::commands::CommandContext;
use crate::config::Config;
use crate::error::Result;

/// Interactive mode helper for rustyline.
struct InteractiveHelper {
    /// Contact names for completion.
    contacts: Vec<String>,
    /// Command names for completion.
    commands: Vec<&'static str>,
}

impl InteractiveHelper {
    fn new() -> Self {
        Self {
            contacts: Vec::new(),
            commands: vec![
                // General
                "quit",
                "q",
                "exit",
                "help",
                "?",
                "to",
                "infos",
                "i",
                "ver",
                "v",
                "battery",
                "clock",
                "reboot",
                "sleep",
                "s",
                "advert",
                "a",
                "floodadv",
                "scope",
                // Contacts
                "contacts",
                "list",
                "lc",
                "reload_contacts",
                "rc",
                "contact_info",
                "ci",
                "contact_name",
                "cn",
                "contact_key",
                "ck",
                "contact_type",
                "ct",
                "contact_lastmod",
                "clm",
                "dtrace",
                "dt",
                "path",
                "disc_path",
                "dp",
                "reset_path",
                "rp",
                "change_path",
                "cp",
                "change_flags",
                "cf",
                "share_contact",
                "sc",
                "export_contact",
                "ec",
                "import_contact",
                "ic",
                "remove_contact",
                "pending_contacts",
                "add_pending",
                "flush_pending",
                // Messaging
                "msg",
                "m",
                "{",
                "send",
                "chan",
                "ch",
                "public",
                "dch",
                "recv",
                "r",
                "wait_msg",
                "wm",
                "wait_ack",
                "wa",
                "}",
                "sync_msgs",
                "sm",
                "msgs_subscribe",
                "ms",
                // Channels
                "get_channels",
                "gc",
                "get_channel",
                "set_channel",
                "remove_channel",
                "add_channel",
                // Device management
                "node_discover",
                "nd",
                "contact_timeout",
                "req_acl",
                "time",
                // Repeaters
                "login",
                "l",
                "logout",
                "cmd",
                "c",
                "[",
                "req_status",
                "rs",
                "req_neighbours",
                "rn",
                "req_telemetry",
                "rt",
                "req_mma",
                "rm",
                "req_binary",
                "rb",
                "trace",
                "tr",
                "wmt8",
                "]",
                "trywait_msg",
                "wmt",
                // Advanced
                "get",
                "set",
                "stats",
                "export_key",
                "import_key",
                "get_vars",
                "set_var",
                "self_telemetry",
                "t",
                "card",
                "e",
                // Scripts
                "script",
                "apply_to",
                "at",
            ],
        }
    }

    fn update_contacts(&mut self, contacts: Vec<String>) {
        self.contacts = contacts;
    }
}

impl Completer for InteractiveHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let line = &line[..pos];
        let words: Vec<&str> = line.split_whitespace().collect();

        if words.is_empty() || (words.len() == 1 && !line.ends_with(' ')) {
            // Complete command
            let prefix = words.first().unwrap_or(&"");
            let matches: Vec<Pair> = self
                .commands
                .iter()
                .filter(|c| c.starts_with(prefix))
                .map(|c| Pair {
                    display: (*c).to_string(),
                    replacement: (*c).to_string(),
                })
                .collect();
            let start = line.rfind(char::is_whitespace).map_or(0, |i| i + 1);
            Ok((start, matches))
        } else {
            // Complete contact name for relevant commands
            let cmd = words[0].to_lowercase();
            let needs_contact = matches!(
                cmd.as_str(),
                "to" | "msg"
                    | "m"
                    | "send"
                    | "cmd"
                    | "c"
                    | "login"
                    | "l"
                    | "logout"
                    | "contact_info"
                    | "ci"
                    | "path"
                    | "disc_path"
                    | "dp"
                    | "reset_path"
                    | "rp"
                    | "change_path"
                    | "cp"
                    | "change_flags"
                    | "cf"
                    | "share_contact"
                    | "sc"
                    | "export_contact"
                    | "ec"
                    | "remove_contact"
                    | "req_status"
                    | "rs"
                    | "req_neighbours"
                    | "rn"
                    | "req_telemetry"
                    | "rt"
                    | "req_mma"
                    | "rm"
                    | "req_binary"
                    | "rb"
            );

            if needs_contact && (words.len() == 1 || (words.len() == 2 && !line.ends_with(' '))) {
                let prefix = words.get(1).unwrap_or(&"").to_lowercase();
                let matches: Vec<Pair> = self
                    .contacts
                    .iter()
                    .filter(|c| c.to_lowercase().starts_with(&prefix))
                    .map(|c| Pair {
                        display: c.clone(),
                        replacement: c.clone(),
                    })
                    .collect();
                let start = line.rfind(char::is_whitespace).map_or(0, |i| i + 1);
                Ok((start, matches))
            } else {
                Ok((pos, Vec::new()))
            }
        }
    }
}

impl Hinter for InteractiveHelper {
    type Hint = String;

    fn hint(&self, _line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        None
    }
}

impl Highlighter for InteractiveHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        Cow::Borrowed(prompt)
    }
}

impl Validator for InteractiveHelper {}

impl Helper for InteractiveHelper {}

/// Runs interactive mode.
pub async fn run(ctx: &CommandContext) -> Result<()> {
    println!("Interactive mode. Type 'help' for commands, 'quit' to exit.");

    let mut helper = InteractiveHelper::new();

    // Update contact list
    {
        let client = ctx.client.lock().await;
        let contacts = client.contacts().await;
        helper.update_contacts(contacts.values().map(|c| c.name.clone()).collect());
    }

    let mut rl: Editor<InteractiveHelper, DefaultHistory> = Editor::new().map_err(|e| {
        crate::error::CliError::Io(std::io::Error::other(format!(
            "Failed to create editor: {e}"
        )))
    })?;
    rl.set_helper(Some(helper));

    // Load history
    if let Some(history_file) = Config::history_file() {
        let _ = rl.load_history(&history_file);
    }

    // Subscribe to events in background
    let subscription = ctx.subscribe().await;
    let display = ctx.display.clone();
    let state = ctx.state.clone();
    let client = ctx.client.clone();

    let event_task = tokio::spawn(async move {
        let mut subscription = subscription;
        loop {
            tokio::select! {
                event = subscription.recv() => {
                    if let Some(event) = event {
                        handle_background_event(&event, &display, &state, &client).await;
                    } else {
                        break;
                    }
                }
            }
        }
    });

    loop {
        // Build prompt
        let prompt = build_prompt(ctx).await;

        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(line);

                // Handle special commands
                match line.to_lowercase().as_str() {
                    "quit" | "q" | "exit" => break,
                    "help" | "?" => {
                        print_help();
                        continue;
                    }
                    _ => {}
                }

                // Parse and execute command
                if let Err(e) = process_line(ctx, line).await {
                    ctx.display.print_error(&e.to_string());
                }

                // Update contact list for completion
                {
                    let client = ctx.client.lock().await;
                    let contacts = client.contacts().await;
                    if let Some(helper) = rl.helper_mut() {
                        helper.update_contacts(contacts.values().map(|c| c.name.clone()).collect());
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(e) => {
                eprintln!("Error: {e}");
                break;
            }
        }
    }

    // Save history
    if let Some(history_file) = Config::history_file() {
        let _ = rl.save_history(&history_file);
    }

    // Cancel event task
    event_task.abort();

    Ok(())
}

/// Builds the interactive prompt.
async fn build_prompt(ctx: &CommandContext) -> String {
    let state = ctx.state.lock().await;

    let device_name = state
        .device_name
        .clone()
        .unwrap_or_else(|| "meshcore-cli-rs".into());
    let current = state.current_contact.clone();
    let scope = state.flood_scope.clone();

    drop(state);

    let mut prompt = current.unwrap_or(device_name);

    if let Some(scope) = scope {
        prompt = format!("{prompt}%{scope}");
    }

    format!("{prompt}> ")
}

/// Processes a line of input.
async fn process_line(ctx: &CommandContext, line: &str) -> Result<()> {
    let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
    let cmd = parts[0].to_lowercase();
    let args = parts.get(1).unwrap_or(&"");

    match cmd.as_str() {
        // Navigation
        "to" => {
            let target = args.trim();
            let mut state = ctx.state.lock().await;

            if target == "/" || target == "~" {
                // Go to root
                state.set_contact(None);
            } else if target == ".." {
                // Swap with previous
                state.swap_contacts();
            } else if target == "!" {
                // Go to last sender
                if let Some(sender) = state.last_sender.clone() {
                    state.set_contact(Some(sender));
                }
            } else if !target.is_empty() {
                // Check if it contains a scope suffix
                let (name, scope) = if let Some((n, s)) = target.split_once('%') {
                    (n, Some(s))
                } else {
                    (target, None)
                };

                // Verify contact exists
                drop(state);
                let contact = ctx.get_contact(name).await?;
                let mut state = ctx.state.lock().await;
                state.set_contact(Some(contact.name.clone()));

                if let Some(s) = scope {
                    state.flood_scope = Some(s.to_string());
                }
            }
            Ok(())
        }

        // Quick message sending (when in a contact)
        "send" | "\"" => {
            let state = ctx.state.lock().await;
            if let Some(contact) = state.current_contact.clone() {
                drop(state);
                let message = vec![(*args).to_string()];
                ctx.cmd_msg(&contact, &message, false, 30).await
            } else {
                ctx.display
                    .print_error("No contact selected. Use 'to <contact>' first.");
                Ok(())
            }
        }

        // Forward to command handler
        _ => {
            // Check if we should send as message (when in a chat contact)
            let state = ctx.state.lock().await;
            let current = state.current_contact.clone();
            drop(state);

            // If we're in a contact and the line doesn't start with a command,
            // treat it as a message
            if let Some(contact) = current {
                // Check if it's a known command
                let is_command = matches!(
                    cmd.as_str(),
                    "infos"
                        | "i"
                        | "ver"
                        | "v"
                        | "battery"
                        | "clock"
                        | "reboot"
                        | "contacts"
                        | "list"
                        | "lc"
                        | "contact_info"
                        | "ci"
                        | "contact_name"
                        | "cn"
                        | "contact_key"
                        | "ck"
                        | "contact_type"
                        | "ct"
                        | "contact_lastmod"
                        | "clm"
                        | "dtrace"
                        | "dt"
                        | "msg"
                        | "m"
                        | "{"
                        | "chan"
                        | "ch"
                        | "recv"
                        | "r"
                        | "path"
                        | "login"
                        | "l"
                        | "logout"
                        | "cmd"
                        | "c"
                        | "["
                        | "get"
                        | "set"
                        | "advert"
                        | "a"
                        | "scope"
                        | "wait_ack"
                        | "wa"
                        | "}"
                        | "wait_msg"
                        | "wm"
                        | "wmt8"
                        | "]"
                        | "sync_msgs"
                        | "sm"
                        | "msgs_subscribe"
                        | "ms"
                        | "self_telemetry"
                        | "t"
                        | "card"
                        | "e"
                        | "stats"
                        | "export_contact"
                        | "ec"
                        | "import_contact"
                        | "ic"
                        | "share_contact"
                        | "sc"
                        | "remove_contact"
                        | "change_path"
                        | "cp"
                        | "change_flags"
                        | "cf"
                        | "add_pending"
                        | "script"
                        | "apply_to"
                        | "at"
                        | "export_key"
                        | "import_key"
                        | "get_vars"
                        | "set_var"
                        | "help"
                        | "?"
                );

                if !is_command && !line.starts_with('/') && !line.starts_with('.') {
                    // Send as message
                    let message = vec![line.to_string()];
                    return ctx.cmd_msg(&contact, &message, false, 30).await;
                }
            }

            // Parse as command
            forward_command(ctx, &cmd, args).await
        }
    }
}

/// Forwards a command to the appropriate handler.
async fn forward_command(ctx: &CommandContext, cmd: &str, args: &str) -> Result<()> {
    let args_vec: Vec<String> = if args.is_empty() {
        Vec::new()
    } else {
        args.split_whitespace().map(String::from).collect()
    };

    match cmd {
        // General
        "infos" | "i" => ctx.cmd_infos().await,
        "ver" | "v" => ctx.cmd_ver().await,
        "battery" => ctx.cmd_battery().await,
        "clock" => ctx.cmd_clock(false).await,
        "sync_time" | "st" => ctx.cmd_sync_time().await,
        "reboot" => ctx.cmd_reboot().await,
        "advert" | "a" => ctx.cmd_advert(false).await,
        "floodadv" => ctx.cmd_advert(true).await,
        "card" | "e" => ctx.cmd_card().await,
        "self_telemetry" | "t" => ctx.cmd_self_telemetry().await,

        // Contacts
        "contacts" | "list" | "lc" => ctx.cmd_contacts().await,
        "reload_contacts" | "rc" => ctx.cmd_reload_contacts().await,
        "contact_info" | "ci" if !args.is_empty() => ctx.cmd_contact_info(args.trim()).await,
        "path" if !args.is_empty() => ctx.cmd_path(args.trim()).await,
        "disc_path" | "dp" if !args.is_empty() => ctx.cmd_disc_path(args.trim()).await,
        "reset_path" | "rp" if !args.is_empty() => ctx.cmd_reset_path(args.trim()).await,
        "pending_contacts" => ctx.cmd_pending_contacts().await,
        "flush_pending" => ctx.cmd_flush_pending().await,
        "add_pending" if !args.is_empty() => ctx.cmd_add_pending(args.trim()).await,
        "change_path" | "cp" if args_vec.len() >= 2 => {
            ctx.cmd_change_path(&args_vec[0], &args_vec[1]).await
        }
        "change_flags" | "cf" if args_vec.len() >= 2 => {
            ctx.cmd_change_flags(&args_vec[0], &args_vec[1]).await
        }
        "share_contact" | "sc" if !args.is_empty() => ctx.cmd_share_contact(args.trim()).await,
        "export_contact" | "ec" => {
            let contact = if args.is_empty() {
                None
            } else {
                Some(args.trim())
            };
            ctx.cmd_export_contact(contact).await
        }
        "import_contact" | "ic" if !args.is_empty() => ctx.cmd_import_contact(args.trim()).await,
        "remove_contact" if !args.is_empty() => ctx.cmd_remove_contact(args.trim()).await,

        // Contact-context commands (use current contact if no arg)
        "contact_name" | "cn" => {
            let name = if args.is_empty() {
                ctx.state.lock().await.current_contact.clone()
            } else {
                Some(args.trim().to_string())
            };
            if let Some(name) = name {
                let contact = ctx.get_contact(&name).await?;
                println!("{}", contact.name);
            } else {
                ctx.display.print_error("No contact selected");
            }
            Ok(())
        }
        "contact_key" | "ck" => {
            let name = if args.is_empty() {
                ctx.state.lock().await.current_contact.clone()
            } else {
                Some(args.trim().to_string())
            };
            if let Some(name) = name {
                let contact = ctx.get_contact(&name).await?;
                println!("{}", contact.public_key.to_hex());
            } else {
                ctx.display.print_error("No contact selected");
            }
            Ok(())
        }
        "contact_type" | "ct" => {
            let name = if args.is_empty() {
                ctx.state.lock().await.current_contact.clone()
            } else {
                Some(args.trim().to_string())
            };
            if let Some(name) = name {
                let contact = ctx.get_contact(&name).await?;
                let type_str = match contact.device_type {
                    meshcore::types::ContactType::Node => "node",
                    meshcore::types::ContactType::Repeater => "repeater",
                    meshcore::types::ContactType::Room => "room",
                    meshcore::types::ContactType::Unknown => "unknown",
                };
                println!("{type_str}");
            } else {
                ctx.display.print_error("No contact selected");
            }
            Ok(())
        }
        "dtrace" | "dt" => {
            let name = if args.is_empty() {
                ctx.state.lock().await.current_contact.clone()
            } else {
                Some(args.trim().to_string())
            };
            if let Some(name) = name {
                // Discover path first, then show it
                ctx.cmd_disc_path(&name).await?;
                ctx.cmd_path(&name).await
            } else {
                ctx.display.print_error("No contact selected");
                Ok(())
            }
        }
        "contact_lastmod" | "clm" => {
            let name = if args.is_empty() {
                ctx.state.lock().await.current_contact.clone()
            } else {
                Some(args.trim().to_string())
            };
            if let Some(name) = name {
                let contact = ctx.get_contact(&name).await?;
                if contact.last_modified > 0 {
                    use chrono::{TimeZone, Utc};
                    if let Some(dt) = Utc
                        .timestamp_opt(i64::from(contact.last_modified), 0)
                        .single()
                    {
                        println!("{}", dt.format("%Y-%m-%d %H:%M:%S"));
                    } else {
                        println!("{}", contact.last_modified);
                    }
                } else {
                    println!("never");
                }
            } else {
                ctx.display.print_error("No contact selected");
            }
            Ok(())
        }

        // Messaging
        "msg" | "m" | "{" if args_vec.len() >= 2 => {
            ctx.cmd_msg(&args_vec[0], &args_vec[1..], false, 30).await
        }
        "recv" | "r" => ctx.cmd_recv().await,
        "sync_msgs" | "sm" => ctx.cmd_sync_msgs().await,
        "msgs_subscribe" | "ms" => ctx.cmd_msgs_subscribe().await,
        "wait_ack" | "wa" | "}" => {
            let timeout = args_vec.first().and_then(|s| s.parse().ok()).unwrap_or(30);
            ctx.cmd_wait_ack(timeout).await
        }
        "wait_msg" | "wm" => {
            let timeout = args_vec.first().and_then(|s| s.parse().ok()).unwrap_or(30);
            ctx.cmd_wait_msg(timeout).await
        }
        "trywait_msg" | "wmt" if !args.is_empty() => {
            let timeout: u64 = args.trim().parse().unwrap_or(8);
            ctx.cmd_trywait_msg(timeout).await
        }
        "chan" | "ch" if args_vec.len() >= 2 => {
            let channel: u8 = args_vec[0].parse().unwrap_or(0);
            ctx.cmd_chan(channel, &args_vec[1..]).await
        }
        "public" | "dch" if !args.is_empty() => ctx.cmd_public(&[args.to_string()]).await,

        // Repeaters
        "login" | "l" if args_vec.len() >= 2 => ctx.cmd_login(&args_vec[0], &args_vec[1]).await,
        "logout" if !args.is_empty() => ctx.cmd_logout(args.trim()).await,
        "cmd" | "c" | "[" if args_vec.len() >= 2 => {
            ctx.cmd_cmd(&args_vec[0], &args_vec[1..], false, 30).await
        }
        "req_status" | "rs" if !args.is_empty() => ctx.cmd_req_status(args.trim()).await,
        "wmt8" | "]" => ctx.cmd_wmt8().await,
        "trace" | "tr" if !args.is_empty() => ctx.cmd_trace(args.trim()).await,

        // Repeaters
        "req_binary" | "rb" if args_vec.len() >= 2 => {
            ctx.cmd_req_binary(&args_vec[0], &args_vec[1]).await
        }
        "req_neighbours" | "rn" if !args.is_empty() => ctx.cmd_req_neighbours(args.trim()).await,
        "req_telemetry" | "rt" if !args.is_empty() => ctx.cmd_req_telemetry(args.trim()).await,
        "req_mma" | "rm" if !args.is_empty() => ctx.cmd_req_mma(args.trim()).await,

        // Channels
        "get_channels" | "gc" => ctx.cmd_get_channels().await,
        "get_channel" if !args.is_empty() => ctx.cmd_get_channel(args.trim()).await,
        "set_channel" if args_vec.len() >= 3 => {
            let num: u8 = args_vec[0].parse().unwrap_or(0);
            let key = args_vec.get(2).map(String::as_str);
            ctx.cmd_set_channel(num, &args_vec[1], key).await
        }
        "add_channel" if !args_vec.is_empty() => {
            let key = args_vec.get(1).map(String::as_str);
            ctx.cmd_add_channel(&args_vec[0], key).await
        }
        "remove_channel" if !args.is_empty() => ctx.cmd_remove_channel(args.trim()).await,
        "scope" if !args.is_empty() => ctx.cmd_scope(args.trim()).await,

        // Device management
        "node_discover" | "nd" => {
            let filter: u8 = args.trim().parse().unwrap_or(0);
            ctx.cmd_node_discover(filter).await
        }
        "contact_timeout" if args_vec.len() >= 2 => {
            let timeout: u64 = args_vec[1].parse().unwrap_or(30);
            ctx.cmd_contact_timeout(&args_vec[0], timeout).await
        }
        "req_acl" if !args.is_empty() => ctx.cmd_req_acl(args.trim()).await,
        "time" if !args.is_empty() => {
            let epoch: u32 = args.trim().parse().unwrap_or(0);
            ctx.cmd_set_time(epoch).await
        }

        // Get/Set
        "get" if !args.is_empty() => ctx.cmd_get(args.trim()).await,
        "set" if args_vec.len() >= 2 => ctx.cmd_set(&args_vec[0], &args_vec[1..].join(" ")).await,

        // Stats
        "stats" => {
            let st = match args.trim() {
                "radio" => crate::cli::StatsTypeArg::Radio,
                "packets" => crate::cli::StatsTypeArg::Packets,
                _ => crate::cli::StatsTypeArg::Core,
            };
            ctx.cmd_stats(st).await
        }

        // Sleep
        "sleep" | "s" => {
            let secs: f64 = args.trim().parse().unwrap_or(1.0);
            ctx.cmd_sleep(secs).await
        }

        // Script and apply_to
        "script" if !args.is_empty() => ctx.cmd_script(args.trim()).await,
        "apply_to" | "at" if args_vec.len() >= 2 => {
            ctx.cmd_apply_to(&args_vec[0], &args_vec[1..]).await
        }

        // Advanced
        "export_key" => ctx.cmd_export_key().await,
        "import_key" if !args.is_empty() => ctx.cmd_import_key(args.trim()).await,
        "get_vars" => ctx.cmd_get_vars().await,
        "set_var" if args_vec.len() >= 2 => {
            ctx.cmd_set_var(&args_vec[0], &args_vec[1..].join(" "))
                .await
        }

        _ => {
            ctx.display.print_error(&format!("Unknown command: {cmd}"));
            Ok(())
        }
    }
}

/// Handles a background event.
async fn handle_background_event(
    event: &meshcore::event::Event,
    display: &crate::display::Display,
    state: &std::sync::Arc<tokio::sync::Mutex<crate::config::SessionState>>,
    client: &std::sync::Arc<
        tokio::sync::Mutex<meshcore::MeshCore<meshcore::transport::serial::SerialTransport>>,
    >,
) {
    use meshcore::event::Event;

    match event {
        Event::ContactMessage(msg) => {
            let contacts = client.lock().await.contacts().await;
            let sender_name = crate::commands::lookup_sender_name(&contacts, &msg.sender_prefix);

            // Print above the prompt
            let mut stdout = std::io::stdout();
            let _ = stdout.execute(SetForegroundColor(Color::Cyan));
            println!("\r{sender_name}: {}", msg.text);
            let _ = stdout.execute(ResetColor);

            let mut state = state.lock().await;
            state.last_sender = Some(sender_name);
        }
        Event::ChannelMessage(msg) => {
            // Channel messages don't include sender info
            let mut stdout = std::io::stdout();
            let _ = stdout.execute(SetForegroundColor(Color::Green));
            println!("\r#{}: {}", msg.channel_index, msg.text);
            let _ = stdout.execute(ResetColor);
        }
        Event::Ack(ack) => {
            let mut stdout = std::io::stdout();
            let _ = stdout.execute(SetForegroundColor(Color::Green));
            println!("\r[ACK {:08x}]", ack.code);
            let _ = stdout.execute(ResetColor);
        }
        Event::Advertisement(key) => {
            if !display.is_json() {
                let mut stdout = std::io::stdout();
                let _ = stdout.execute(SetForegroundColor(Color::Yellow));
                println!("\r[Advert from {}]", key.to_hex());
                let _ = stdout.execute(ResetColor);
            }

            let mut state = state.lock().await;
            state.add_pending(key.to_hex(), None);
        }
        Event::NewContactAdvert(contact) => {
            if !display.is_json() {
                let mut stdout = std::io::stdout();
                let _ = stdout.execute(SetForegroundColor(Color::Yellow));
                println!(
                    "\r[New contact: {} ({})]",
                    contact.name,
                    contact.public_key.to_hex()
                );
                let _ = stdout.execute(ResetColor);
            }

            let mut state = state.lock().await;
            state.add_pending_contact(*contact.clone());
        }
        Event::LoginSuccess => {
            display.print_ok("Login success");
        }
        Event::LoginFailed => {
            display.print_error("Login failed");
        }
        Event::MessagesWaiting => {
            if !display.is_json() {
                println!("\r[Messages waiting]");
            }
        }
        _ => {}
    }
}

/// Prints help information.
fn print_help() {
    println!("Interactive Mode Commands:");
    println!();
    println!("Navigation:");
    println!("  to <contact>     - Select a contact (supports %scope suffix)");
    println!("  to / or to ~     - Go to root (your device)");
    println!("  to ..            - Go to previous contact");
    println!("  to !             - Go to last message sender");
    println!();
    println!("When in a contact, just type to send a message.");
    println!();
    println!("Device Commands:");
    println!("  infos (i)        - Device info");
    println!("  ver (v)          - Firmware version");
    println!("  battery          - Battery status");
    println!("  get <param>      - Get parameter (use 'get help' for list)");
    println!("  set <p> <v>      - Set parameter (use 'set help' for list)");
    println!();
    println!("Contact Commands:");
    println!("  contacts (lc)    - List contacts");
    println!("  contact_info (ci)- Contact details");
    println!("  cn / ck / ct     - Contact name/key/type");
    println!("  path             - Show path to contact");
    println!("  dtrace (dt)      - Discover and trace path");
    println!();
    println!("Messaging:");
    println!("  msg <c> <text>   - Send message (alias: {{)");
    println!("  recv (r)         - Read next message");
    println!("  sync_msgs (sm)   - Get all unread messages");
    println!("  wait_ack (wa, }}) - Wait for ACK");
    println!("  chan <n> <text>  - Send to channel");
    println!();
    println!("Repeaters:");
    println!("  login <c> <pwd>  - Login to repeater");
    println!("  cmd <c> <cmd>    - Send command (alias: [)");
    println!("  wmt8 (])         - Wait 8s for message");
    println!();
    println!("Other:");
    println!("  script <file>    - Run script file");
    println!("  apply_to <f> <c> - Apply commands to filtered contacts");
    println!("  help (?)         - Show this help");
    println!("  quit (q)         - Exit interactive mode");
}
