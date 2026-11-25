//! The chat (interactive) mode.
//!
//! Lines are processed in stages like in the reference:
//!
//! 1. [`process_redirected_line`]: `>`, `>>` and `|` redirections and the
//!    `<key|` substitution,
//! 2. [`process_pipeline`]: `a | b` runs `a` then `b`,
//! 3. [`process_line`]: navigation (`to`), slash commands, replies (`!`),
//!    commands at the root, messages or remote commands in a contact.

mod chat;
mod completion;

use std::collections::HashMap;
use std::sync::Arc;

use rustyline::error::ReadlineError;

pub use chat::{FilterError, apply_command_to_contacts, process_contact_chat_line};

use crate::app::{App, Target, contact_type, to_value, vint, vstr};
use crate::commands::{
    Recipient, SendError, msg_ack, process_cmds, send_chan_msg, subscribe_to_msgs,
};
use crate::editor::{Completion, LineEditor, StyledPrompt};
use crate::output::{self, Sink};
use crate::paths;
use crate::util::ansi;
use crate::util::py;
use crate::util::shlex;
use crate::util::text::{
    FormatErrorKind, SafeFormatter, has_placeholder, send_text, split_pipeline, strip_outer_quotes,
};

/// Where the chat currently is, and where it was before (`to ..`).
#[derive(Debug, Clone, Default)]
pub struct Nav {
    /// Selected destination, `None` at the root (the node itself).
    pub contact: Option<Target>,
    /// Active scope; `None` when the device has no scopes.
    pub scope: Option<String>,
    /// Previous destination.
    pub prev_contact: Option<Target>,
    /// Previous scope.
    pub prev_scope: Option<String>,
}

/// Compares two destinations (same contact key or same channel).
#[must_use]
pub fn same_target(first: Option<&Target>, second: Option<&Target>) -> bool {
    match (first, second) {
        (None, None) => true,
        (Some(Target::Contact(one)), Some(Target::Contact(other))) => {
            one.public_key == other.public_key
        }
        (
            Some(Target::Channel { index, name }),
            Some(Target::Channel {
                index: other_index,
                name: other_name,
            }),
        ) => index == other_index && name == other_name,
        _ => false,
    }
}

impl Nav {
    /// Moves to a new destination, remembering the previous one.
    fn set_contact(&mut self, contact: Option<Target>) {
        if !same_target(contact.as_ref(), self.contact.as_ref()) {
            self.prev_contact = std::mem::replace(&mut self.contact, contact);
        }
    }

    /// Changes the scope, remembering the previous one.
    fn set_scope(&mut self, scope: Option<String>) {
        if scope != self.scope {
            self.prev_scope = std::mem::replace(&mut self.scope, scope);
        }
    }
}

/// Returns an up-to-date copy of a contact target from the device cache.
fn refresh(app: &App, target: Option<&Target>) -> Option<Target> {
    match target {
        Some(Target::Contact(c)) => Some(Target::Contact(Box::new(
            app.mc
                .contact(&c.public_key)
                .unwrap_or_else(|| (**c).clone()),
        ))),
        other => other.cloned(),
    }
}

/// Builds the prompt (node name, destination, path or scope).
fn build_prompt(app: &App, nav: &Nav, last_ack: bool) -> StyledPrompt {
    let settings = app.settings().clone();
    let color = settings.color;
    let classic = settings.classic || !color;
    let mut p = String::new();
    if !classic {
        p.push_str(ansi::INVERT);
    }
    p.push_str(ansi::BGRAY);
    p.push_str(&app.self_name());
    let scope = nav.scope.as_deref().filter(|s| !s.is_empty());
    match refresh(app, nav.contact.as_ref()) {
        None => {
            if let Some(scope) = scope {
                p.push('|');
                p.push_str(scope);
            }
            if classic {
                p.push('>');
            } else {
                p.push_str(ansi::NORMAL);
                p.push_str(&settings.arrow_head);
            }
        }
        Some(target) => {
            if classic {
                p.push('/');
            } else {
                p.push_str(if settings.invert_slash {
                    ansi::INVERT
                } else {
                    ansi::NORMAL
                });
                p.push_str(&settings.slash_start);
            }
            if last_ack {
                p.push_str(match target.kind() {
                    4 => ansi::BYELLOW,
                    3 => ansi::BCYAN,
                    2 => ansi::BMAGENTA,
                    0 => ansi::BGREEN,
                    _ => ansi::BBLUE,
                });
            } else {
                p.push_str(ansi::BRED);
                if classic {
                    p.push('!');
                }
            }
            if !classic {
                p.push_str(&settings.slash_end);
                p.push_str(ansi::INVERT);
            }
            p.push_str(&target.name());
            let contact = target.contact().map(to_value);
            let hops = contact.as_ref().map_or(-1, |c| vint(c, "out_path_len"));
            if target.kind() == 0 || hops == -1 {
                if let Some(scope) = scope {
                    p.push('|');
                    p.push_str(scope);
                }
            } else if hops == 0 {
                p.push_str("|0");
            } else if let Some(contact) = contact {
                // the prompt only shows the first byte of each hop
                let path = vstr(&contact, "out_path");
                let size =
                    usize::try_from(vint(&contact, "out_path_hash_mode") + 1).unwrap_or(1) * 2;
                let short: String = (0..usize::try_from(hops).unwrap_or(0))
                    .filter_map(|i| path.get(i * size..i * size + 2))
                    .collect();
                p.push('|');
                p.push_str(&short);
            }
            if classic {
                p.push_str(ansi::NORMAL);
                p.push('>');
            } else {
                p.push_str(ansi::NORMAL);
                p.push_str(&settings.arrow_head);
            }
            p.push_str(ansi::END);
        }
    }
    p.push(' ');
    StyledPrompt::new(&p, color)
}

