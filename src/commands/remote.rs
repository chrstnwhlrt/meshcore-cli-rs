//! Commands for remote nodes: login, requests, trace, discovery.

use std::fmt::Write as _;
use std::time::Duration;

use meshcore::{Contact, Event, PublicKey, TraceRequest};
use serde_json::{Value, json};

use super::{CmdError, CmdResult, Ctx};
use crate::app::{App, to_value, vint, vstr};
use crate::display::{py_number, terminal_width, type_name};
use crate::output::Sink;
use crate::util::ansi;
use crate::util::text::{pad_left, pad_right};

/// Why a trace argument was rejected.
#[derive(Debug, PartialEq, Eq)]
pub enum TraceArgError {
    /// The reference CLI fails with an exception (`Unexpected error ...`).
    Command(String),
    /// `meshcore_py` rejects the path (`Invalid path format: ...` is logged).
    Path(String),
}

/// Splits a trace argument into path bytes and flags.
///
/// `6a,61` and `6a83,6144` infer the hash size (1, 2, 4 or 8 bytes) from
/// the first token, `6a836144:1` gives the flags explicitly and `6a61`
/// uses one byte hashes, like the reference and `meshcore_py` do.
///
/// # Errors
///
/// Returns the reason in the form the reference reports it.
pub fn parse_trace_arg(arg: &str) -> Result<(Vec<u8>, u8), TraceArgError> {
    use crate::util::py;
    if arg.contains(',') {
        let first = arg.split(',').next().unwrap_or_default();
        let hash_len = first.chars().count() / 2;
        let flags = match hash_len {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            n => {
                return Err(TraceArgError::Path(format!("unknown path_hash_len {n}")));
            }
        };
        let mut bytes = Vec::new();
        for token in arg.split(',') {
            let token = py::strip(token);
            if token.is_empty() {
                break;
            }
            if token.chars().count() != hash_len * 2 {
                return Err(TraceArgError::Path(String::new()));
            }
            bytes.extend(py::fromhex(token).map_err(TraceArgError::Path)?);
        }
        return Ok((bytes, flags));
    }
    let mut parts = arg.split(':');
    let hops = parts.next().unwrap_or_default();
    let flags = parts
        .next()
        .map(|flags| py::int(flags, 10))
        .transpose()
        .map_err(TraceArgError::Command)?;
    let bytes = py::fromhex(hops).map_err(TraceArgError::Command)?;
    // checked last, when meshcore_py appends the flags to its bytearray
    let flags = flags.map_or(Ok(0), |flags| {
        u8::try_from(flags)
            .map_err(|_| TraceArgError::Command("byte must be in range(0, 256)".to_string()))
    })?;
    Ok((bytes, flags))
}

/// Formats trace results (`→snr→[hash]→snr→`).
fn format_trace(app: &App, trace: &Value, end: &str) -> String {
    let settings = app.settings().clone();
    let classic = settings.classic || !settings.color;
    let mut out = String::from(" ");
    if let Some(Value::Array(hops)) = trace.get("path") {
        for hop in hops {
            out.push_str(if classic { "→" } else { ansi::INVERT });
            let snr = hop.get("snr").and_then(Value::as_f64).unwrap_or(0.0);
            if settings.color {
                out.push_str(if snr >= 10.0 {
                    ansi::BGREEN
                } else if snr <= 0.0 {
                    ansi::BRED
                } else {
                    ansi::BGRAY
                });
            }
            let _ = write!(out, "{snr:.2}");
            if classic {
                out.push('→');
            } else {
                out.push_str(ansi::NORMAL);
                out.push_str(&settings.arrow_head);
            }
            if settings.color {
                out.push_str(ansi::END);
            }
            if let Some(hash) = hop.get("hash").and_then(Value::as_str) {
                let _ = write!(out, "[{hash}]");
            }
        }
    }
    out.push_str(end);
    out
}

