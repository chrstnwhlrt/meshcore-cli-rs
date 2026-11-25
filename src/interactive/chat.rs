//! Commands inside a contact (`process_contact_chat_line`) and `apply_to`.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use meshcore::{Contact, EventKind};
use serde_json::{Value, json};

use crate::app::{App, contact_type, to_value, vint, vstr};
use crate::commands::{Recipient, print_trace_to, process_cmds};
use crate::display::type_name;
use crate::output::Sink;
use crate::paths;
use crate::util::json::dumps;
use crate::util::text::{pad_right, strip_outer_quotes};
use crate::util::time::{format_local, parse_filter_time};

/// Commands that take the contact as their only argument and can be
/// chained (`ci cn`, `rp login` ...).
fn is_contact_command(line: &str) -> bool {
    let word = |w: &str| line == w || line.starts_with(&format!("{w} "));
    word("sc")
        || line.starts_with("share_contact")
        || word("ec")
        || line.starts_with("export_contact")
        || word("uc")
        || line.starts_with("upload_contact")
        || word("rp")
        || line.starts_with("reset_path")
        || word("dp")
        || line.starts_with("disc_path")
        || line.starts_with("contact_info")
        || word("ci")
        || line.starts_with("req_status")
        || word("rs")
        || line.starts_with("req_neighbours")
        || word("rn")
        || line.starts_with("req_telemetry")
        || word("rt")
        || line.starts_with("req_regions")
        || word("rr")
        || line.starts_with("req_owner")
        || word("ro")
        || line.starts_with("req_clock")
        || line.starts_with("req_acl")
        || word("ra")
        || word("path")
        || line.starts_with("advert_path")
        || word("ap")
        || line.starts_with("reset_advert_time")
        || line.starts_with("logout")
}

/// Runs a command and collects its output.
async fn capture(app: &Arc<App>, args: &[String], json: bool) -> String {
    let mut buffer = Sink::buffer();
    Box::pin(process_cmds(app, args, json, &mut buffer, "")).await;
    buffer.into_string()
}

/// Output of the first command combined with the rest of the line
/// (`process_second`).
#[allow(clippy::too_many_arguments)]
async fn process_second(
    app: &Arc<App>,
    contact: &Contact,
    line: &str,
    inside: bool,
    json: bool,
    sink: &mut Sink,
    end: &str,
    first: String,
) -> Result<(), super::Raised> {
    let mut res = first.clone();
    if let Some((_, secline)) = line.split_once(' ') {
        let mut buffer = Sink::buffer();
        // an exception of the second part leaves the first one unprinted
        Box::pin(process_contact_chat_line(
            app,
            contact,
            secline,
            true,
            json,
            &mut buffer,
            end,
        ))
        .await?;
        // Ctrl-C at a prompt of the second part ends it all, unprinted
        if app.interrupted.load(Ordering::Relaxed) {
            return Ok(());
        }
        let second = buffer.into_string();
        if !second.is_empty() {
            if json {
                res = format!("{res}, {second}");
                if !inside && !first.is_empty() {
                    res = format!("[{res}]\n");
                }
            } else if !first.is_empty() {
                res = format!("{res} {second}");
            } else {
                res = second;
            }
        }
    } else if !res.is_empty() && (!json || !inside) {
        res.push_str(end);
    }
    sink.write(&res);
    sink.flush();
    Ok(())
}

/// `{"key": value}` in json mode, the value otherwise.
fn make_value(json: bool, key: &str, value: Value) -> String {
    if json {
        let mut map = serde_json::Map::new();
        map.insert(key.to_string(), value);
        dumps(&Value::Object(map))
    } else {
        value
            .as_str()
            .map_or_else(|| value.to_string(), str::to_string)
    }
}

/// Parses a permission like `0x0f`, `#15` or hex `0f`, as the reference
/// does with `int()`.
fn parse_perm(text: &str) -> Result<i64, String> {
    if text.starts_with("0x") {
        crate::util::py::int(text, 0)
    } else if let Some(dec) = text.strip_prefix('#') {
        crate::util::py::int(dec, 10)
    } else {
        crate::util::py::int(text, 16)
    }
}