/// Expands `@alias args` (`{c}` is the current contact or the first arg).
fn expand_alias(app: &App, nav: &Nav, line: &str) -> Result<String, String> {
    let args = shlex::split(&line[1..]).map_err(|e| e.to_string())?;
    let name = args.first().ok_or("list index out of range")?;
    // the KeyError of ALIASES[name] shows the name as repr
    let alias = app
        .state()
        .alias(name)
        .ok_or_else(|| crate::util::json::py_repr_str(name))?;
    let mut keywords = HashMap::new();
    let positional: Vec<String> = if has_placeholder(&alias, "c").map_err(|e| e.to_string())? {
        if let Some(target) = nav.contact.as_ref().filter(|t| t.kind() > 0) {
            keywords.insert("c".to_string(), target.name());
            args[1..].to_vec()
        } else {
            let c = args.get(1).ok_or("list index out of range")?;
            keywords.insert("c".to_string(), c.clone());
            args[2..].to_vec()
        }
    } else {
        args[1..].to_vec()
    };
    SafeFormatter::format(&alias, &positional, &keywords).map_err(|e| e.to_string())
}

/// Fetches the contact list with a progress display (`get_contacts`).
async fn fetch_contacts_animated(app: &App) {
    if !app.mc.contacts().is_empty() {
        return;
    }
    output::print("Fetching contacts ");
    let mut subscription = app.mc.subscribe();
    if app.mc.get_contacts_async(0).await.is_err() {
        output::print(" Error\n");
        return;
    }
    let mut received = 0usize;
    // the reference waits in windows of 5 s and gives up after one without
    // a contact; other events do not count
    let window = std::time::Duration::from_secs(5);
    let mut window_end = tokio::time::Instant::now() + window;
    let mut activity = false;
    loop {
        match tokio::time::timeout_at(window_end, subscription.recv()).await {
            Ok(Some(meshcore::Event::NextContact(_))) => {
                received += 1;
                activity = true;
                output::print(".");
            }
            Ok(Some(meshcore::Event::Contacts(list))) => {
                let total = to_value(&*list)
                    .get("contacts")
                    .and_then(serde_json::Value::as_array)
                    .map_or(received, Vec::len);
                output::print(&format!(
                    "{} Done\n",
                    ".".repeat(total.saturating_sub(received))
                ));
                return;
            }
            Ok(Some(meshcore::Event::Error(_))) => {
                output::print(" Error\n");
                return;
            }
            Ok(Some(_)) => {}
            Err(_) if activity => {
                activity = false;
                window_end += window;
            }
            Ok(None) | Err(_) => {
                // like the reference, without ending the line
                log::debug!("Timeout while getting contacts");
                return;
            }
        }
    }
}