/// `trace <path>`.
pub async fn trace(c: &mut Ctx<'_>) -> CmdResult {
    let arg = c.arg(1)?.to_string();
    let (path, flags) = match parse_trace_arg(&arg) {
        Ok(parsed) => parsed,
        Err(TraceArgError::Command(e)) => return Err(CmdError::Failed(e)),
        Err(TraceArgError::Path(e)) => {
            log::error!("Invalid path format: {e}");
            return Ok(1);
        }
    };
    let request = TraceRequest {
        auth_code: 0,
        tag: None,
        flags: Some(flags),
        path,
    };
    match c
        .app
        .mc
        .trace_sync(&request, meshcore::RequestOptions::default())
        .await
    {
        Ok(trace) => {
            let trace = to_value(&trace);
            if c.json {
                let text = crate::util::json::dumps_indent(&trace, 2);
                c.line(&text);
            } else {
                let text = format_trace(c.app, &trace, c.end);
                c.emit(&text);
            }
        }
        Err(meshcore::Error::Timeout) => {
            let text = format!("Timeout waiting trace for path {arg}");
            c.error(&json!({"error": "timeout waiting trace"}), &text);
        }
        // an empty path, which the firmware does not trace
        Err(meshcore::Error::InvalidArgument(e)) => log::error!("Invalid path format: {e}"),
        Err(e) => log::debug!("trace failed: {e}"),
    }
    Ok(1)
}

/// Builds the trace string to a contact along its stored path
/// (`print_trace_to`): out to the destination and back.
#[must_use]
pub fn trace_path_to(contact: &Value) -> Option<String> {
    let hops = usize::try_from(vint(contact, "out_path_len")).ok()?;
    let path = vstr(contact, "out_path");
    let mut size = usize::try_from(vint(contact, "out_path_hash_mode") + 1).unwrap_or(1);
    let elems: Vec<String> = (0..hops)
        .map(|i| {
            let start = i * size * 2;
            let hop = path.get(start..start + size * 2).unwrap_or_default();
            // three byte hashes are traced with their first two bytes
            hop.chars().take(4).collect()
        })
        .collect();
    if size == 3 {
        size = 2;
    }
    let mut trace = String::new();
    let kind = vint(contact, "type");
    if kind == 2 || kind == 3 {
        trace = vstr(contact, "public_key").chars().take(2 * size).collect();
    }
    for elem in elems.iter().rev() {
        trace = if trace.is_empty() {
            elem.clone()
        } else {
            format!("{elem}{trace}{elem}")
        };
    }
    if size >= 2 {
        trace.push_str(":1");
    }
    Some(trace)
}

/// Traces the path to a contact (`trace` inside a contact).
pub async fn print_trace_to(
    app: &std::sync::Arc<App>,
    contact: &Contact,
    json: bool,
    sink: &mut Sink,
    end: &str,
) {
    let value = to_value(contact);
    match trace_path_to(&value) {
        None => {
            if json {
                sink.write(&format!(
                    "{}{end}",
                    crate::util::json::dumps(&json!({"error": "No path to destination"}))
                ));
            } else {
                sink.write(&format!("No path to destination{end}"));
            }
            sink.flush();
        }
        Some(trace) => {
            let cmds = vec!["trace".to_string(), trace];
            Box::pin(super::process_cmds(app, &cmds, json, sink, end)).await;
        }
    }
}

/// Builds a trace from a path discovery result (`print_disc_trace_to`).
#[must_use]
pub fn trace_from_discovery(discovery: &Value, contact: &Value) -> String {
    let hops = |path_key: &str, len_key: &str| -> Vec<String> {
        let path = vstr(discovery, path_key);
        let size = usize::try_from(vint(discovery, len_key))
            .unwrap_or(1)
            .max(1);
        path.as_bytes()
            .chunks(size * 2)
            .map(|chunk| String::from_utf8_lossy(chunk).chars().take(4).collect())
            .collect()
    };
    let mut trace: Vec<String> = hops("out_path", "out_path_hash_len");
    let kind = vint(contact, "type");
    if kind == 2 || kind == 3 {
        let size = trace.first().map_or(2, String::len);
        trace.push(vstr(contact, "public_key").chars().take(size).collect());
    }
    for elem in hops("in_path", "in_path_hash_len") {
        if trace.last() != Some(&elem) {
            trace.push(elem);
        }
    }
    let mut text = trace.join(",");
    if trace.len() == 1 && text.len() > 2 {
        text.push(',');
    }
    text
}

/// Discovers the path to a contact and traces it (`dtrace`).
pub async fn print_disc_trace_to(
    app: &std::sync::Arc<App>,
    contact: &Contact,
    json: bool,
    sink: &mut Sink,
    end: &str,
) {
    let options = app.request_options(&contact.public_key);
    match app
        .mc
        .send_path_discovery_sync(&contact.public_key, options)
        .await
    {
        // the reference reads every failure as a timeout here
        Err(_) => log::error!("Timeout discovering path"),
        Ok(discovery) => {
            let trace = trace_from_discovery(&to_value(&discovery), &to_value(contact));
            log::info!("Trying {trace}");
            let cmds = vec!["trace".to_string(), trace];
            Box::pin(super::process_cmds(app, &cmds, json, sink, end)).await;
        }
    }
}

