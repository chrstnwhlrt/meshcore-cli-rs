//! Messaging commands.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use meshcore::{Contact, Destination, EventKind, RetryOptions};
use serde_json::{Value, json};

use super::{CmdError, CmdResult, Ctx};
use crate::app::{App, to_value};
use crate::display::format_message;

/// Adds the fields of a sent message and writes it to the message log.
fn log_sent(app: &App, mut sent: Value, kind: &str, recipient: &str, text: &str, txt_type: u8) {
    if let Value::Object(map) = &mut sent {
        map.insert("type".into(), json!(kind));
        map.insert("recipient".into(), json!(recipient));
        map.insert("text".into(), json!(text));
        map.insert("txt_type".into(), json!(txt_type));
        map.insert("sender".into(), json!(app.self_name()));
        map.insert("name".into(), json!(recipient));
    }
    app.log_message(sent);
}

/// Where a direct message goes: a contact or a raw key prefix.
pub enum Recipient {
    /// A known contact.
    Contact(Box<Contact>),
    /// A public key (prefix) given in hex.
    Key(Vec<u8>),
}

impl Recipient {
    fn destination(&self) -> Destination {
        match self {
            Self::Contact(c) => Destination::from(c.as_ref()),
            Self::Key(bytes) => Destination::from_hex(&crate::app::hex(bytes))
                .unwrap_or_else(|_| Destination::from(c_prefix(bytes))),
        }
    }
}

fn c_prefix(bytes: &[u8]) -> [u8; 6] {
    let mut prefix = [0u8; 6];
    for (dst, src) in prefix.iter_mut().zip(bytes) {
        *dst = *src;
    }
    prefix
}

/// Sends a message without waiting (`send_msg`); logs it on success.
pub async fn send_msg(app: &App, to: &Recipient, text: &str) -> Result<Value, meshcore::Error> {
    let sent = app.mc.send_msg(to.destination(), text, None, 0).await?;
    let value = to_value(&sent);
    if let Recipient::Contact(contact) = to {
        log_sent(app, value.clone(), "SENT_MSG", &contact.name, text, 0);
    }
    Ok(value)
}

/// Sends a message with retries and waits for the ack (`msg_ack`).
///
/// Returns true when the message was acknowledged.
///
/// # Errors
///
/// A text the device cannot take (too long, empty or with a NUL
/// character); nothing was sent.
pub async fn msg_ack(app: &App, to: &Recipient, text: &str) -> Result<bool, meshcore::Error> {
    let (max_attempts, flood_after, max_flood_attempts) = {
        let s = app.settings();
        (s.max_attempts, s.flood_after, s.max_flood_attempts)
    };
    let timeout = match to {
        Recipient::Contact(contact) => app.timeout_for(&contact.public_key),
        Recipient::Key(_) => None,
    };
    let options = RetryOptions {
        max_attempts,
        flood_after,
        max_flood_attempts,
        timeout,
        ..RetryOptions::default()
    };
    match app
        .mc
        .send_msg_with_retry(to.destination(), text, options)
        .await
    {
        Ok(Some(sent)) => {
            if let Recipient::Contact(contact) = to {
                log_sent(app, to_value(&sent), "SENT_MSG", &contact.name, text, 0);
            }
            Ok(true)
        }
        Ok(None) => Ok(false),
        Err(e @ meshcore::Error::InvalidArgument(_)) => Err(e),
        Err(e) => {
            log::debug!("send_msg_with_retry: {e}");
            Ok(false)
        }
    }
}

/// Sends a command to a node (`send_cmd`); logs it on success.
pub async fn send_cmd_logged(
    app: &App,
    contact: &Contact,
    cmd: &str,
) -> Result<Value, meshcore::Error> {
    let sent = app
        .mc
        .send_cmd(
            Destination::from(contact),
            cmd,
            None,
            Some(contact.contact_type),
        )
        .await?;
    let value = to_value(&sent);
    log_sent(app, value.clone(), "SENT_CMD", &contact.name, cmd, 1);
    Ok(value)
}