/// True while the chat loop runs, from where the reference's `try` starts:
/// a Ctrl-C that cancels a command there ends the chat with "Exiting cli"
/// instead of the program's "Exited cleanly".
pub static IN_LOOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Runs the chat mode (`interactive_loop`).
pub async fn interactive_loop(app: &Arc<App>, json_output: bool) {
    output::println(
        "Interactive mode, most commands from terminal chat should work.\n\
         Use \"to\" to select recipient, use Tab to complete name ...\n\
         Some cmds have an help accessible with ?<cmd>. Do ?[Tab] to get a list.\n\
         \"quit\", \"q\", CTRL+D will end interactive mode",
    );

    let mut nav = Nav::default();
    nav.scope = app.set_scope(Some("")).await;
    nav.prev_scope = nav.scope.clone();

    fetch_contacts_animated(app).await;
    app.channels(true).await;

    let mut stdout = Sink::Stdout;
    process_cmds(
        app,
        &["sync_msgs".to_string()],
        json_output,
        &mut stdout,
        "\n",
    )
    .await;
    subscribe_to_msgs(app, json_output).await;
    IN_LOOP.store(true, std::sync::atomic::Ordering::Relaxed);

    if let Ok(vars) = app.mc.get_custom_vars().await
        && let serde_json::Value::Object(map) = to_value(&vars)
    {
        app.state().custom_vars = map.keys().cloned().collect();
    }

    let history = paths::existing_config_dir().map(|_| paths::history_file());
    let mut editor = match LineEditor::new(history) {
        Ok(editor) => editor,
        Err(e) => {
            log::error!("Cannot start line editor: {e}");
            IN_LOOP.store(false, std::sync::atomic::Ordering::Relaxed);
            return;
        }
    };
    let printer = editor.external_printer();
    output::set_external_printer(printer);

    let mut last_ack = true;
    loop {
        // also without a scope, which resets one a failed change left
        nav.scope = app.set_scope(nav.scope.as_deref()).await;

        let prompt = build_prompt(app, &nav, last_ack);
        let tree = completion::make_completion_dict(app, nav.contact.as_ref());
        editor.set_completion(Completion::Nested {
            tree,
            slash_paths: true,
        });

        let Ok((returned, result)) = tokio::task::spawn_blocking(move || {
            let result = editor.readline(&prompt);
            (editor, result)
        })
        .await
        else {
            log::error!("line editor task failed");
            break;
        };
        editor = returned;

        let line = match result {
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => {
                output::print("Exiting cli\n");
                break;
            }
            Err(e) => {
                log::error!("Unexpected error: {e}");
                break;
            }
        };
        // prompt_toolkit keeps the line as typed in the history
        editor.remember(&line);
        let mut line = py::strip(&line).to_string();
        if line.is_empty() {
            continue;
        }
        if let Some(topic) = line.strip_prefix('?') {
            output::print(&crate::help::help_for(topic));
            continue;
        }
        if matches!(line.as_str(), "quit" | "q" | "/quit" | "/q") {
            break;
        }
        if line.starts_with('@') && line.len() > 1 {
            match expand_alias(app, &nav, &line) {
                Ok(expanded) => line = expanded,
                Err(e) => {
                    log::error!("Error {e}");
                    continue;
                }
            }
        }

        let mut json = json_output;
        if let Some(rest) = line.strip_prefix('.') {
            json = true;
            line = py::strip(rest).to_string();
        }

        let mut sink = Sink::Stdout;
        match Box::pin(process_redirected_line(
            app,
            &line,
            nav.clone(),
            json,
            &mut sink,
            "\n",
        ))
        .await
        {
            Ok((contact, scope, ack)) => {
                last_ack = ack;
                nav.set_contact(contact);
                nav.set_scope(scope);
            }
            // the loop's "except Exception": destination, scope and ack stay
            Err(raised) => log::error!("Unexpected error: {}", raised.message()),
        }
        // a Ctrl-C no command handled ends the chat, as in the reference
        if app
            .interrupted
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            output::print("Exiting cli\n");
            break;
        }
    }
    IN_LOOP.store(false, std::sync::atomic::Ordering::Relaxed);
    output::set_external_printer(None);
}

/// Result of processing a line: destination, scope and whether the last
/// message was acknowledged, or the exception that left it.
pub type LineResult = Result<(Option<Target>, Option<String>, bool), Raised>;

/// A Python exception the reference lets leave a chat line: the chat loop
/// reports it as "Unexpected error" and keeps destination, scope and ack,
/// a pipeline stops there, and a redirection or the command handler may
/// catch it first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Raised {
    /// `ValueError`, also from `int()`, `float()` and `shlex`.
    Value(String),
    /// `IndexError`.
    Index(String),
    /// `KeyError`; its text is the repr of the key.
    Key(String),
    /// Any other exception, like a `MemoryError`.
    Other(String),
}

impl Raised {
    /// The text Python shows for the exception.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Value(m) | Self::Index(m) | Self::Key(m) | Self::Other(m) => m,
        }
    }
}

/// Logs why a process for a redirection could not be started; Python
/// names the program in its other `OSError`s.
fn report_spawn_error(e: &std::io::Error, pcom: &str, program: &str) {
    match e.kind() {
        std::io::ErrorKind::NotFound => log::error!("File not found {pcom}"),
        std::io::ErrorKind::PermissionDenied => log::error!("Permission denied: {pcom}"),
        _ => log::error!("Cannot pipe: {}", py::os_error(e, Some(program))),
    }
}