/// Reads a password, showing stars for it; Ctrl-C and Ctrl-D cancel.
async fn read_password(prompt: &str) -> Result<String, CmdError> {
    let prompt = prompt.to_string();
    tokio::task::spawn_blocking(move || crate::editor::read_password(&prompt))
        .await
        .map_err(|e| CmdError::Failed(e.to_string()))?
        .map_err(|_| CmdError::Cancelled)
}

/// `login <name> <password>` (`$` asks for the password).
pub async fn login(c: &mut Ctx<'_>) -> CmdResult {
    let name = c.arg(1)?.to_string();
    let Some(contact) = c.app.contact_from_arg(&name).await else {
        c.unknown_contact(&name);
        return Ok(2);
    };
    // read once the contact is known, as in the reference
    let mut password = c.arg(2)?.to_string();
    if password == "$" {
        password = read_password("Password: ").await?;
    }
    login_contact(c, &contact, &password).await;
    Ok(2)
}

/// Logs into a contact and prints the result.
pub async fn login_contact(c: &mut Ctx<'_>, contact: &Contact, password: &str) {
    let options = c.app.request_options(&contact.public_key);
    match c
        .app
        .mc
        .send_login_sync(&contact.public_key, password, options)
        .await
    {
        Ok(_) => {
            if c.json {
                c.json_indent(&json!({"login_success": true}));
            } else {
                c.line("Login success");
            }
        }
        Err(meshcore::Error::LoginFailed) => {
            if c.json {
                c.json_indent(&json!({"login_success": false, "error": "login failed"}));
            } else {
                c.line("Login failed");
            }
        }
        // a password the device would cut short, refused before sending
        Err(meshcore::Error::InvalidArgument(e)) => c.line(&format!("Login failed : {e}")),
        Err(_) => c.line("Login failed : Error or Timeout waiting response"),
    }
}

/// Looks up the contact of argument 1 (`{"error": "unknown contact"}`).
async fn remote_contact(c: &mut Ctx<'_>) -> Result<Option<Contact>, CmdError> {
    let name = c.arg(1)?.to_string();
    let _ = c.app.mc.ensure_contacts(false).await;
    let contact = c.app.contact_from_arg(&name).await;
    if contact.is_none() {
        let text = format!("Unknown contact {name}");
        c.error(&json!({"error": "unknown contact"}), &text);
    }
    Ok(contact)
}

/// `logout <name>`.
pub async fn logout(c: &mut Ctx<'_>) -> CmdResult {
    if let Some(contact) = remote_contact(c).await? {
        match c.app.mc.send_logout(&contact.public_key).await {
            Err(e) => c.line(&format!("Error while logout: {e}")),
            Ok(()) => {
                if c.json {
                    c.json_line(&json!({}));
                } else {
                    c.line("Logout ok");
                }
            }
        }
    }
    Ok(1)
}

/// `disc_path <ct>`: discovers the in and out path of a contact.
pub async fn disc_path(c: &mut Ctx<'_>) -> CmdResult {
    let Some(contact) = remote_contact(c).await? else {
        return Ok(1);
    };
    let _ = c.app.mc.ensure_contacts(false).await;
    let options = c.app.request_options(&contact.public_key);
    let result = match c
        .app
        .mc
        .send_path_discovery_sync(&contact.public_key, options)
        .await
    {
        Ok(discovery) => to_value(&discovery),
        Err(_) => json!({"error": "timeout"}),
    };
    if c.json {
        c.json_indent(&result);
    } else if result.get("error").is_some() {
        c.line("Timeout while discovering path");
    } else {
        let or_direct = |s: String| {
            if s.is_empty() {
                "direct".to_string()
            } else {
                s
            }
        };
        let text = format!(
            "Path for {}: out {}, in {}",
            contact.name,
            or_direct(vstr(&result, "out_path")),
            or_direct(vstr(&result, "in_path"))
        );
        c.line(&text);
    }
    Ok(1)
}