/// Handles a line typed inside a contact; returns false when the line is
/// not one of the contact commands (it is then sent as message or command).
pub async fn process_contact_chat_line(
    app: &Arc<App>,
    contact: &Contact,
    line: &str,
    inside: bool,
    json: bool,
    sink: &mut Sink,
    end: &str,
) -> Result<bool, super::Raised> {
    let kind = contact_type(contact);
    if kind == 0 {
        return Ok(false);
    }
    let name = contact.name.clone();
    let value = to_value(contact);
    let mut line = line.to_string();

    // a single word may carry a scope: login%#region
    if !line.contains(' ')
        && let Some((cmd, scope)) = line.rsplit_once('%')
    {
        let scope = scope.to_string();
        line = cmd.to_string();
        app.set_scope(&scope).await;
    }

    if let Some(cmd) = line.strip_prefix(':') {
        let args = vec!["cmd".to_string(), name, cmd.to_string()];
        process_cmds(app, &args, json, sink, "\n").await;
        return Ok(true);
    }
    if line == "reset path" {
        let args = vec!["reset_path".to_string(), name];
        process_cmds(app, &args, json, sink, "\n").await;
        return Ok(true);
    }
    if line.starts_with("contact_key") || line.starts_with("ck") {
        let first = line.split(' ').next().unwrap_or_default();
        let mut key = vstr(&value, "public_key");
        if line.starts_with("contact_key:") || line.starts_with("ck:") {
            let len_text = first.split(':').nth(1).unwrap_or_default();
            let len = match crate::util::py::int(len_text, 10) {
                Ok(len) => len,
                Err(e) => return Err(super::Raised::Value(e)),
            };
            // public_key[0:2*klen], negative lengths count from the end
            let chars = key.chars().count();
            let doubled = len.saturating_mul(2);
            let end = usize::try_from(doubled).map_or_else(
                |_| chars.saturating_sub(usize::try_from(doubled.unsigned_abs()).unwrap_or(chars)),
                |n| n.min(chars),
            );
            key = key.chars().take(end).collect();
        }
        let out = make_value(json, "public_key", json!(key));
        process_second(app, contact, &line, inside, json, sink, end, out).await?;
        return Ok(true);
    }
    if line.starts_with("contact_type") || line.starts_with("ct") {
        let out = make_value(
            json,
            "contact_type",
            json!(pad_right(&type_name(i64::from(kind)), 4)),
        );
        process_second(app, contact, &line, inside, json, sink, end, out).await?;
        return Ok(true);
    }
    if line.starts_with("contact_name") || line.starts_with("cn") {
        let out = make_value(json, "adv_name", json!(name));
        process_second(app, contact, &line, inside, json, sink, end, out).await?;
        return Ok(true);
    }
    if line.starts_with("contact_lastmod") {
        let lastmod = vint(&value, "lastmod");
        let out = if json {
            make_value(true, "lastmod", json!(lastmod))
        } else {
            format!(
                "{name} updated {} ({lastmod})",
                format_local(lastmod, "%Y-%m-%d at %H:%M:%S")
            )
        };
        process_second(app, contact, &line, inside, json, sink, end, out).await?;
        return Ok(true);
    }
    if line.starts_with("path") {
        let out = make_value(
            json,
            "path",
            json!(crate::commands::contact_path_str(&value)),
        );
        process_second(app, contact, &line, inside, json, sink, end, out).await?;
        return Ok(true);
    }
    if line.starts_with("sleep ") || line.starts_with("s ") {
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        // int() in the reference; fractions are taken too, like `sleep`
        let (secs, cmd_pos) = match parts.get(1).map(|p| crate::commands::seconds(p)) {
            Some(Ok(secs)) => (secs, 2),
            Some(Err(_)) => (1.0, 1),
            None => (1.0, 0),
        };
        if cmd_pos > 0
            && let Some(secline) = line.splitn(cmd_pos + 1, ' ').nth(cmd_pos)
        {
            Box::pin(process_contact_chat_line(
                app, contact, secline, inside, json, sink, end,
            ))
            .await?;
            sink.flush();
        }
        if secs.is_finite() && secs > 0.0 {
            tokio::time::sleep(crate::util::time::duration_from_secs(secs)).await;
        }
        return Ok(true);
    }
    if is_contact_command(&line) {
        let cmd = crate::util::py::split(&line)
            .next()
            .unwrap_or_default()
            .to_string();
        let out = capture(app, &[cmd, name], json).await;
        process_second(app, contact, &line, inside, json, sink, end, out).await?;
        return Ok(true);
    }
    if let Some(timeout) = line.strip_prefix("set timeout ") {
        match crate::util::py::float(timeout.split(' ').next().unwrap_or_default()) {
            Ok(secs) => {
                app.state()
                    .timeouts
                    .insert(contact.public_key.to_string(), secs);
            }
            Err(e) => return Err(super::Raised::Value(e)),
        }
        return Ok(true);
    }
    if line == "get timeout" {
        // the reference prints 0 when no timeout was set, else the float
        let timeout = app
            .state()
            .timeouts
            .get(&contact.public_key.to_string())
            .map_or_else(|| "0".to_string(), |t| crate::display::py_float(*t));
        let text = format!("timeout: {timeout}");
        if json {
            sink.write(&dumps(&json!({ "timeout": text })));
        } else {
            sink.write(&format!("{text}\n"));
        }
        sink.flush();
        return Ok(true);
    }
    if (kind == 4 && line.starts_with("get mma "))
        || (kind > 1
            && (line.starts_with("get telemetry")
                || line.starts_with("get status")
                || line.starts_with("get acl")))
    {
        let words: Vec<&str> = line.split(' ').collect();
        let mut args = vec![format!("req_{}", words.get(1).unwrap_or(&"")), name];
        args.extend(words.iter().skip(2).map(|w| (*w).to_string()));
        if line.starts_with("get mma ") && args.len() < 4 {
            args.push("0".to_string());
        }
        process_cmds(app, &args, json, sink, "\n").await;
        return Ok(true);
    }
    if kind > 1 && (line.starts_with("setperm ") || line.starts_with("set perm ")) {
        // shlex's ValueError leaves the line
        let words =
            crate::util::shlex::split(&line).map_err(|e| super::Raised::Value(e.to_string()))?;
        let off = usize::from(line.starts_with("set perm"));
        let (Some(target), Some(perm)) = (words.get(1 + off), words.get(2 + off)) else {
            sink.write("Wrong number of parameters");
            sink.flush();
            return Ok(true);
        };
        let perm = match parse_perm(perm) {
            Ok(perm) => perm,
            Err(e) => return Err(super::Raised::Value(e)),
        };
        let key = if let Some(ct) = app.contact_from_arg(target).await {
            ct.public_key.to_string()
        } else {
            let self_key = vstr(&app.self_info_value(), "public_key");
            if target == "self" || self_key.starts_with(target.as_str()) {
                self_key
            } else {
                target.clone()
            }
        };
        let args = vec!["cmd".to_string(), name, format!("setperm {key} {perm}")];
        process_cmds(app, &args, json, sink, "\n").await;
        return Ok(true);
    }
    if line == "trace" || line == "tr" {
        let mut buffer = Sink::buffer();
        print_trace_to(app, contact, json, &mut buffer, "").await;
        process_second(
            app,
            contact,
            &line,
            inside,
            json,
            sink,
            end,
            buffer.into_string(),
        )
        .await?;
        return Ok(true);
    }
    if line == "dtrace" || line == "dt" {
        let mut buffer = Sink::buffer();
        crate::commands::print_disc_trace_to(app, contact, json, &mut buffer, "").await;
        process_second(
            app,
            contact,
            &line,
            inside,
            json,
            sink,
            end,
            buffer.into_string(),
        )
        .await?;
        return Ok(true);
    }
    if let Some((first, rest)) = line.split_once(' ') {
        let mut first = first.to_string();
        if let Some((cmd, scope)) = first.rsplit_once('%') {
            let scope = scope.to_string();
            first = cmd.to_string();
            app.set_scope(&scope).await;
        }
        if matches!(
            first.as_str(),
            "cmd" | "msg" | "cp" | "change_path" | "cf" | "change_flags" | "req_binary" | "login"
        ) {
            let text = if first == "msg" {
                strip_outer_quotes(rest).to_string()
            } else {
                rest.to_string()
            };
            let args = vec![first, name, text];
            process_cmds(app, &args, json, sink, "\n").await;
            return Ok(true);
        }
    }
    if line == "login" {
        login_with_stored_password(app, contact, json, sink).await?;
        return Ok(true);
    }
    if line.starts_with("forget_password") || line.starts_with("fp") {
        // os.remove, whose OSError leaves the line
        let file = paths::password_file(&contact.public_key.to_string());
        if file.exists()
            && let Err(e) = std::fs::remove_file(&file)
        {
            return Err(super::Raised::Other(crate::util::py::os_error(
                &e,
                Some(&file.display().to_string()),
            )));
        }
        if let Some((_, secline)) = line.split_once(' ') {
            Box::pin(process_contact_chat_line(
                app, contact, secline, inside, json, sink, end,
            ))
            .await?;
        }
        return Ok(true);
    }
    if kind == 4 && (line.starts_with("req_mma ") || line.starts_with("rm ")) {
        let mut words: Vec<String> = line.split(' ').map(str::to_string).collect();
        if words.len() < 3 {
            words.push("0".to_string());
        }
        let args = vec![words[0].clone(), name, words[1].clone(), words[2].clone()];
        process_cmds(app, &args, json, sink, "\n").await;
        return Ok(true);
    }
    Ok(false)
}

