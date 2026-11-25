//! The command engine (`next_cmd` / `process_cmds` of the reference).
//!
//! Commands are given as a list of words. Each command consumes its
//! arguments and the remaining words are processed as the next command, so
//! commands can be chained on the command line (`msg Alice hi wa`), in
//! scripts and in the chat mode.

mod channels;
mod contacts;
mod device;
mod getset;
mod messaging;
mod misc;
mod remote;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::Value;

use crate::app::App;
use crate::output::Sink;
use crate::util::json::{dumps, dumps_indent};

pub use contacts::contact_path_str;
pub use messaging::{Recipient, msg_ack, send_chan_msg, subscribe_to_msgs};
pub use misc::seconds;
pub use remote::{print_disc_trace_to, print_trace_to};

/// Why a command stopped.
#[derive(Debug)]
pub enum CmdError {
    /// Missing or malformed arguments (`IndexError` in the reference).
    Params,
    /// The user cancelled a prompt.
    Cancelled,
    /// Not a command; the caller may try something else (e.g. `cli`).
    Unknown,
    /// Any other failure.
    Failed(String),
}

impl From<meshcore::Error> for CmdError {
    fn from(e: meshcore::Error) -> Self {
        Self::Failed(e.to_string())
    }
}

/// The payload of `meshcore_py`'s error event for a failed request, which
/// the reference prints or merges into its output in a few places.
#[must_use]
pub fn py_error_payload(error: &meshcore::Error) -> Option<Value> {
    match error {
        meshcore::Error::Device(code) => {
            let mut map = serde_json::Map::new();
            map.insert("error_code".into(), Value::from(code.code()));
            if let Some(name) = code.name() {
                map.insert("code_string".into(), Value::from(name));
            }
            Some(Value::Object(map))
        }
        meshcore::Error::Timeout => Some(serde_json::json!({"reason": "no_event_received"})),
        _ => None,
    }
}

/// The next option word of a command: `args[0]` when it starts with "-",
/// like the reference's `while args[0][0] == "-"`, for which an empty word
/// is an `IndexError`.
pub fn next_option(args: &[String]) -> Result<Option<&String>, CmdError> {
    match args.first() {
        Some(arg) if arg.is_empty() => Err(CmdError::Params),
        Some(arg) if arg.starts_with('-') => Ok(Some(arg)),
        _ => Ok(None),
    }
}

/// Number of arguments consumed by a command (`argnum`).
pub type CmdResult = Result<usize, CmdError>;

/// Everything a command needs while it runs.
pub struct Ctx<'a> {
    /// Application state.
    pub app: &'a Arc<App>,
    /// The command word and all following words.
    pub cmds: &'a [String],
    /// Json output requested.
    pub json: bool,
    /// Line terminator for results.
    pub end: &'a str,
    /// Where output goes.
    pub sink: &'a mut Sink,
    /// Collected output, written to the sink when the command is done.
    pub out: String,
}

impl Ctx<'_> {
    /// Argument `i` (0 is the command itself).
    pub fn arg(&self, i: usize) -> Result<&str, CmdError> {
        self.cmds.get(i).map(String::as_str).ok_or(CmdError::Params)
    }

    /// True if argument `i` exists.
    #[must_use]
    pub fn has(&self, i: usize) -> bool {
        i < self.cmds.len()
    }

    /// Appends text to the output.
    pub fn emit(&mut self, text: &str) {
        self.out.push_str(text);
    }

    /// Appends text followed by the line terminator.
    pub fn line(&mut self, text: &str) {
        self.out.push_str(text);
        self.out.push_str(self.end);
    }

    /// Appends `json.dumps(value)` and the terminator.
    pub fn json_line(&mut self, value: &Value) {
        let text = dumps(value);
        self.line(&text);
    }

    /// Appends `json.dumps(value, indent=4)` and the terminator.
    pub fn json_indent(&mut self, value: &Value) {
        let text = dumps_indent(value, 4);
        self.line(&text);
    }

    /// Prints `{"error": ..}` in json mode, the text otherwise.
    pub fn error(&mut self, json: &Value, text: &str) {
        if self.json {
            self.json_line(json);
        } else {
            self.line(text);
        }
    }

    /// Reports an unknown contact the way most commands do.
    pub fn unknown_contact(&mut self, name: &str) {
        let json = serde_json::json!({"error": "contact unknown", "name": name});
        let text = format!("Unknown contact {name}");
        self.error(&json, &text);
    }

    /// Writes the collected output to the sink.
    pub fn flush_out(&mut self) {
        let out = std::mem::take(&mut self.out);
        self.sink.write(&out);
        self.sink.flush();
    }
}