/// Parses the `-t` filter of `node_discover`: a number (`int()`, packed
/// only when sending, so a later `-t` can replace one out of range) or
/// names.
///
/// # Errors
///
/// A number beyond 64 bits, which Python takes and fails on when sending.
pub fn discover_types(arg: &str, current: i64) -> Result<i64, CmdError> {
    match crate::util::py::int(arg, 10) {
        Ok(value) => return Ok(value),
        Err(e) if !e.starts_with("invalid literal") => return Err(CmdError::Failed(e)),
        Err(_) => {}
    }
    if arg.contains("all") {
        return Ok(0xFF);
    }
    let mut types = current;
    if arg.contains("rep") || arg.contains("rpt") {
        types |= 4;
    }
    if arg.contains("cli") || arg.contains("comp") {
        types |= 2;
    }
    if arg.contains("room") {
        types |= 8;
    }
    if arg.contains("sens") {
        types |= 16;
    }
    Ok(types)
}

/// The next event of a subscription that matches `filter`, waiting at most
/// `timeout`; `None` on timeout or when the connection is gone.
async fn next_matching(
    events: &mut meshcore::Subscription,
    timeout: Duration,
    filter: impl Fn(&Event) -> bool,
) -> Option<Event> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(event)) if filter(&event) => return Some(event),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return None,
        }
    }
}

/// `node_discover [-t<filter>] [-f]`: discovers zero hop nodes.
pub async fn node_discover(c: &mut Ctx<'_>) -> CmdResult {
    let mut argnum = 0;
    let mut types = 0xFF;
    let mut prefix_only = true;
    let mut filter_args = false;
    let mut args = &c.cmds[1..];
    while let Some(arg) = super::next_option(args)? {
        if arg.starts_with("-f") {
            prefix_only = false;
        }
        if let Some(filter) = arg.strip_prefix("-t") {
            if !filter_args {
                filter_args = true;
                types = 0;
            }
            types = discover_types(filter, types)?;
        }
        argnum += 1;
        args = &args[1..];
    }
    // packed with to_bytes(1) when meshcore_py sends the request
    let types = u8::try_from(types).map_err(|_| {
        CmdError::Failed(if types < 0 {
            "can't convert negative int to unsigned".to_string()
        } else {
            "int too big to convert".to_string()
        })
    })?;

    // known contacts are left out of list_discovered, so load them first
    // (the reference loads them only after the check)
    let _ = c.app.mc.ensure_contacts(false).await;
    // subscribe before sending: answers can arrive back to back, and the
    // reference, which waits anew after each one, can miss the next
    let mut events = c.app.mc.subscribe();
    let Ok(tag) = c
        .app
        .mc
        .send_node_discover_req(types, prefix_only, None, None)
        .await
    else {
        c.line("Error sending discover request");
        return Ok(argnum);
    };

    let mut found: Vec<Value> = Vec::new();
    let mut new_nodes: Vec<Value> = Vec::new();
    while let Some(event) = next_matching(&mut events, Duration::from_secs(5), |e: &Event| {
        matches!(e, Event::DiscoverResponse(_)) && e.tag() == Some(tag)
    })
    .await
    {
        let node = to_value(&event);
        let key = vstr(&node, "pubkey");
        if key.len() == 64 && c.app.mc.get_contact_by_key_prefix(&key).is_none() {
            new_nodes.push(node.clone());
        }
        found.push(node);
    }
    c.app.state().discovered = new_nodes;

    if c.json {
        c.json_line(&Value::Array(found));
    } else {
        let mut text = format!("Discovered {} nodes:{}", found.len(), c.end);
        for node in &found {
            let key = vstr(node, "pubkey");
            let name = match c.app.mc.get_contact_by_key_prefix(&key) {
                Some(contact) => format!(
                    "{} {}",
                    key.chars().take(6).collect::<String>(),
                    contact.name
                ),
                None => key.chars().take(16).collect(),
            };
            let snr = |k: &str| node.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            let _ = writeln!(
                text,
                " {} {} SNR: {:6.2}->{:6.2} RSSI: ->{:4}",
                pad_right(&name, 28),
                pad_left(&type_name(vint(node, "node_type")), 4),
                snr("SNR_in"),
                snr("SNR"),
                vint(node, "RSSI")
            );
        }
        c.emit(&text);
    }
    Ok(argnum)
}

/// `list_discovered`.
pub fn list_discovered(c: &mut Ctx<'_>) -> usize {
    let nodes = Value::Array(c.app.state().discovered.clone());
    c.json_indent(&nodes);
    0
}