/// Handles redirections at the start of a line (`process_redirected_line`).
pub async fn process_redirected_line(
    app: &Arc<App>,
    line: &str,
    nav: Nav,
    json: bool,
    sink: &mut Sink,
    end: &str,
) -> LineResult {
    let (contact, scope) = (nav.contact.clone(), nav.scope.clone());
    let unchanged = move || -> LineResult { Ok((contact.clone(), scope.clone(), true)) };

    if let Some(rest) = line.strip_prefix(">>").or_else(|| line.strip_prefix('>')) {
        let append = line.starts_with(">>");
        let Ok((file, mcline)) = shlex::split_first_token(py::strip(rest)) else {
            log::error!("Couldn't parse filename");
            return unchanged();
        };
        let path = paths::expand_user(&file);
        let opened = if append {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
        } else {
            std::fs::File::create(&path)
        };
        return match opened {
            Ok(file) => {
                let mut file_sink = Sink::File(file, None);
                let result = Box::pin(process_redirected_line(
                    app,
                    &mcline,
                    nav,
                    json,
                    &mut file_sink,
                    end,
                ))
                .await;
                // the redirection catches a ValueError of the line it runs,
                // and an OSError writing the file
                if let Err(Raised::Value(_)) = result {
                    log::error!("Couldn't parse filename");
                    return unchanged();
                }
                if let Some(e) = file_sink.broken() {
                    let e = py::os_error(e, None);
                    if append {
                        log::error!("Cannot append to file: {e}");
                    } else {
                        log::error!("Cannot write to file: {e}");
                    }
                    return unchanged();
                }
                result
            }
            Err(e) => {
                let e = py::os_error(&e, Some(&path.display().to_string()));
                if append {
                    log::error!("Cannot append to file: {e}");
                } else {
                    log::error!("Cannot write to file: {e}");
                }
                unchanged()
            }
        };
    }

    if let Some(rest) = line.strip_prefix('|') {
        let Ok((pcom, mcline)) = shlex::split_first_token(py::strip(rest)) else {
            log::error!("Couldn't parse name");
            return unchanged();
        };
        let argv = match shlex::split(&pcom) {
            Ok(argv) if !argv.is_empty() => argv,
            // Popen([]) fails on args[0], which this redirection does not catch
            Ok(_) => return Err(Raised::Index("list index out of range".to_string())),
            Err(_) => {
                log::error!("Couldn't parse name");
                return unchanged();
            }
        };
        // Python looks an empty program name up in the PATH directories
        if argv[0].is_empty() {
            report_spawn_error(&crate::handlers::empty_program_error(), &pcom, &argv[0]);
            return unchanged();
        }
        let stdout = match sink.take_stdio() {
            Ok(stdout) => stdout,
            Err(e) => {
                log::error!("Cannot pipe: {e}");
                return unchanged();
            }
        };
        let child = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(stdout)
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(e) => {
                report_spawn_error(&e, &pcom, &argv[0]);
                return unchanged();
            }
        };
        let (result, broken) = match child.stdin.take() {
            Some(stdin) => {
                let mut child_sink = Sink::Child(stdin, None);
                let result = Box::pin(process_redirected_line(
                    app,
                    &mcline,
                    nav,
                    json,
                    &mut child_sink,
                    end,
                ))
                .await;
                let broken = child_sink.broken().map(|e| py::os_error(e, None));
                drop(child_sink);
                (result, broken)
            }
            None => (unchanged(), None),
        };
        let _ = tokio::task::spawn_blocking(move || child.wait()).await;
        // the redirection catches a ValueError of the line it runs, and
        // the BrokenPipeError of a process that stopped reading
        if let Err(Raised::Value(_)) = result {
            log::error!("Couldn't parse name");
            return unchanged();
        }
        if let Some(e) = broken {
            log::error!("Cannot pipe: {e}");
            return unchanged();
        }
        return result;
    }

    if let Some(rest) = line.strip_prefix('<') {
        // the reference's exceptions: IndexError for "<" alone or without
        // "|", ValueError ("Couldn't parse name") for a malformed command
        // or template
        let (key, rest) = if rest.is_empty() {
            log::error!("Error with substitution: string index out of range");
            return unchanged();
        } else if let Some(after) = rest.strip_prefix('|') {
            (String::new(), py::strip(after))
        } else if let Some((key, after)) = rest.split_once('|') {
            (py::strip(key).to_string(), py::strip(after))
        } else {
            log::error!("Error with substitution: list index out of range");
            return unchanged();
        };
        let Ok((pcom, mcline)) = shlex::split_first_token(rest) else {
            log::error!("Couldn't parse name");
            return unchanged();
        };
        let argv = match shlex::split(&pcom) {
            Ok(argv) if !argv.is_empty() => argv,
            // subprocess.run([]) fails on args[0]
            Ok(_) => {
                log::error!("Error with substitution: list index out of range");
                return unchanged();
            }
            Err(_) => {
                log::error!("Couldn't parse name");
                return unchanged();
            }
        };
        if argv[0].is_empty() {
            report_spawn_error(&crate::handlers::empty_program_error(), &pcom, &argv[0]);
            return unchanged();
        }
        // subprocess.run(..., capture_output=True, text=True): stdin stays
        // the terminal's
        let output = match tokio::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::inherit())
            .output()
            .await
        {
            Ok(output) => output,
            Err(e) => {
                report_spawn_error(&e, &pcom, &argv[0]);
                return unchanged();
            }
        };
        // text=True decodes both outputs strictly (a UnicodeDecodeError is a
        // ValueError) and with universal newlines
        let (Ok(stdout), Ok(_)) = (
            String::from_utf8(output.stdout),
            std::str::from_utf8(&output.stderr),
        ) else {
            log::error!("Couldn't parse name");
            return unchanged();
        };
        let stdout = stdout.replace("\r\n", "\n").replace('\r', "\n");
        let replacement = py::strip(&stdout).to_string();
        let formatted = if key.is_empty() {
            SafeFormatter::format(&mcline, &[replacement], &HashMap::new())
        } else {
            let mut keywords = HashMap::new();
            keywords.insert(key, replacement);
            SafeFormatter::format(&mcline, &[], &keywords)
        };
        let result = match formatted {
            Ok(mcline) => {
                Box::pin(process_redirected_line(app, &mcline, nav, json, sink, end)).await
            }
            Err(e) => match e.kind {
                FormatErrorKind::Index => Err(Raised::Index(e.to_string())),
                FormatErrorKind::Value => Err(Raised::Value(e.to_string())),
                FormatErrorKind::Memory => Err(Raised::Other(e.to_string())),
            },
        };
        // what the substitution catches, of its own and of the line it runs
        return match result {
            Err(Raised::Index(e)) => {
                log::error!("Error with substitution: {e}");
                unchanged()
            }
            Err(Raised::Value(_)) => {
                log::error!("Couldn't parse name");
                unchanged()
            }
            other => other,
        };
    }

    Box::pin(process_pipeline(app, line, nav, json, sink, end)).await
}