/// Sends a channel message (`send_chan_msg`); logs it on success.
pub async fn send_chan_msg(app: &App, index: u8, text: &str) -> Result<Value, meshcore::Error> {
    app.mc.send_chan_msg(index, text, None).await?;
    let chan_name = app
        .cached_channel_name(index)
        .unwrap_or_else(|| format!("channel {index}"));
    // key order of the reference: type first, then the channel
    let sent = json!({"type": "SENT_CHAN", "channel_idx": index, "chan_name": chan_name});
    log_sent(app, sent, "SENT_CHAN", &chan_name, text, 0);
    Ok(json!({}))
}

/// How long a fetched message may take to show up on the event task.
const SHOW_WAIT: Duration = Duration::from_secs(2);

/// Shows incoming messages from now on, as json when `json` is set
/// (`subscribe_to_msgs`).
///
/// Messages already waiting are read and shown before this returns, as
/// `meshcore_py` does with the first of them, so they come before the output
/// of the next command; later ones are fetched when the device announces
/// them.
pub async fn subscribe_to_msgs(app: &Arc<App>, json: bool) {
    let _ = app.mc.ensure_contacts(false).await;
    app.settings().json_msgs = json;
    app.msgs_subscribed.store(true, Ordering::Relaxed);
    let mut shown = app.messages_shown.subscribe();
    while let Ok(Some(_)) = app.mc.get_msg().await {
        // the event task shows it
        let _ = tokio::time::timeout(SHOW_WAIT, shown.changed()).await;
        shown.borrow_and_update();
    }
    app.mc.start_auto_message_fetching();
}

/// `msg [-a] <dest> <text>`.
pub async fn msg(c: &mut Ctx<'_>) -> CmdResult {
    let mut argnum = 2;
    let mut wait_ack = false;
    let mut args = &c.cmds[1..];
    while let Some(first) = args.first() {
        let mut chars = first.chars();
        match chars.next() {
            None => return Err(CmdError::Params),
            Some('-') => {}
            Some(_) => break,
        }
        argnum += 1;
        if chars.next() == Some('a') {
            wait_ack = true;
        }
        args = &args[1..];
    }
    let dest_arg = args.first().ok_or(CmdError::Params)?.clone();

    let mut recipient = None;
    if dest_arg.chars().count() >= 12
        && let Ok(bytes) = crate::util::py::fromhex(&dest_arg)
    {
        recipient = Some(Recipient::Key(bytes));
    }
    if recipient.is_none() {
        recipient = c
            .app
            .contact_from_arg(&dest_arg)
            .await
            .map(|contact| Recipient::Contact(Box::new(contact)));
    }
    let Some(recipient) = recipient else {
        let text = format!("Unknown destination {dest_arg}");
        c.error(
            &json!({"error": "unknown destination", "dest": dest_arg}),
            &text,
        );
        return Ok(argnum);
    };
    // like the reference, the text is read once the destination is known
    let text = args.get(1).ok_or(CmdError::Params)?.clone();

    if wait_ack {
        match msg_ack(c.app, &recipient, &text).await {
            Ok(true) => {}
            Ok(false) => c.error(&json!({"ack": false}), "No ack received"),
            Err(e) => c.line(&format!("Error sending message: {e}")),
        }
    } else {
        match send_msg(c.app, &recipient, &text).await {
            Err(e) => c.line(&format!("Error sending message: {e}")),
            Ok(sent) => {
                if c.json {
                    c.json_indent(&sent);
                }
            }
        }
    }
    Ok(argnum)
}

/// `chan <n|name> <text>`.
pub async fn chan(c: &mut Ctx<'_>) -> CmdResult {
    let chan = c.arg(1)?.to_string();
    // like the reference: a name is looked up first, then the text is read,
    // and a number beyond a byte fails last, when it is packed
    let named = if super::channels::is_channel_number(&chan) {
        None
    } else {
        let Some(entry) = c.app.channel_by_name(&chan).await else {
            log::error!("Invalid channel");
            return Ok(2);
        };
        Some(entry.index)
    };
    let text = c.arg(2)?.to_string();
    let index = match named {
        Some(index) => index,
        None => super::channels::slot_number(&chan)?,
    };
    match send_chan_msg(c.app, index, &text).await {
        Err(e) => c.line(&format!("Error sending message: {e}")),
        Ok(payload) => {
            if c.json {
                c.json_indent(&payload);
            }
        }
    }
    Ok(2)
}