/// `add_discovered [-n<name>] [-a] <prefix>`.
pub async fn add_discovered(c: &mut Ctx<'_>) -> CmdResult {
    let mut name: Option<String> = None;
    let mut add_all = false;
    let mut argnum = 1;
    let mut args = &c.cmds[1..];
    while let Some(arg) = super::next_option(args)? {
        if let Some(n) = arg.strip_prefix("-n") {
            name = Some(n.to_string());
        }
        if arg.starts_with("-a") {
            add_all = true;
            argnum -= 1;
        }
        argnum += 1;
        args = &args[1..];
    }
    let prefix = if add_all {
        String::new()
    } else {
        args.first().ok_or(CmdError::Params)?.clone()
    };

    let nodes: Vec<Value> = {
        let mut state = c.app.state();
        let (selected, kept): (Vec<Value>, Vec<Value>) = state
            .discovered
            .drain(..)
            .partition(|n| add_all || vstr(n, "pubkey").starts_with(&prefix));
        state.discovered = kept;
        selected
    };

    let mut added = Vec::new();
    for node in nodes {
        let key_hex = vstr(&node, "pubkey");
        let node_name = if add_all || name.is_none() {
            let n: String = key_hex.chars().take(8).collect();
            name = Some(n.clone());
            n
        } else {
            name.clone().unwrap_or_default()
        };
        let Ok(key) = PublicKey::from_hex(&key_hex) else {
            continue;
        };
        let kind = u8::try_from(vint(&node, "node_type")).unwrap_or(0);
        let contact = super::contacts::new_contact(key, kind, &node_name);
        match c.app.mc.add_contact(&contact).await {
            Err(e) => c.emit(&format!("Error adding contact: {e}\n")),
            Ok(()) => added.push(json!({"name": node_name, "pubkey": key_hex, "type": kind})),
        }
    }
    let _ = c.app.mc.get_contacts(0).await;
    c.json_line(&Value::Array(added));
    Ok(argnum)
}

/// The contact of a request; an unknown one is reported and the command
/// ends there, its arguments consumed (one unless `argnum` says otherwise).
macro_rules! with_contact {
    ($c:expr) => {
        with_contact!($c, 1)
    };
    ($c:expr, $argnum:expr) => {
        match remote_contact($c).await? {
            Some(contact) => contact,
            None => return Ok($argnum),
        }
    };
}

/// Reports a failed request.
fn request_error(c: &mut Ctx<'_>, json_text: &str, text: &str) {
    c.error(&json!({"error": json_text}), text);
}

/// `req_regions <ct>`.
pub async fn req_regions(c: &mut Ctx<'_>) -> CmdResult {
    let contact = with_contact!(c);
    let options = c.app.request_options(&contact.public_key);
    match c
        .app
        .mc
        .req_regions_sync(&contact.public_key, options)
        .await
    {
        Err(_) => request_error(c, "Getting data", "Error getting data"),
        Ok(regions) => {
            if c.json {
                c.json_line(&json!({"repeater": contact.name, "regions": regions}));
            } else {
                c.line(&format!("{} repeats {regions}", contact.name));
            }
        }
    }
    Ok(1)
}

/// `req_owner <ct>`.
pub async fn req_owner(c: &mut Ctx<'_>) -> CmdResult {
    let contact = with_contact!(c);
    let options = c.app.request_options(&contact.public_key);
    match c.app.mc.req_owner_sync(&contact.public_key, options).await {
        Err(_) => request_error(c, "Getting data", "Error getting data"),
        Ok(owner) => {
            let owner = to_value(&owner);
            if c.json {
                c.json_line(&owner);
            } else if vstr(&owner, "owner").is_empty() {
                c.line(&format!("{} has no owner set", vstr(&owner, "name")));
            } else {
                c.line(&format!(
                    "{} is owned by {}",
                    vstr(&owner, "name"),
                    vstr(&owner, "owner")
                ));
            }
        }
    }
    Ok(1)
}

/// `req_clock <ct>`: the remote node's clock.
pub async fn req_clock(c: &mut Ctx<'_>) -> CmdResult {
    let contact = with_contact!(c);
    let options = c.app.request_options(&contact.public_key);
    match c.app.mc.req_basic_sync(&contact.public_key, options).await {
        Err(_) => request_error(c, "Getting data", "Error getting data"),
        Ok(response) => {
            // int.from_bytes(data[0:4], "little") like the reference
            let clock = response
                .data
                .iter()
                .take(4)
                .rev()
                .fold(0u32, |acc, b| acc << 8 | u32::from(*b));
            c.line(&clock.to_string());
        }
    }
    Ok(1)
}