/// Result of [`next_cmd`].
enum Next {
    /// Continue after this many words.
    Consumed(usize),
    /// Stop silently (error already reported).
    Abort,
    /// The word was not a command.
    Unknown,
}

/// Runs the first command of `cmds` (`next_cmd`).
async fn next_cmd(
    app: &Arc<App>,
    cmds: &[String],
    json: bool,
    sink: &mut Sink,
    end: &str,
) -> (Next, String) {
    let first = cmds[0].as_str();
    if let Some(topic) = first.strip_prefix('?') {
        // printed like the reference does, past any redirection
        crate::output::print(&crate::help::help_for(topic));
        return (Next::Consumed(1), String::new());
    }
    let (json, name) = match first.strip_prefix('.') {
        Some(rest) => (true, rest.to_string()),
        None => (json, first.to_string()),
    };

    let mut ctx = Ctx {
        app,
        cmds,
        json,
        end,
        sink,
        out: String::new(),
    };
    let result = dispatch(&mut ctx, &name).await;
    // Ctrl-C at a prompt deeper down: next_cmd catches KeyboardInterrupt
    if app.interrupted.swap(false, Ordering::Relaxed) {
        log::error!("Cancelled");
        return (Next::Abort, String::new());
    }
    match result {
        Ok(argnum) => {
            log::debug!(
                "cmd {} processed ...",
                crate::util::json::py_str_list(&cmds[..(argnum + 1).min(cmds.len())])
            );
            (Next::Consumed(argnum + 1), ctx.out)
        }
        Err(CmdError::Unknown) => {
            log::debug!(
                "Unknown command : {name}, {} not executed ...",
                crate::util::json::py_str_list(cmds)
            );
            (Next::Unknown, ctx.out)
        }
        Err(CmdError::Params) => {
            log::error!("Error in parameters");
            (Next::Abort, String::new())
        }
        Err(CmdError::Cancelled) => {
            log::error!("Cancelled");
            (Next::Abort, String::new())
        }
        Err(CmdError::Failed(e)) => {
            log::error!("Unexpected error while processing '{first}': {e}");
            (Next::Abort, String::new())
        }
    }
}