/// Runs the parts of `a | b | c` one after the other (`process_pipeline`).
pub async fn process_pipeline(
    app: &Arc<App>,
    pipeline: &str,
    mut nav: Nav,
    json: bool,
    sink: &mut Sink,
    end: &str,
) -> LineResult {
    let mut last_ack = true;
    for line in split_pipeline(pipeline) {
        if app.interrupted.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        // an exception stops the pipeline
        let (contact, scope, ack) =
            Box::pin(process_line(app, &line, nav.clone(), json, sink, end)).await?;
        nav.set_contact(contact);
        nav.set_scope(scope);
        last_ack = last_ack && ack;
    }
    Ok((nav.contact, nav.scope, last_ack))
}

/// Parses `name%scope`.
fn split_scope(dest: &str) -> (String, Option<String>) {
    match dest.rsplit_once('%') {
        Some((name, scope)) => (name.to_string(), Some(scope.to_string())),
        None => (dest.to_string(), None),
    }
}

/// Handles `to <dest>`; returns whether the destination changed.
async fn goto(app: &Arc<App>, dest: &str, nav: &mut Nav, sink: &mut Sink) -> Result<bool, Raised> {
    let mut dest = dest.to_string();
    if dest.starts_with('"') || dest.starts_with('\'') {
        // shlex.split(dest)[0]
        match shlex::split(&dest) {
            Ok(words) => dest = words.into_iter().next().unwrap_or_default(),
            Err(e) => return Err(Raised::Value(e.to_string())),
        }
    }
    let mut dest_scope = None;
    if dest.contains('%') && nav.scope.is_some() {
        let (name, scope) = split_scope(&dest);
        dest = name;
        dest_scope = scope;
    } else if let Some(scope) = app.state().scopes.get(&dest) {
        dest_scope = Some(scope.clone());
    }

    let self_name = app.self_name();
    let mut new_contact = app
        .contact_from_arg(&dest)
        .await
        .map(|c| Target::Contact(Box::new(c)));
    if new_contact.is_none() {
        // "ch<n>" is a channel number when int() takes the rest; other
        // names starting with "ch" are looked up by name below
        let channel_number = match dest.strip_prefix("ch").map(|n| py::int(n, 10)) {
            Some(Ok(n)) => Some(n),
            // the reference's mc.channels[n] with a number beyond 64 bits
            Some(Err(e)) if !e.starts_with("invalid literal") => {
                return Err(Raised::Index(
                    "cannot fit 'int' into an index-sized integer".to_string(),
                ));
            }
            _ => None,
        };
        if dest == "public" {
            let name = app
                .cached_channel_name(0)
                .unwrap_or_else(|| "public".to_string());
            new_contact = Some(Target::Channel { name, index: 0 });
        } else if let Some(number) = channel_number {
            // named after the fetched slot, and like the reference's
            // mc.channels[n] an error beyond the slots; a negative number
            // counts from the end, and the slot keeps that number
            let cached = app.state().channels.clone();
            let slot = match cached {
                Some(channels) => {
                    let position = if number < 0 {
                        i64::try_from(channels.len()).unwrap_or(i64::MAX) + number
                    } else {
                        number
                    };
                    usize::try_from(position)
                        .ok()
                        .and_then(|i| channels.get(i))
                        .map(|entry| (entry.name.clone(), number))
                }
                None => Some((format!("chan{number}"), number)),
            };
            let Some((name, index)) = slot else {
                return Err(Raised::Index("list index out of range".to_string()));
            };
            new_contact = Some(Target::Channel { name, index });
        } else if dest == ".." {
            new_contact = nav.prev_contact.clone();
            if dest_scope.is_none() && nav.prev_scope.is_some() {
                dest_scope = nav.prev_scope.clone();
            }
        } else if dest == "~" || dest == "/" || dest == self_name {
            new_contact = None;
        } else if dest == "!" {
            new_contact = app.state().last_node.clone();
        } else if let Some(channel) = app.channel_by_name(&dest).await {
            new_contact = Some(Target::Channel {
                name: channel.name,
                index: i64::from(channel.index),
            });
        } else {
            sink.write(&format!("Contact '{dest}' not found in contacts.\n"));
            new_contact = nav.contact.clone();
        }
    }

    // leaving a channel with an associated scope resets the scope
    if let Some(current) = &nav.contact
        && dest_scope.is_none()
        && let Some(channel_scope) = app.state().scopes.get(&current.name())
        && nav.scope.as_deref() == Some(channel_scope.as_str())
    {
        dest_scope = Some(String::new());
    }
    let dest_scope = dest_scope.or_else(|| nav.scope.clone());
    if nav.scope.is_some() && dest_scope != nav.scope {
        let new_scope = match &dest_scope {
            Some(scope) => app.set_scope(Some(scope)).await,
            None => None,
        };
        nav.set_scope(new_scope);
    }
    let changed = !same_target(new_contact.as_ref(), nav.contact.as_ref());
    if changed {
        nav.set_contact(new_contact);
    }
    Ok(changed)
}