/// `req_telemetry <ct>`.
pub async fn req_telemetry(c: &mut Ctx<'_>) -> CmdResult {
    let contact = with_contact!(c);
    let options = c.app.request_options(&contact.public_key);
    match c
        .app
        .mc
        .req_telemetry_sync(&contact.public_key, options)
        .await
    {
        Err(_) => request_error(c, "Getting data", "Error getting data"),
        Ok(telemetry) => {
            let value = json!({
                "name": contact.name,
                "pubkey_pre": contact.public_key.to_string().chars().take(16).collect::<String>(),
                "lpp": to_value(&telemetry),
            });
            c.json_indent(&value);
        }
    }
    Ok(1)
}

/// `req_status <ct>`.
pub async fn req_status(c: &mut Ctx<'_>) -> CmdResult {
    let contact = with_contact!(c);
    let options = c.app.request_options(&contact.public_key);
    match c.app.mc.req_status_sync(&contact.public_key, options).await {
        Err(_) => request_error(c, "Getting data", "Error getting data"),
        Ok(status) => c.json_indent(&to_value(&status)),
    }
    Ok(1)
}

/// Reads a time span of `req_mma` in seconds (`30s`, `5m`, `2h`, plain
/// minutes) with `int()`, like the reference.
fn mma_seconds(text: &str) -> Result<i64, CmdError> {
    let last = text.chars().last().ok_or(CmdError::Params)?;
    let (number, factor) = match last {
        's' => (&text[..text.len() - 1], 1),
        'm' => (&text[..text.len() - 1], 60),
        'h' => (&text[..text.len() - 1], 3600),
        _ => (text, 60),
    };
    Ok(crate::util::py::int(number, 10)
        .map_err(CmdError::Failed)?
        .saturating_mul(factor))
}

/// A time span packed like `meshcore_py`'s `to_bytes(4)` (unsigned).
fn mma_span(secs: i64) -> Result<u32, CmdError> {
    u32::try_from(secs).map_err(|_| {
        CmdError::Failed(if secs < 0 {
            "can't convert negative int to unsigned".to_string()
        } else {
            "int too big to convert".to_string()
        })
    })
}

/// `req_mma <ct> <from> <to>`: min/max/avg of a sensor.
pub async fn req_mma(c: &mut Ctx<'_>) -> CmdResult {
    let contact = with_contact!(c, 3);
    let from = mma_seconds(c.arg(2)?)?;
    let to = mma_seconds(c.arg(3)?)?;
    // packed once both are read, as in meshcore_py
    let (from, to) = (mma_span(from)?, mma_span(to)?);
    let options = c.app.request_options(&contact.public_key);
    match c
        .app
        .mc
        .req_mma_sync(&contact.public_key, from, to, options)
        .await
    {
        Err(_) => request_error(c, "Getting data", "Error getting data"),
        Ok(entries) => c.json_indent(&to_value(&entries)),
    }
    Ok(3)
}

/// `req_acl <ct>`: access control list of a node.
pub async fn req_acl(c: &mut Ctx<'_>) -> CmdResult {
    let contact = with_contact!(c);
    let options = c.app.request_options(&contact.public_key);
    match c.app.mc.req_acl_sync(&contact.public_key, options).await {
        Err(_) => request_error(c, "Getting data", "Error getting data"),
        Ok(entries) => {
            let entries = to_value(&entries);
            if c.json {
                c.json_indent(&entries);
            } else {
                let self_key = vstr(&c.app.self_info_value(), "public_key");
                let lines: Vec<String> = entries
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .map(|e| {
                        let key = vstr(e, "key");
                        let mut name = format!(" [{key}] ");
                        match c.app.mc.get_contact_by_key_prefix(&key) {
                            Some(ct) => name.push_str(&ct.name),
                            None if self_key.starts_with(&key) => name.push_str("self"),
                            None => {}
                        }
                        format!("{name}{}: {:02x}", ansi::column(42), vint(e, "perm"))
                    })
                    .collect();
                c.line(&lines.join("\n"));
            }
        }
    }
    Ok(1)
}