/// `login` inside a contact: uses the stored password or asks for one.
///
/// # Errors
///
/// A password file that cannot be read or written, whose error the
/// reference does not catch here.
async fn login_with_stored_password(
    app: &Arc<App>,
    contact: &Contact,
    json: bool,
    sink: &mut Sink,
) -> Result<(), super::Raised> {
    let password_file =
        paths::existing_config_dir().map(|_| paths::password_file(&contact.public_key.to_string()));
    let file_error = |file: &std::path::Path, e: crate::util::py::TextError| match e {
        // a UnicodeDecodeError is a ValueError
        crate::util::py::TextError::Decode(text) => super::Raised::Value(text),
        crate::util::py::TextError::Io(_) => {
            super::Raised::Other(e.message(&file.display().to_string()))
        }
    };
    let mut password = String::new();
    if let Some(file) = password_file.as_ref().filter(|f| f.exists()) {
        // f.readline().strip()
        let content = crate::util::py::read_text(file, crate::util::py::TextRead::Lines)
            .map_err(|e| file_error(file, e))?;
        password = crate::util::py::lines(&content)
            .next()
            .map(|l| crate::util::py::strip(l).to_string())
            .unwrap_or_default();
    }
    if password.is_empty() {
        sink.flush();
        let prompt = format!("Password for {}: ", contact.name);
        let read = tokio::task::spawn_blocking(move || crate::editor::read_password(&prompt)).await;
        password = match read {
            Ok(Ok(entered)) => entered,
            // a KeyboardInterrupt, which the reference does not catch here
            Ok(Err(rustyline::error::ReadlineError::Interrupted)) => {
                app.interrupted.store(true, Ordering::Relaxed);
                return Ok(());
            }
            _ => {
                log::info!("Canceled");
                return Ok(());
            }
        };
        if let Some(file) = &password_file {
            std::fs::write(file, &password)
                .map_err(|e| file_error(file, crate::util::py::TextError::Io(e)))?;
        }
    }
    let args = vec!["login".to_string(), contact.name.clone(), password];
    process_cmds(app, &args, json, sink, "\n").await;
    Ok(())
}