/// Selects the implementation of a command.
async fn dispatch(c: &mut Ctx<'_>, name: &str) -> CmdResult {
    match name {
        "help" => {
            // print() in the reference, past any redirection
            crate::output::println(crate::help::command_help());
            Ok(0)
        }
        "ver" | "query" | "v" | "q" => device::ver(c).await,
        "clock" => device::clock(c).await,
        "sync_time" | "clock sync" | "st" => device::sync_time(c).await.map(|()| 0),
        "time" => device::time(c).await,
        "apply_to" | "at" => {
            let filter = c.arg(1)?.to_string();
            let line = c.arg(2)?.to_string();
            let (json, end) = (c.json, c.end.to_string());
            c.flush_out();
            match crate::interactive::apply_command_to_contacts(
                c.app, &filter, &line, json, c.sink, &end,
            )
            .await
            {
                Err(
                    crate::interactive::FilterError::Index
                    | crate::interactive::FilterError::Raised(crate::interactive::Raised::Index(_)),
                ) => Err(CmdError::Params),
                Err(crate::interactive::FilterError::Value(e)) => Err(CmdError::Failed(e)),
                // the exception of a command run on a contact, as next_cmd
                // reports it
                Err(crate::interactive::FilterError::Raised(raised)) => {
                    Err(CmdError::Failed(raised.message().to_string()))
                }
                Ok(()) | Err(crate::interactive::FilterError::Logged(_)) => Ok(2),
            }
        }
        "set" => getset::set(c).await,
        "get" => getset::get(c).await,
        "self_telemetry" | "t" => device::self_telemetry(c).await,
        "get_channel" => channels::get_channel(c).await,
        "get_channels" | "gc" => channels::get_channels(c).await,
        "set_channel" => channels::set_channel(c).await,
        "add_channel" => channels::add_channel(c).await,
        "remove_channel" => channels::remove_channel(c).await,
        "scope" => channels::scope(c).await,
        "reboot" => device::reboot(c).await,
        "msg" | "m" | "{" => messaging::msg(c).await,
        "chan" | "ch" => messaging::chan(c).await,
        "chan_data" => messaging::chan_data(c).await,
        "public" | "dch" => messaging::public(c).await,
        "cmd" | "c" | "[" => messaging::cmd(c).await,
        "trace" | "tr" => remote::trace(c).await,
        "login" | "l" => remote::login(c).await,
        "logout" => remote::logout(c).await,
        "contact_timeout" => contacts::contact_timeout(c).await,
        "disc_path" | "dp" => remote::disc_path(c).await,
        "node_discover" | "nd" => remote::node_discover(c).await,
        "list_discovered" => Ok(remote::list_discovered(c)),
        "add_discovered" => remote::add_discovered(c).await,
        "req_regions" | "rr" => remote::req_regions(c).await,
        "req_owner" | "ro" => remote::req_owner(c).await,
        "req_clock" => remote::req_clock(c).await,
        "req_telemetry" | "rt" => remote::req_telemetry(c).await,
        "req_status" | "rs" => remote::req_status(c).await,
        "req_mma" | "rm" => remote::req_mma(c).await,
        "req_acl" | "ra" => remote::req_acl(c).await,
        "req_neighbours" | "rn" => remote::req_neighbours(c).await,
        "req_binary" => remote::req_binary(c).await,
        "contacts" | "list" | "lc" | "reload_contacts" | "rc" => {
            contacts::list(c, matches!(name, "reload_contacts" | "rc")).await
        }
        "pending_contacts" => Ok(contacts::pending_contacts(c)),
        "flush_pending" => {
            c.app.mc.flush_pending_contacts();
            Ok(0)
        }
        "add_pending" => contacts::add_pending(c).await,
        "path" => contacts::path(c).await,
        "contact_info" | "ci" => contacts::contact_info(c).await,
        "add_contact" => contacts::add_contact(c).await,
        "reset_advert_time" => contacts::reset_advert_time(c).await,
        "change_path" | "cp" => contacts::change_path(c).await,
        "change_flags" | "cf" => contacts::change_flags(c).await,
        "reset_path" | "rp" => contacts::reset_path(c).await,
        "advert_path" | "ap" => contacts::advert_path(c).await,
        "share_contact" | "sc" => contacts::share_contact(c).await,
        "export_contact" | "ec" => contacts::export_contact(c).await,
        "import_contact" | "ic" => contacts::import_contact(c).await,
        "upload_contact" | "uc" => contacts::upload_contact(c).await,
        "card" => device::card(c).await,
        "upload_card" => device::upload_card(c).await,
        "remove_contact" => contacts::remove_contact(c).await,
        "recv" | "r" => messaging::recv(c).await,
        "sync_msgs" | "sm" => messaging::sync_msgs(c).await,
        "infos" | "i" => device::infos(c).await,
        "advert" | "a" => device::advert(c, false).await,
        "flood_advert" | "floodadv" => device::advert(c, true).await,
        "cli" => device::cli(c).await,
        "send_raw" => device::send_raw(c).await,
        "sleep" | "s" => misc::sleep(c).await,
        "wait_key" | "wk" => misc::wait_key(c).await,
        "wait_msg" | "wm" => messaging::wait_msg(c, None).await.map(|()| 0),
        "trywait_msg" | "wmt" => {
            let secs = misc::seconds(c.arg(1)?)?;
            messaging::wait_msg(c, Some(secs)).await.map(|()| 1)
        }
        "wmt8" | "]" => messaging::wait_msg(c, Some(8.0)).await.map(|()| 0),
        "wait_ack" | "wa" | "}" => messaging::wait_ack(c).await,
        "msgs_subscribe" | "ms" => {
            messaging::subscribe_to_msgs(c.app, c.json).await;
            Ok(0)
        }
        "echo" => {
            let text = c.arg(1)?.to_string();
            c.line(&text);
            Ok(1)
        }
        "script" => misc::script(c).await,
        "alias" => misc::alias(c),
        "aliases" => {
            let value = c.app.state().aliases_value();
            c.json_indent(&value);
            Ok(0)
        }
        "aliases_load" => misc::aliases_load(c).await,
        "handler_attach" => misc::handler_attach(c),
        "handler_detach" => misc::handler_detach(c).await,
        "handlers" => {
            let value = c.app.handlers.list();
            c.json_line(&value);
            Ok(0)
        }
        // kept for compatibility; chat mode is entered with -i
        "chat" | "im" | "interactive" => Ok(0),
        _ => Err(CmdError::Unknown),
    }
}