/// `req_neighbours <ct>`: neighbour list of a repeater.
pub async fn req_neighbours(c: &mut Ctx<'_>) -> CmdResult {
    let contact = with_contact!(c);
    let options = c.app.request_options(&contact.public_key);
    // order 0 and 4 byte key prefixes, the defaults of meshcore_py
    match c
        .app
        .mc
        .fetch_all_neighbours(&contact.public_key, 0, 4, options)
        .await
    {
        Err(_) => request_error(c, "Getting data", "Error getting data"),
        Ok(response) => {
            let res = to_value(&response);
            if c.json {
                c.json_indent(&res);
            } else {
                let text = format_neighbours(c.app, &res, &contact.name, terminal_width());
                c.line(&text);
            }
        }
    }
    Ok(1)
}

/// Formats a neighbour list like the reference, depending on the width.
#[must_use]
pub fn format_neighbours(app: &App, res: &Value, name: &str, width: usize) -> String {
    let results = vint(res, "results_count");
    let mut out = format!(
        "Got {results} neighbours out of {} from {name}:",
        vint(res, "neighbours_count")
    );
    if results > 0 {
        out.push('\n');
    }
    let empty = Vec::new();
    let neighbours = res
        .get("neighbours")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let lines: Vec<String> = neighbours
        .iter()
        .map(|n| {
            let key = vstr(n, "pubkey");
            let label = match app.mc.get_contact_by_key_prefix(&key) {
                Some(ct) if width > 60 => format!(
                    "{}{}",
                    pad_right(
                        &format!("[{}] {}", key.chars().take(8).collect::<String>(), ct.name),
                        30
                    ),
                    ansi::column(31)
                ),
                Some(ct) => format!("{}{}", pad_right(&ct.name, 20), ansi::column(21)),
                None if width > 60 => format!("[{key}]{}", ansi::column(31)),
                None => format!("[{key}]{}", ansi::column(21)),
            };
            let secs = vint(n, "secs_ago");
            let detail = |text: String| {
                if width > 62 {
                    format!("{text} ({secs}s)")
                } else {
                    text
                }
            };
            let ago = if secs >= 86_400 {
                detail(format!("{}d ago", secs / 86_400))
            } else if secs >= 3600 {
                detail(format!("{}h ago", secs / 3600))
            } else if secs >= 60 {
                detail(format!("{}m ago", secs / 60))
            } else {
                format!("{secs}s")
            };
            let snr = py_number(n.get("snr").unwrap_or(&Value::Null));
            format!(
                " {label} {ago}, {snr}dB{}",
                if width > 66 { " SNR" } else { "" }
            )
        })
        .collect();
    out.push_str(&lines.join("\n"));
    out
}