/// Why `apply_to` stopped: its filter was rejected in one of the three
/// ways the reference reports, or a command run on a contact raised.
#[derive(Debug, PartialEq, Eq)]
pub enum FilterError {
    /// Python's `IndexError`: a part of the filter is missing.
    Index,
    /// Python's `ValueError` from `int()` or `float()`, with its message.
    Value(String),
    /// A message the reference logs itself before giving up.
    Logged(String),
    /// An exception of the command run on a contact, which ends `apply_to`.
    Raised(super::Raised),
}

impl From<Option<String>> for FilterError {
    fn from(error: Option<String>) -> Self {
        error.map_or(Self::Index, Self::Value)
    }
}

/// Filter of `apply_to`.
#[derive(Debug, Default, PartialEq)]
pub struct Filter {
    upd_before: Option<f64>,
    upd_after: Option<f64>,
    adv_before: Option<f64>,
    adv_after: Option<f64>,
    contact_type: Option<i64>,
    min_hops: Option<i64>,
    max_hops: Option<i64>,
    flags: Option<i64>,
}

impl Filter {
    /// Parses a comma separated filter like `t=2,u>1d,d`.
    ///
    /// # Errors
    ///
    /// Returns the message the reference logs for invalid filters.
    pub fn parse(text: &str, now: f64) -> Result<Self, FilterError> {
        let mut filter = Self::default();
        for f in text.split(',') {
            if f == "all" {
                continue;
            }
            let mut chars = f.chars();
            let kind = chars.next().ok_or(FilterError::Index)?;
            let op = chars.next();
            let value = chars.as_str();
            let int = |v: &str| crate::util::py::int(v, 10).map_err(FilterError::Value);
            match kind {
                'u' | 'a' => {
                    // the value is read before the operator is checked
                    let t = parse_filter_time(value, now)?;
                    let (before, after) = if kind == 'u' {
                        (&mut filter.upd_before, &mut filter.upd_after)
                    } else {
                        (&mut filter.adv_before, &mut filter.adv_after)
                    };
                    match op {
                        Some('<') => *before = Some(t),
                        Some('>') => *after = Some(t),
                        _ => {
                            return Err(FilterError::Logged(
                                "Time filter can only be < or >".to_string(),
                            ));
                        }
                    }
                }
                't' => match op.ok_or(FilterError::Index)? {
                    '=' => filter.contact_type = Some(int(value)?),
                    _ => {
                        return Err(FilterError::Logged(
                            "Type can only be equals to a value".to_string(),
                        ));
                    }
                },
                'd' => filter.min_hops = Some(0),
                'f' => filter.max_hops = Some(-1),
                'h' => match op.ok_or(FilterError::Index)? {
                    '>' => filter.min_hops = Some(int(value)?.saturating_add(1)),
                    '<' => filter.max_hops = Some(int(value)?.saturating_sub(1)),
                    '=' => {
                        let v = int(value)?;
                        filter.min_hops = Some(v);
                        filter.max_hops = Some(v);
                    }
                    _ => {}
                },
                'b' => {
                    if op.ok_or(FilterError::Index)? == '=' {
                        filter.flags = Some(int(value)?);
                    }
                }
                _ => return Err(FilterError::Logged(format!("Unknown filter {f}"))),
            }
        }
        Ok(filter)
    }