/// Processes one line of the chat (`process_line`).
pub async fn process_line(
    app: &Arc<App>,
    line: &str,
    mut nav: Nav,
    json: bool,
    sink: &mut Sink,
    end: &str,
) -> LineResult {
    let mut last_ack = true;

    if let Some(raw) = line.strip_prefix("$$") {
        match shlex::split(raw) {
            Ok(args) => {
                if !process_cmds(app, &args, json, sink, "\n").await {
                    log::error!(
                        "Problem while executing {}",
                        crate::util::json::py_str_list(&args)
                    );
                }
            }
            Err(_) => log::error!("Error parsing line {}", &line[1..]),
        }
        return Ok((nav.contact, nav.scope, true));
    }

    let mut line = line.trim_end_matches(['\r', '\n']).to_string();
    if line == "~" || line == "/" {
        line = "/to ~".to_string();
    }

    if line.starts_with("/scope") || (line.starts_with("scope") && nav.contact.is_none()) {
        if nav.scope.is_some() {
            if let Some((_, new_scope)) = line.split_once(' ') {
                let scope = app.set_scope(Some(new_scope)).await;
                nav.set_scope(scope);
            } else {
                sink.write(nav.scope.as_deref().unwrap_or_default());
                sink.write("\n");
            }
        }
    } else if (nav.contact.is_none() && (line.starts_with("apply_to ") || line.starts_with("at ")))
        || line.starts_with("/apply_to ")
        || line.starts_with("/at ")
    {
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        let result = if parts.len() == 3 {
            apply_command_to_contacts(app, parts[1], parts[2], json, sink, end).await
        } else {
            Err(FilterError::Index)
        };
        // the reference catches an IndexError here, also one of the
        // commands run on the contacts; other exceptions leave the line
        match result {
            Err(FilterError::Index | FilterError::Raised(Raised::Index(_))) => {
                log::error!("Error with apply_to command parameters");
            }
            Err(FilterError::Value(e)) => return Err(Raised::Value(e)),
            Err(FilterError::Raised(raised)) => return Err(raised),
            Ok(()) | Err(FilterError::Logged(_)) => {}
        }
    } else if let Some(dest) = line
        .strip_prefix("to ")
        .or_else(|| line.strip_prefix("/to "))
    {
        if goto(app, dest, &mut nav, sink).await? {
            last_ack = true;
        }
    } else if line == "to" || line == "/to" {
        let name = nav
            .contact
            .as_ref()
            .map_or_else(|| app.self_name(), Target::name);
        sink.write(&format!("{name}\n"));
    } else if line.starts_with('/') {
        last_ack = process_slash_cmd(app, &line, nav.scope.clone(), json, sink, end).await?;
    } else if let Some(text) = line.strip_prefix("public ") {
        let args = vec!["public".to_string(), text.to_string()];
        process_cmds(app, &args, json, sink, "\n").await;
    } else if let Some(text) = line.strip_prefix('!') {
        let last = app.state().last_node.clone();
        match last {
            None => sink.write("No received msg yet !\n"),
            Some(Target::Channel { index, .. }) => {
                chat_chan_msg(app, channel_slot(index)?, text, sink, end).await?;
            }
            Some(Target::Contact(contact)) => {
                last_ack =
                    chat_msg(app, &Recipient::Contact(contact.clone()), text, sink, end).await?;
                if !last_ack {
                    nav.set_contact(Some(Target::Contact(contact)));
                }
            }
        }
    } else if nav.contact.is_none() || line.starts_with('.') {
        if let Some(cli) = line.strip_prefix(':') {
            let args = vec!["cli".to_string(), py::strip(cli).to_string()];
            if !process_cmds(app, &args, json, sink, end).await {
                log::error!(
                    "Problem while running {}",
                    crate::util::json::py_str_list(&args)
                );
            }
        } else {
            match shlex::split(&line) {
                Ok(args) => {
                    let mut ok = process_cmds(app, &args, json, sink, end).await;
                    let mut ran = args;
                    if !ok && app.has_cli() {
                        ran = vec!["cli".to_string(), py::strip(&line).to_string()];
                        ok = process_cmds(app, &ran, json, sink, end).await;
                    }
                    if !ok {
                        log::error!(
                            "Problem while running {}",
                            crate::util::json::py_str_list(&ran)
                        );
                    }
                }
                Err(_) => log::error!("Error processing {line}"),
            }
        }
    } else if let Some(target) = refresh(app, nav.contact.as_ref()) {
        last_ack = chat_line(app, &target, &line, json, sink, end).await?;
    }
    sink.flush();
    Ok((nav.contact, nav.scope, last_ack))
}