/// `req_binary <ct> <hex>`: binary request, the first byte is the request
/// type (01 status, 02 keep alive, 03 telemetry, 04 mma, 05 acl,
/// 06 neighbours, 07 owner info ...), the rest its payload.
pub async fn req_binary(c: &mut Ctx<'_>) -> CmdResult {
    let contact = with_contact!(c, 2);
    let data = crate::util::py::fromhex(c.arg(2)?).map_err(CmdError::Failed)?;
    let (kind, payload) = data
        .split_first()
        .ok_or_else(|| CmdError::Failed("empty request".to_string()))?;
    let request_type = meshcore::BinaryReqType::from_code(*kind)
        .ok_or_else(|| CmdError::Failed(format!("unknown binary request type {kind:02x}")))?;
    let options = c.app.request_options(&contact.public_key);
    match c
        .app
        .mc
        .req_binary_sync(&contact.public_key, request_type, payload, options)
        .await
    {
        Err(_) => request_error(c, "Getting binary data", "Error getting binary data"),
        Ok(response) => c.json_line(&to_value(&response)),
    }
    Ok(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mma_time_spans() {
        assert_eq!(mma_seconds("30s").ok(), Some(30));
        assert_eq!(mma_seconds("5m").ok(), Some(300));
        assert_eq!(mma_seconds("2h").ok(), Some(7200));
        assert_eq!(mma_seconds("10").ok(), Some(600));
        assert!(matches!(mma_seconds(""), Err(CmdError::Params)));
        assert!(
            matches!(mma_seconds("xm"), Err(CmdError::Failed(m)) if m == "invalid literal for int() with base 10: 'x'")
        );
        assert_eq!(mma_seconds("-1s").ok(), Some(-1));
        assert!(
            matches!(mma_span(-1), Err(CmdError::Failed(m)) if m == "can't convert negative int to unsigned")
        );
        assert!(
            matches!(mma_seconds("99999999999h").and_then(mma_span), Err(CmdError::Failed(m)) if m == "int too big to convert")
        );
    }

    #[test]
    fn trace_arguments() {
        assert_eq!(parse_trace_arg("6a61").unwrap(), (vec![0x6a, 0x61], 0));
        assert_eq!(parse_trace_arg("6a,61").unwrap(), (vec![0x6a, 0x61], 0));
        assert_eq!(
            parse_trace_arg("6a83,6144").unwrap(),
            (vec![0x6a, 0x83, 0x61, 0x44], 1)
        );
        assert_eq!(
            parse_trace_arg("6a836144:1").unwrap(),
            (vec![0x6a, 0x83, 0x61, 0x44], 1)
        );
        assert_eq!(parse_trace_arg("6a83,").unwrap(), (vec![0x6a, 0x83], 1));
        assert_eq!(
            parse_trace_arg("6a836144,01020304").unwrap(),
            (vec![0x6a, 0x83, 0x61, 0x44, 1, 2, 3, 4], 2)
        );
        assert_eq!(
            parse_trace_arg("5f1002,7c2003"),
            Err(TraceArgError::Path("unknown path_hash_len 3".to_string()))
        );
        assert_eq!(
            parse_trace_arg("5f10,7c"),
            Err(TraceArgError::Path(String::new()))
        );
        assert_eq!(
            parse_trace_arg("6g"),
            Err(TraceArgError::Command(
                "non-hexadecimal number found in fromhex() arg at position 1".to_string()
            ))
        );
        assert!(matches!(
            parse_trace_arg("6a:x"),
            Err(TraceArgError::Command(_))
        ));
        // the hex is read before the flags are packed
        assert_eq!(
            parse_trace_arg("zz:300"),
            Err(TraceArgError::Command(
                "non-hexadecimal number found in fromhex() arg at position 0".to_string()
            ))
        );
        assert_eq!(
            parse_trace_arg("6a:300"),
            Err(TraceArgError::Command(
                "byte must be in range(0, 256)".to_string()
            ))
        );
    }

    #[test]
    fn trace_to_contact() {
        let repeater = json!({"type": 2, "public_key": "abcdef", "out_path_len": 2,
            "out_path": "1122", "out_path_hash_mode": 0});
        assert_eq!(trace_path_to(&repeater).unwrap(), "1122ab2211");
        let client = json!({"type": 1, "public_key": "abcdef", "out_path_len": 2,
            "out_path": "11112222", "out_path_hash_mode": 1});
        assert_eq!(trace_path_to(&client).unwrap(), "111122221111:1");
        let three = json!({"type": 2, "public_key": "abcdef01", "out_path_len": 2,
            "out_path": "111111222222", "out_path_hash_mode": 2});
        assert_eq!(trace_path_to(&three).unwrap(), "11112222abcd22221111:1");
        assert!(trace_path_to(&json!({"out_path_len": -1})).is_none());
    }

    #[test]
    fn trace_from_discovered_paths() {
        let disc = json!({"out_path": "1122", "out_path_hash_len": 1, "in_path": "2211", "in_path_hash_len": 1});
        assert_eq!(trace_from_discovery(&disc, &json!({"type": 1})), "11,22,11");
        assert_eq!(
            trace_from_discovery(&disc, &json!({"type": 2, "public_key": "ab01"})),
            "11,22,ab,22,11"
        );
        let wide = json!({"out_path": "1111", "out_path_hash_len": 2, "in_path": "", "in_path_hash_len": 2});
        assert_eq!(trace_from_discovery(&wide, &json!({"type": 1})), "1111,");
    }

    #[test]
    fn discover_filters() {
        assert_eq!(discover_types("all", 0).ok(), Some(0xFF));
        assert_eq!(discover_types("rep,sens", 0).ok(), Some(4 | 16));
        assert_eq!(discover_types("3", 0).ok(), Some(3));
        // range checked when sending; a later -t may replace it
        assert_eq!(discover_types("300", 0).ok(), Some(300));
        assert_eq!(discover_types("-1", 0).ok(), Some(-1));
        assert!(discover_types("99999999999999999999", 0).is_err());
        assert_eq!(discover_types("", 0).ok(), Some(0));
    }
}