    /// True when a contact payload matches.
    #[must_use]
    pub fn matches(&self, c: &Value) -> bool {
        #[allow(clippy::cast_precision_loss)]
        let num = |k: &str| vint(c, k) as f64;
        self.contact_type.is_none_or(|t| vint(c, "type") == t)
            && self.upd_before.is_none_or(|t| num("lastmod") < t)
            && self.upd_after.is_none_or(|t| num("lastmod") > t)
            && self.adv_before.is_none_or(|t| num("last_advert") < t)
            && self.adv_after.is_none_or(|t| num("last_advert") > t)
            && self.min_hops.is_none_or(|h| vint(c, "out_path_len") >= h)
            && self.max_hops.is_none_or(|h| vint(c, "out_path_len") <= h)
            && self.flags.is_none_or(|f| vint(c, "flags") & f == f)
    }
}

/// Runs a command on all contacts matching a filter (`apply_to`).
///
/// # Errors
///
/// Returns why the filter was rejected; messages of the kind
/// [`FilterError::Logged`] are already logged.
pub async fn apply_command_to_contacts(
    app: &Arc<App>,
    filter: &str,
    line: &str,
    json: bool,
    sink: &mut Sink,
    end: &str,
) -> Result<(), FilterError> {
    let _ = app.mc.ensure_contacts(false).await;
    // time.time() of the reference, with its fraction of a second
    let now = crate::util::time::now_f64();
    let filter = match Filter::parse(filter, now) {
        Ok(filter) => filter,
        Err(FilterError::Logged(message)) => {
            log::error!("{message}");
            return Err(FilterError::Logged(message));
        }
        Err(e) => return Err(e),
    };

    if json {
        sink.write("[\n");
    }
    let mut count = 0;
    let mut has_prev = false;
    for contact in app.mc.contacts() {
        // Ctrl-C at a prompt ends the whole command, summary included
        if app.interrupted.load(Ordering::Relaxed) {
            return Ok(());
        }
        if !filter.matches(&to_value(&contact)) {
            continue;
        }
        count += 1;
        let mut buffer = Sink::buffer();
        if Box::pin(process_contact_chat_line(
            app,
            &contact,
            line,
            false,
            json,
            &mut buffer,
            "",
        ))
        .await
        .map_err(FilterError::Raised)?
        {
            // handled
        } else if line == "remove_contact" {
            let args = vec![line.to_string(), contact.name.clone()];
            Box::pin(process_cmds(app, &args, json, &mut buffer, "")).await;
        } else if line.starts_with("send") || line.starts_with('"') {
            let text = line.strip_prefix("send").map_or(line, |rest| {
                crate::util::py::strip(rest.get(1..).unwrap_or_default())
            });
            super::chat_msg(
                app,
                &Recipient::Contact(Box::new(contact.clone())),
                strip_outer_quotes(text),
                &mut buffer,
                "",
            )
            .await;
        } else if matches!(contact_type(&contact), 2..=4) {
            let args = vec!["cmd".to_string(), contact.name.clone(), line.to_string()];
            Box::pin(process_cmds(app, &args, json, &mut buffer, "")).await;
            // wait for the reply of the command
            app.mc
                .wait_for_event(EventKind::MessagesWaiting, Some(Duration::from_secs(7)))
                .await;
        } else {
            log::error!("Can't send {line} to {}", contact.name);
        }
        let out = buffer.into_string();
        if !out.is_empty() {
            if has_prev {
                if json {
                    sink.write(",");
                }
                sink.write("\n");
            } else {
                has_prev = true;
            }
        }
        sink.write(&out);
    }
    if app.interrupted.load(Ordering::Relaxed) {
        return Ok(());
    }
    if json {
        sink.write(&format!("\n]{end}"));
    } else {
        sink.write(&format!("> {count} matches in contacts{end}"));
    }
    sink.flush();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters() {
        let f = Filter::parse("t=2,u>1d,d", 1_000_000.0).unwrap();
        assert_eq!(f.contact_type, Some(2));
        assert_eq!(f.upd_after, Some(1_000_000.0 - 86_400.0));
        assert_eq!(f.min_hops, Some(0));
        let c =
            json!({"type": 2, "lastmod": 990_000, "out_path_len": 1, "flags": 0, "last_advert": 0});
        assert!(f.matches(&c));
        let old =
            json!({"type": 2, "lastmod": 100, "out_path_len": 1, "flags": 0, "last_advert": 0});
        assert!(!f.matches(&old));

        let flood = Filter::parse("f", 0.0).unwrap();
        assert!(flood.matches(&json!({"out_path_len": -1})));
        assert!(!flood.matches(&json!({"out_path_len": 0})));

        let hops = Filter::parse("h>1", 0.0).unwrap();
        assert_eq!(hops.min_hops, Some(2));
        let fav = Filter::parse("b=1", 0.0).unwrap();
        assert!(fav.matches(&json!({"flags": 3})));
        assert!(!fav.matches(&json!({"flags": 2})));
    }

    #[test]
    fn filter_errors() {
        let logged = |m: &str| Err(FilterError::Logged(m.to_string()));
        assert_eq!(
            Filter::parse("t>2", 0.0),
            logged("Type can only be equals to a value")
        );
        assert_eq!(
            Filter::parse("u=2d", 0.0),
            logged("Time filter can only be < or >")
        );
        assert_eq!(Filter::parse("z", 0.0), logged("Unknown filter z"));
        assert_eq!(Filter::parse("", 0.0), Err(FilterError::Index));
        assert_eq!(Filter::parse("t", 0.0), Err(FilterError::Index));
        assert_eq!(Filter::parse("u<", 0.0), Err(FilterError::Index));
        assert_eq!(
            Filter::parse("t=x", 0.0),
            Err(FilterError::Value(
                "invalid literal for int() with base 10: 'x'".to_string()
            ))
        );
        assert_eq!(Filter::parse("all", 0.0).unwrap(), Filter::default());
    }

    #[test]
    fn permissions() {
        assert_eq!(parse_perm("0x0f"), Ok(15));
        assert_eq!(parse_perm("#15"), Ok(15));
        assert_eq!(parse_perm("0f"), Ok(15));
        assert_eq!(
            parse_perm("zz"),
            Err("invalid literal for int() with base 16: 'zz'".to_string())
        );
    }

    #[test]
    fn contact_commands() {
        assert!(is_contact_command("rp"));
        assert!(is_contact_command("rp login"));
        assert!(is_contact_command("req_clock"));
        assert!(!is_contact_command("rpx"));
        assert!(!is_contact_command("login"));
    }
}