/// `public <text>`: message on channel 0.
pub async fn public(c: &mut Ctx<'_>) -> CmdResult {
    let text = c.arg(1)?.to_string();
    match send_chan_msg(c.app, 0, &text).await {
        Err(e) => c.line(&format!("Error sending message: {e}")),
        Ok(payload) => {
            if c.json {
                c.json_indent(&payload);
            }
        }
    }
    Ok(1)
}

/// `chan_data [-p<path>[:mode]] <chan> <type> <hex>`.
pub async fn chan_data(c: &mut Ctx<'_>) -> CmdResult {
    let mut argnum = 3;
    let mut hash_mode = 0i64;
    let mut path: Option<Vec<u8>> = None;
    let mut args = &c.cmds[1..];
    while let Some(arg) = super::next_option(args)? {
        if let Some(spec) = arg.strip_prefix("-p") {
            // path_str.split(":")[0] and [1] of the reference
            let mut parts = spec.split(':');
            let hops = parts.next().unwrap_or_default();
            if let Some(mode) = parts.next() {
                hash_mode = crate::util::py::int(mode, 10).map_err(CmdError::Failed)?;
            }
            path = Some(if hops == "0" {
                Vec::new()
            } else {
                crate::util::py::fromhex(hops).map_err(CmdError::Failed)?
            });
        }
        argnum += 1;
        args = &args[1..];
    }
    let chan = args.first().ok_or(CmdError::Params)?.clone();
    // like the reference, type and payload are only read for a known channel
    let index = if super::channels::is_channel_number(&chan) {
        Some(crate::util::py::int(&chan, 10).map_err(CmdError::Failed)?)
    } else {
        c.app
            .channel_by_name(&chan)
            .await
            .map(|entry| i64::from(entry.index))
    };
    let Some(index) = index else {
        log::error!("Invalid channel");
        return Ok(argnum);
    };
    let data_type = parse_int(args.get(1).ok_or(CmdError::Params)?)?;
    let payload =
        crate::util::py::fromhex(args.get(2).ok_or(CmdError::Params)?).map_err(CmdError::Failed)?;
    // the checks of meshcore_py's send_channel_data, with its messages
    let failed = |text: &str| CmdError::Failed(text.to_string());
    let index = u8::try_from(index).map_err(|_| failed("chan must fit in one byte (0-255)"))?;
    let data_type = u16::try_from(data_type)
        .ok()
        .filter(|t| *t >= 1)
        .ok_or_else(|| failed("data_type must be between 0x0001 and 0xFFFF"))?;
    if payload.len() > meshcore::MAX_CHANNEL_DATA_LENGTH {
        return Err(CmdError::Failed(format!(
            "payload must be at most {} bytes",
            meshcore::MAX_CHANNEL_DATA_LENGTH
        )));
    }
    if path.as_ref().is_some_and(|p| p.len() >= 0xFF) {
        return Err(failed("path must be shorter than 255 bytes"));
    }
    // then the mode divides the path length and is packed with it
    let hash_mode = match (&path, hash_mode) {
        (None, _) => 0,
        (Some(_), -1) => return Err(failed("division by zero")),
        (Some(_), mode) => u8::try_from(mode)
            .ok()
            .filter(|mode| *mode <= 3)
            .ok_or_else(|| failed("bytes must be in range(0, 256)"))?,
    };
    match c
        .app
        .mc
        .send_channel_data(index, data_type, &payload, path.as_deref(), hash_mode)
        .await
    {
        Err(meshcore::Error::InvalidArgument(e)) => return Err(CmdError::Failed(e)),
        Err(e) => c.line(&format!("Error sending chan data: {e}")),
        Ok(()) => {
            if c.json {
                c.json_indent(&json!({}));
            }
        }
    }
    Ok(argnum)
}

/// Parses an integer like Python's `int(x, 0)`.
fn parse_int(text: &str) -> Result<i64, CmdError> {
    crate::util::py::int(text, 0).map_err(CmdError::Failed)
}