/// Sends a message from the chat and waits for its ack. A text the device
/// cannot take is reported; the reference sends it and fails silently.
///
/// # Errors
///
/// A message log that cannot be written, which the reference raises.
pub(crate) async fn chat_msg(
    app: &App,
    to: &Recipient,
    text: &str,
    sink: &mut Sink,
    end: &str,
) -> Result<bool, Raised> {
    match msg_ack(app, to, text).await {
        Ok(acked) => Ok(acked),
        Err(SendError::Send(e)) => {
            sink.write(&format!("Error sending message: {e}{end}"));
            Ok(false)
        }
        Err(SendError::Log(e)) => Err(Raised::Other(e)),
    }
}

/// The slot of a channel target, packed like `meshcore_py`'s `to_bytes(1)`
/// when a message is sent to it.
fn channel_slot(index: i64) -> Result<u8, Raised> {
    u8::try_from(index).map_err(|_| {
        Raised::Other(if index < 0 {
            "can't convert negative int to unsigned".to_string()
        } else {
            "int too big to convert".to_string()
        })
    })
}

/// Sends a channel message from the chat. As in the reference, failures go
/// unreported, except a text the device cannot take, which the reference
/// would send cut short.
///
/// # Errors
///
/// A message log that cannot be written, which the reference raises.
async fn chat_chan_msg(
    app: &App,
    index: u8,
    text: &str,
    sink: &mut Sink,
    end: &str,
) -> Result<(), Raised> {
    match send_chan_msg(app, index, text).await {
        Err(SendError::Send(e @ meshcore::Error::InvalidArgument(_))) => {
            sink.write(&format!("Error sending message: {e}{end}"));
            Ok(())
        }
        Err(SendError::Log(e)) => Err(Raised::Other(e)),
        Ok(_) | Err(SendError::Send(_)) => Ok(()),
    }
}