/// Runs a list of commands (`process_cmds`).
///
/// Returns false when a word was not a command.
pub async fn process_cmds(
    app: &Arc<App>,
    cmds: &[String],
    json: bool,
    sink: &mut Sink,
    end: &str,
) -> bool {
    let mut pos = 0;
    while pos < cmds.len() {
        let (next, output) = Box::pin(next_cmd(app, &cmds[pos..], json, sink, end)).await;
        sink.write(&output);
        sink.flush();
        // a broken pipe to a process ends the commands, the `|` redirection
        // reports it
        if sink.broken().is_some() {
            return true;
        }
        match next {
            Next::Consumed(n) => pos += n,
            Next::Abort => return true,
            Next::Unknown => return false,
        }
    }
    true
}

/// Runs a script file (`process_script`); missing files are looked up in
/// the configuration directory.
///
/// # Errors
///
/// A file that cannot be read gives Python's `OSError` message.
pub async fn process_script(
    app: &Arc<App>,
    file: &str,
    json: bool,
    sink: &mut Sink,
) -> Result<String, String> {
    let mut output = String::new();
    let file = crate::util::py::strip(file);
    let Some(path) = crate::paths::resolve_script(file) else {
        log::info!("file {file} not found");
        if json {
            output.push_str(&dumps(
                &serde_json::json!({"error": format!("file {file} not found")}),
            ));
            output.push('\n');
        }
        return Ok(output);
    };
    run_script_file(app, &path, json, sink).await?;
    Ok(output)
}

async fn run_script_file(
    app: &Arc<App>,
    path: &Path,
    json: bool,
    sink: &mut Sink,
) -> Result<(), String> {
    // f.readlines() in the reference
    let content = crate::util::py::read_text(path, crate::util::py::TextRead::Lines)
        .map_err(|e| e.message(&path.display().to_string()))?;
    for line in crate::util::py::lines(&content) {
        let line = crate::util::py::strip(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        log::debug!("processing {line}");
        match crate::util::shlex::split(line) {
            Ok(cmds) => {
                if !Box::pin(process_cmds(app, &cmds, json, sink, "\n")).await {
                    log::error!(
                        "Problem executing {}",
                        crate::util::json::py_str_list(&cmds)
                    );
                }
            }
            Err(_) => log::error!("Error processing {line}"),
        }
    }
    Ok(())
}