/// `cmd <name> <command>`: sends a command to a node.
pub async fn cmd(c: &mut Ctx<'_>) -> CmdResult {
    let name = c.arg(1)?.to_string();
    match c.app.contact_from_arg(&name).await {
        None => {
            let text = format!("Unknown destination {name}");
            c.error(
                &json!({"error": "contact destination", "dest": name}),
                &text,
            );
        }
        Some(contact) => {
            // read once the contact is known, as in the reference
            let command = c.arg(2)?.to_string();
            match send_cmd_logged(c.app, &contact, &command).await {
                Err(e) => c.line(&format!("Error sending cmd: {e}")),
                Ok(sent) => {
                    if c.json {
                        c.json_indent(&sent);
                    }
                }
            }
        }
    }
    Ok(2)
}

/// Fetches one message and formats it; `None` when there is none.
async fn next_message(c: &mut Ctx<'_>, end: &str) -> Option<String> {
    match c.app.mc.get_msg().await {
        Ok(Some(message)) => {
            // sender names come from the contact list, which json leaves out
            if !c.json {
                let _ = c.app.mc.ensure_contacts(false).await;
            }
            Some(format_message(c.app, &to_value(&message), c.json, end))
        }
        Ok(None) => {
            log::debug!("No more messages");
            None
        }
        Err(e) => {
            // the payload of meshcore_py's error event, as the reference
            // prints it
            let payload = super::py_error_payload(&e)
                .map_or_else(|| e.to_string(), |p| crate::util::json::py_repr(&p));
            log::error!("Error retrieving messages: {payload}");
            None
        }
    }
}

/// `recv` / `r`: reads the next message.
pub async fn recv(c: &mut Ctx<'_>) -> CmdResult {
    // process_event_message with its own end of line, as in the reference
    if let Some(text) = next_message(c, "\n").await {
        c.emit(&text);
    }
    Ok(0)
}

/// `sync_msgs` / `sm`: reads all waiting messages.
pub async fn sync_msgs(c: &mut Ctx<'_>) -> CmdResult {
    let end = if c.json { "" } else { "\n" };
    if c.json {
        c.emit("[");
    }
    let mut first = true;
    while let Some(text) = next_message(c, end).await {
        if !first && c.json {
            c.emit(",\n");
        }
        c.emit(&text);
        first = false;
    }
    if c.json {
        let end = c.end.to_string();
        c.emit(&format!("]{end}"));
    }
    Ok(0)
}

/// `wait_msg`, `trywait_msg <secs>`, `wmt8`: waits for a message and reads it.
pub async fn wait_msg(c: &mut Ctx<'_>, timeout: Option<f64>) -> Result<(), CmdError> {
    let duration = timeout.map(crate::util::time::duration_from_secs);
    let event = c
        .app
        .mc
        .wait_for_event(EventKind::MessagesWaiting, duration)
        .await;
    if event.is_none() {
        if timeout.is_none() {
            c.line("Timeout waiting msg");
        }
        return Ok(());
    }
    if let Some(text) = next_message(c, "\n").await {
        c.emit(&text);
    }
    Ok(())
}

/// `wait_ack` / `wa`: waits up to 5 seconds for an ack.
pub async fn wait_ack(c: &mut Ctx<'_>) -> CmdResult {
    match c
        .app
        .mc
        .wait_for_event(EventKind::Ack, Some(Duration::from_secs(5)))
        .await
    {
        None => c.error(
            &json!({"error": "Timeout waiting ack"}),
            "Timeout waiting ack",
        ),
        Some(event) => {
            if c.json {
                c.json_indent(&to_value(&event));
            } else {
                c.line("Msg acked");
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_int_parsing() {
        assert_eq!(parse_int("0x10").unwrap(), 16);
        assert_eq!(parse_int("12").unwrap(), 12);
        assert_eq!(parse_int("0b11").unwrap(), 3);
        assert!(parse_int("zz").is_err());
    }

    #[test]
    fn prefixes() {
        assert_eq!(c_prefix(&[1, 2, 3]), [1, 2, 3, 0, 0, 0]);
        assert_eq!(c_prefix(&[1, 2, 3, 4, 5, 6, 7]), [1, 2, 3, 4, 5, 6]);
    }
}