/// A line typed while a contact or channel is selected.
async fn chat_line(
    app: &Arc<App>,
    target: &Target,
    line: &str,
    json: bool,
    sink: &mut Sink,
    end: &str,
) -> Result<bool, Raised> {
    if let Some(contact) = target.contact()
        && process_contact_chat_line(app, contact, line, false, json, sink, end).await?
    {
        return Ok(true);
    }
    if line == "list" {
        let names: Vec<String> = app.mc.contacts().iter().map(|c| c.name.clone()).collect();
        sink.write(&format!("{}\n", names.join(", ")));
        return Ok(true);
    }
    let (explicit, text) = if let Some(text) = send_text(line) {
        (true, strip_outer_quotes(text).to_string())
    } else if line.starts_with('"') {
        (true, strip_outer_quotes(line).to_string())
    } else {
        (false, strip_outer_quotes(line).to_string())
    };
    match target {
        Target::Channel { index, .. } => {
            // the reference strips the quotes of an explicit message twice
            let text = if explicit {
                strip_outer_quotes(&text)
            } else {
                &text
            };
            chat_chan_msg(app, channel_slot(*index)?, text, sink, end).await?;
            Ok(true)
        }
        Target::Contact(contact) => match contact_type(contact) {
            // the reference takes a contact of type 0 for a channel and
            // fails on its missing channel number
            0 => Err(Raised::Key("'chan_nb'".to_string())),
            kind if explicit || kind == 1 => {
                chat_msg(app, &Recipient::Contact(contact.clone()), &text, sink, end).await
            }
            2..=4 => {
                let args = vec!["cmd".to_string(), contact.name.clone(), line.to_string()];
                process_cmds(app, &args, json, sink, end).await;
                Ok(true)
            }
            // a type the reference does not know: the line is dropped
            _ => Ok(true),
        },
    }
}

/// Handles lines starting with `/` (`process_slash_cmd`).
async fn process_slash_cmd(
    app: &Arc<App>,
    line: &str,
    scope: Option<String>,
    json: bool,
    sink: &mut Sink,
    end: &str,
) -> Result<bool, Raised> {
    let path = line.split(' ').next().unwrap_or_default();
    // back to the scope of the line, also to none (see App::set_scope)
    let restore = |app: Arc<App>, scope: Option<String>| async move {
        app.set_scope(scope.as_deref()).await;
    };
    let mut result = true;

    if path.matches('/').count() == 1 {
        let args: Vec<&str> = line[1..].split(' ').collect();
        let (dest, dest_scope) = split_scope(args[0]);
        if let Some(dest_scope) = dest_scope {
            app.set_scope(Some(&dest_scope)).await;
        }
        let rest = line.split_once(' ').map(|(_, r)| r.to_string());
        let contact = app.contact_from_arg(&dest).await;
        if let (Some(text), Some(contact)) = (&rest, contact) {
            if matches!(contact_type(&contact), 1 | 3) {
                // the reference drops the ack here, the prompt stays as it is
                chat_msg(app, &Recipient::Contact(Box::new(contact)), text, sink, end).await?;
            } else {
                sink.write("Can only send msg to chan, client or room\n");
            }
        } else if let (Some(text), Some(channel)) = (&rest, app.channel_by_name(&dest).await) {
            chat_chan_msg(app, channel.index, text, sink, end).await?;
        } else if let Ok(cmds) = shlex::split(&line[1..]) {
            process_cmds(app, &cmds, json, sink, "\n").await;
        } else {
            log::error!("Error processing line {}", &line[1..]);
        }
    } else {
        let cmdline = line[1..]
            .split_once('/')
            .map(|(_, c)| c.to_string())
            .unwrap_or_default();
        let (name, dest_scope) = split_scope(path[1..].split('/').next().unwrap_or_default());
        if let Some(dest_scope) = dest_scope {
            app.set_scope(Some(&dest_scope)).await;
        }
        match app.contact_from_arg(&name).await {
            None => sink.write(&format!("{name} is not a contact\n")),
            Some(contact) => {
                // both run with their own end of line in the reference; an
                // exception leaves the scope of the line as it is
                if !process_contact_chat_line(app, &contact, &cmdline, false, json, sink, "\n")
                    .await?
                    && !cmdline.is_empty()
                {
                    if contact_type(&contact) == 1 {
                        result = chat_msg(
                            app,
                            &Recipient::Contact(Box::new(contact)),
                            &cmdline,
                            sink,
                            end,
                        )
                        .await?;
                    } else {
                        let args = vec!["cmd".to_string(), contact.name.clone(), cmdline];
                        process_cmds(app, &args, json, sink, "\n").await;
                    }
                }
            }
        }
    }
    restore(app.clone(), scope).await;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_in_destinations() {
        assert_eq!(
            split_scope("Alice%#region"),
            ("Alice".to_string(), Some("#region".to_string()))
        );
        assert_eq!(split_scope("Alice"), ("Alice".to_string(), None));
        assert_eq!(
            split_scope("a%b%c"),
            ("a%b".to_string(), Some("c".to_string()))
        );
    }

    #[test]
    fn channel_targets_compare_by_slot() {
        let a = Target::Channel {
            name: "x".into(),
            index: 1,
        };
        let b = Target::Channel {
            name: "x".into(),
            index: 1,
        };
        let c = Target::Channel {
            name: "x".into(),
            index: 2,
        };
        assert!(same_target(Some(&a), Some(&b)));
        assert!(!same_target(Some(&a), Some(&c)));
        assert!(same_target(None, None));
        assert!(!same_target(Some(&a), None));
    }
}
