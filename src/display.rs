//! Display of incoming events and the background event task.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use meshcore::Event;
use serde_json::{Value, json};

use crate::app::{App, Target, to_value, vint, vstr};
use crate::output;
use crate::util::ansi;
use crate::util::json::dumps;
use crate::util::text::escape_ansi;
use crate::util::time::format_print_timestamp;

/// Type names used in displays (`CONTACT_TYPENAMES`).
pub const CONTACT_TYPENAMES: [&str; 5] = ["NONE", "CLI", "REP", "ROOM", "SENS"];

/// Name of a contact type, `t:<n>` for unknown ones.
#[must_use]
pub fn type_name(kind: i64) -> String {
    usize::try_from(kind)
        .ok()
        .and_then(|k| CONTACT_TYPENAMES.get(k))
        .map_or_else(|| format!("t:{kind}"), |s| (*s).to_string())
}

/// Formats a float like Python's `str(float)`.
#[must_use]
pub fn py_float(value: f64) -> String {
    if value.is_nan() {
        "nan".to_string()
    } else if value.is_infinite() {
        if value > 0.0 { "inf" } else { "-inf" }.to_string()
    } else {
        crate::util::json::py_float_repr(value)
    }
}

/// Formats a json number the way Python prints it.
#[must_use]
pub fn py_number(value: &Value) -> String {
    match value {
        Value::Number(n) if n.is_f64() => py_float(n.as_f64().unwrap_or(0.0)),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Current terminal width like `shutil.get_terminal_size()`: `COLUMNS`
/// when it holds a positive number, else the width of the terminal on
/// stdout, else 80.
#[must_use]
pub fn terminal_width() -> usize {
    if let Some(cols) = std::env::var("COLUMNS")
        .ok()
        .and_then(|c| crate::util::py::int(&c, 10).ok())
        .and_then(|c| usize::try_from(c).ok())
        .filter(|c| *c > 0)
    {
        return cols;
    }
    terminal_size::terminal_size_of(std::io::stdout())
        .map(|(w, _)| usize::from(w.0))
        .filter(|w| *w > 0)
        .unwrap_or(80)
}

/// A count as the signed number Python computes with.
fn signed(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Python's `text[0:end]`: a negative `end` drops characters at the back.
fn slice_to(text: &str, end: i64) -> String {
    let count = usize::try_from(end).unwrap_or_else(|_| {
        let back = usize::try_from(end.unsigned_abs()).unwrap_or(usize::MAX);
        text.chars().count().saturating_sub(back)
    });
    text.chars().take(count).collect()
}

/// Formats a received message (`process_event_message`).
///
/// Also remembers the sender as last node (for `to !` and `!` replies).
pub fn format_message(app: &App, data: &Value, json_output: bool, end: &str) -> String {
    if json_output {
        return format!("{}{end}", dumps(data));
    }
    let settings = app.settings().clone();
    let mut path_str = String::new();

    if data.get("sender_timestamp").is_some()
        && let Some(ts) =
            format_print_timestamp(&settings.timestamp, vint(data, "sender_timestamp"))
    {
        path_str.push_str(&ts);
        path_str.push(',');
    }
    if vint(data, "path_len") == 255 {
        path_str.push('D');
    } else {
        path_str.push_str(&vint(data, "path_len").to_string());
    }
    if let Some(snr) = data.get("SNR")
        && settings.print_snr
    {
        let _ = write!(path_str, ",{}dB", py_number(snr));
    }

    let finish = |disp: String| -> String {
        let disp = if settings.color {
            disp
        } else {
            escape_ansi(&disp)
        };
        format!("{disp}{end}")
    };

    match vstr(data, "type").as_str() {
        "PRIV" => {
            let prefix = vstr(data, "pubkey_prefix");
            let contact = app.mc.get_contact_by_key_prefix(&prefix);
            let mut disp = String::new();
            let name = if let Some(c) = &contact {
                app.state().last_node = Some(Target::Contact(Box::new(c.clone())));
                disp.push_str(match crate::app::contact_type(c) {
                    4 => ansi::YELLOW,
                    3 => ansi::CYAN,
                    2 => ansi::MAGENTA,
                    _ => ansi::BLUE,
                });
                c.name.clone()
            } else {
                log::debug!("Unknown contact with pubkey prefix: {prefix}");
                disp.push_str(ansi::RED);
                prefix
            };
            disp.push_str(&name);
            if data.get("signature").is_some() {
                let signature = vstr(data, "signature");
                match app.mc.get_contact_by_key_prefix(&signature) {
                    Some(sender) => {
                        let _ = write!(disp, "/{}{}", ansi::BLUE, sender.name);
                    }
                    None => {
                        let _ = write!(disp, "/{}{signature}", ansi::RED);
                    }
                }
            }
            let _ = write!(disp, " {}({path_str})", ansi::ORANGE);
            disp.push_str(if vint(data, "txt_type") == 1 {
                ansi::LIGHT_GRAY
            } else {
                ansi::END
            });
            let _ = write!(disp, ": {}", vstr(data, "text"));
            finish(disp)
        }
        "CHAN" => {
            let disp = channel_prefix(app, data, &path_str);
            finish(format!("{disp}{}: {}", ansi::END, vstr(data, "text")))
        }
        "DATA" => {
            if !settings.display_channel_data {
                return String::new();
            }
            let disp = channel_prefix(app, data, &path_str);
            finish(format!("{disp}{} {}", ansi::END, dumps(data)))
        }
        _ => format!("{}{end}", dumps(data)),
    }
}

/// Channel name and path part of a channel message; remembers the channel.
fn channel_prefix(app: &App, data: &Value, path_str: &str) -> String {
    let idx = u8::try_from(vint(data, "channel_idx")).unwrap_or(0);
    let path_str = format!("{}({path_str}){}", ansi::YELLOW, ansi::END);
    let name = match app.cached_channel_name(idx) {
        Some(name) => name,
        None if idx == 0 => "public".to_string(),
        None => format!("ch{idx}"),
    };
    app.state().last_node = Some(Target::Channel {
        name: name.clone(),
        index: i64::from(idx),
    });
    format!("{}{name} {path_str}", ansi::GREEN)
}

/// Handles an incoming message event (`handle_message`); `event_type` is
/// the name of its `EventType` in the reference.
async fn handle_message(app: &App, payload: Value, event_type: &str) {
    let json_output = app.settings().json_msgs;
    // sender names come from the contact list, which json leaves out
    if !json_output {
        let _ = app.mc.ensure_contacts(false).await;
    }
    output::print(&format_message(app, &payload, json_output, "\n"));
    // the message is logged before the handlers get it; a log that cannot
    // be written ends the reference's event handler there
    if let Err(e) = app.log_message(payload.clone()) {
        log::error!("Error in event handler for EventType.{event_type}: {e}");
        return;
    }
    if vstr(&payload, "type") == "DATA" {
        app.handlers.enqueue("chan_data", &payload);
    } else {
        app.handlers.enqueue("msgs", &payload);
    }
}

/// Handles a received packet log (`handle_log_rx`).
fn handle_log_rx(app: &App, payload: &Value) {
    let settings = app.settings().clone();
    let payload_type = vint(payload, "payload_type");
    let mut lines = String::new();

    if payload_type == 0x05 && settings.channel_echoes {
        let mut chan_name = vstr(payload, "chan_name");
        let mut message = None;
        if payload.get("message").is_some() {
            message = Some(vstr(payload, "message"));
        } else if settings.echo_unk_chans || !chan_name.is_empty() {
            if chan_name.is_empty() {
                chan_name = vstr(payload, "chan_hash");
            }
            message = Some(if payload.get("crypted").is_some() {
                vstr(payload, "crypted")
            } else {
                "-- unhandled --".to_string()
            });
        }
        if let Some(message) = message.filter(|_| !chan_name.is_empty()) {
            // computed with signs like the reference: in a narrow terminal
            // or with a long path the text loses characters at the back
            let width = signed(terminal_width());
            let path = vstr(payload, "path");
            let path_len = signed(path.chars().count());
            let cars = width - 13 - path_len - signed(chan_name.chars().count()) - 1;
            let dispmsg = slice_to(&message.replace('\n', ""), cars);
            let pad = usize::try_from(cars - signed(dispmsg.chars().count())).unwrap_or(0);
            let snr = payload.get("snr").and_then(Value::as_f64).unwrap_or(0.0);
            let rssi = vint(payload, "rssi");
            // with a COLUMNS value beyond the memory the reference fails
            // building the padding and drops the rest of the event
            if lines.try_reserve(pad).is_err() {
                log::error!("Error in event handler for EventType.RX_LOG_DATA: ");
                return;
            }
            let _ = write!(
                lines,
                "{}{chan_name} {}{dispmsg}",
                ansi::LIGHT_GRAY,
                ansi::DGREEN
            );
            lines.extend(std::iter::repeat_n(' ', pad));
            let _ = writeln!(
                lines,
                " {}{}G{}[{path}]{}{snr:6.2}{rssi:4}{}",
                ansi::START,
                width - 11 - path_len,
                ansi::YELLOW,
                ansi::LIGHT_GRAY,
                ansi::END
            );
        }
    } else if payload_type == 0x04 && settings.advert_echoes {
        let key = vstr(payload, "adv_key");
        let name = if payload.get("adv_name").is_some() {
            vstr(payload, "adv_name")
        } else {
            app.mc
                .get_contact_by_key_prefix(&key)
                .map_or_else(|| key.chars().take(12).collect(), |c| c.name.clone())
        };
        let ts_str = format_print_timestamp(&settings.timestamp, vint(payload, "adv_timestamp"))
            .map(|t| format!(" at {t}"))
            .unwrap_or_default();
        let mut txt = format!(
            "{}Advert for{} {name}{}/{}{ts_str}{}",
            ansi::LIGHT_GRAY,
            ansi::END,
            ansi::GREEN,
            type_name(vint(payload, "adv_type")),
            ansi::END
        );
        if let Some(lat) = payload.get("adv_lat").filter(|v| !v.is_null()) {
            let _ = write!(
                txt,
                " {}coords: {},{}",
                ansi::LIGHT_GRAY,
                py_number(lat),
                py_number(payload.get("adv_lon").unwrap_or(&Value::Null))
            );
        }
        let snr = payload.get("snr").and_then(Value::as_f64).unwrap_or(0.0);
        let _ = writeln!(
            txt,
            " {}path: [{}] {}snr: {snr:.2}dB{}",
            ansi::YELLOW,
            vstr(payload, "path"),
            ansi::LIGHT_GRAY,
            ansi::END
        );
        lines.push_str(&txt);
    }

    if !lines.is_empty() {
        output::print(&if settings.color {
            lines
        } else {
            escape_ansi(&lines)
        });
    }

    // the reference turns pkt_payload into hex here; the library's payload
    // holds it as hex already
    if settings.json_log_rx {
        output::println(&dumps(payload));
    }
    app.handlers.enqueue("rxlog", payload);
}

/// Prints a note about an advert (`handle_advert`).
fn handle_advert(app: &App, key: &str) {
    let settings = app.settings().clone();
    if !settings.print_adverts {
        return;
    }
    let msg = if settings.json_msgs {
        dumps(&json!({"event": "advert", "public_key": key}))
    } else {
        let name = app
            .mc
            .get_contact_by_key_prefix(key)
            .map_or_else(|| "<Unknown Contact>".to_string(), |c| c.name.clone());
        format!("Got advert from {name} [{key}]")
    };
    output::println(&msg);
}

/// Prints a note about a path update (`handle_path_update`).
fn handle_path_update(app: &App, key: &str) {
    let settings = app.settings().clone();
    if !settings.print_path_updates {
        return;
    }
    let msg = if settings.json_msgs {
        dumps(&json!({"event": "path_update", "public_key": key}))
    } else {
        let name = app
            .mc
            .get_contact_by_key_prefix(key)
            .map_or_else(|| "<Unknown Contact>".to_string(), |c| c.name.clone());
        format!("Got path update for {name} [{key}]")
    };
    output::println(&msg);
}

/// Prints a note about a new pending contact (`handle_new_contact`).
fn handle_new_contact(app: &App, contact: &Value) {
    let settings = app.settings().clone();
    if !settings.print_new_contacts {
        return;
    }
    let msg = if settings.json_msgs {
        dumps(&json!({"event": "new_contact", "contact": contact}))
    } else {
        format!(
            "New pending contact {} [{}]",
            vstr(contact, "adv_name"),
            vstr(contact, "public_key")
        )
    };
    output::println(&msg);
}

/// Starts the task that reacts to unsolicited events.
pub fn spawn_event_task(app: Arc<App>) -> tokio::task::JoinHandle<()> {
    let mut subscription = app.mc.subscribe();
    tokio::spawn(async move {
        while let Some(event) = subscription.recv().await {
            match &event {
                Event::Advertisement(key) => handle_advert(&app, &key.to_string()),
                Event::PathUpdate(key) => handle_path_update(&app, &key.to_string()),
                Event::NewContact(_) => handle_new_contact(&app, &to_value(&event)),
                Event::RxLog(_) => handle_log_rx(&app, &to_value(&event)),
                Event::ContactMessage(_) | Event::ChannelMessage(_) | Event::ChannelData(_)
                    if app.msgs_subscribed.load(Ordering::Relaxed) =>
                {
                    let event_type = match &event {
                        Event::ContactMessage(_) => "CONTACT_MSG_RECV",
                        Event::ChannelMessage(_) => "CHANNEL_MSG_RECV",
                        _ => "CHANNEL_DATA_RECV",
                    };
                    handle_message(&app, to_value(&event), event_type).await;
                    app.messages_shown.send_modify(|n| *n = n.wrapping_add(1));
                }
                _ => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_like_python() {
        assert_eq!(py_float(5.0), "5.0");
        assert_eq!(py_float(12.25), "12.25");
        assert_eq!(py_float(-3.5), "-3.5");
        assert_eq!(py_number(&json!(7)), "7");
        assert_eq!(py_number(&json!(7.0)), "7.0");
    }

    #[test]
    fn slices_like_python() {
        assert_eq!(slice_to("abcdef", 3), "abc");
        assert_eq!(slice_to("abcdef", 10), "abcdef");
        assert_eq!(slice_to("abcdef", 0), "");
        assert_eq!(slice_to("abcdef", -2), "abcd");
        assert_eq!(slice_to("abcdef", -9), "");
        assert_eq!(slice_to("äöü", -1), "äö");
    }

    #[test]
    fn type_names() {
        assert_eq!(type_name(2), "REP");
        assert_eq!(type_name(9), "t:9");
        assert_eq!(type_name(-1), "t:-1");
    }
}
