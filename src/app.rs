//! Application state shared by commands, event handlers and the chat mode.
//!
//! The reference keeps its settings in function attributes and globals
//! (`process_event_message.color`, `msg_ack.max_attempts`, `ALIASES` ...).
//! Here they live in [`Settings`] and [`State`] inside [`App`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use meshcore::{Contact, MeshCore, PublicKey};
use serde::Serialize;
use serde_json::Value;

use crate::handlers::Handlers;
use crate::paths;

/// Converts a library value into the payload dict the reference prints.
pub fn to_value<T: Serialize + ?Sized>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// String field of a payload dict (empty when missing).
#[must_use]
pub fn vstr(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Integer field of a payload dict (0 when missing, floats truncated like
/// Python's `int()`).
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn vint(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
}

/// Float field of a payload dict (0.0 when missing).
#[must_use]
pub fn vf64(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Settings changeable with `set` (display and behaviour).
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // independent on/off switches of the reference
pub struct Settings {
    /// Show SNR in received messages (`print_snr`).
    pub print_snr: bool,
    /// ANSI colors (`color`, `-c`).
    pub color: bool,
    /// Show received channel data (`display_channel_data`).
    pub display_channel_data: bool,
    /// Timestamp display: "", "on", "off" or a strftime pattern.
    pub timestamp: String,
    /// Dump RX log packets as json (`json_log_rx`).
    pub json_log_rx: bool,
    /// Show repeats of channel messages (`channel_echoes`).
    pub channel_echoes: bool,
    /// Show repeats of adverts (`advert_echoes`).
    pub advert_echoes: bool,
    /// Also echo channels that cannot be decoded (`echo_unk_chans`).
    pub echo_unk_chans: bool,
    /// Show received adverts (`print_adverts`).
    pub print_adverts: bool,
    /// Show path updates (`print_path_updates`).
    pub print_path_updates: bool,
    /// Show new pending contacts (`print_new_contacts`).
    pub print_new_contacts: bool,
    /// Show incoming messages as json (`json_msgs`, `-j`).
    pub json_msgs: bool,
    /// Plain prompt without inversion (`classic_prompt`, `-C`).
    pub classic: bool,
    /// Arrow at the end of the prompt (`arrow_head`).
    pub arrow_head: String,
    /// Start of the slash between node and contact (`slash_start`).
    pub slash_start: String,
    /// End of the slash (`slash_end`).
    pub slash_end: String,
    /// Invert the slash colors (`invert_slash`).
    pub invert_slash: bool,
    /// Default output format of `lc`.
    pub lc_output_format: String,
    /// Default sort order of `lc`.
    pub lc_sort_order: String,
    /// Attempts when sending chat messages (`max_attempts`).
    pub max_attempts: u32,
    /// Attempt after which the message goes flood (`flood_after`).
    pub flood_after: u32,
    /// Attempts in flood mode (`max_flood_attempts`).
    pub max_flood_attempts: u32,
}

impl Default for Settings {
    fn default() -> Self {
        let classic = cfg!(any(target_os = "macos", target_os = "windows"));
        Self {
            print_snr: false,
            color: true,
            display_channel_data: true,
            timestamp: String::new(),
            json_log_rx: false,
            channel_echoes: false,
            advert_echoes: false,
            echo_unk_chans: false,
            print_adverts: false,
            print_path_updates: false,
            print_new_contacts: false,
            json_msgs: false,
            classic,
            arrow_head: "\u{e0b0}".to_string(),
            slash_start: crate::util::ansi::GRAY_BACK.to_string(),
            slash_end: format!("\u{e0ba}{}", crate::util::ansi::RESET_BACK),
            invert_slash: false,
            lc_output_format: "tkah".to_string(),
            lc_sort_order: String::new(),
            max_attempts: 3,
            flood_after: 2,
            max_flood_attempts: 1,
        }
    }
}

/// A chat destination: a contact or a channel (`{"type": 0, "chan_nb": n}`).
#[derive(Debug, Clone)]
pub enum Target {
    /// A contact of the device.
    Contact(Box<Contact>),
    /// A channel slot.
    Channel {
        /// Channel name as shown on the prompt.
        name: String,
        /// Channel slot (`chan_nb`); negative after `to ch-<n>`, which the
        /// reference counts from the end of the channel list, and then no
        /// message can be sent to it.
        index: i64,
    },
}

impl Target {
    /// The displayed name (`adv_name`).
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Contact(c) => c.name.clone(),
            Self::Channel { name, .. } => name.clone(),
        }
    }

    /// The type number of the reference (0 for channels).
    #[must_use]
    pub fn kind(&self) -> u8 {
        match self {
            Self::Contact(c) => contact_type(c),
            Self::Channel { .. } => 0,
        }
    }

    /// The contact, if this is one.
    #[must_use]
    pub fn contact(&self) -> Option<&Contact> {
        match self {
            Self::Contact(c) => Some(c),
            Self::Channel { .. } => None,
        }
    }
}

/// Type number of a contact (1 client, 2 repeater, 3 room, 4 sensor).
#[must_use]
pub fn contact_type(contact: &Contact) -> u8 {
    u8::try_from(vint(&to_value(contact), "type")).unwrap_or(0)
}

/// A channel slot as returned by `get_channel`, with the secret in hex.
#[derive(Debug, Clone)]
pub struct ChannelEntry {
    /// Slot number.
    pub index: u8,
    /// Channel name, empty for unused slots.
    pub name: String,
    /// The payload dict printed by `get_channel(s)`.
    pub value: Value,
}

impl ChannelEntry {
    /// Builds an entry from a channel info payload.
    #[must_use]
    pub fn from_value(value: Value) -> Self {
        let index = u8::try_from(vint(&value, "channel_idx")).unwrap_or(0);
        let name = vstr(&value, "channel_name");
        Self { index, name, value }
    }
}

/// Session state that is not a user setting.
#[derive(Debug, Default)]
pub struct State {
    /// Sender of the last displayed message (`process_event_message.last_node`).
    pub last_node: Option<Target>,
    /// Aliases in definition order.
    pub aliases: Vec<(String, String)>,
    /// Nodes found by the last `node_discover` that are not contacts.
    pub discovered: Vec<Value>,
    /// Per contact timeouts in seconds, keyed by public key (`contact_timeout`).
    pub timeouts: HashMap<String, f64>,
    /// Per channel scopes from `~/.config/meshcore/scopes`.
    pub scopes: HashMap<String, String>,
    /// True once the device reported that it has no scopes.
    pub no_scope: bool,
    /// Scope currently set on the device.
    pub current_scope: Option<String>,
    /// Channel slots, fetched on first use.
    pub channels: Option<Vec<ChannelEntry>>,
    /// Message log file (`<name>.msgs`).
    pub message_log: Option<PathBuf>,
    /// Names of the device's custom variables (for completion).
    pub custom_vars: Vec<String>,
}

impl State {
    /// Sets or replaces an alias, keeping its position.
    pub fn set_alias(&mut self, key: &str, value: &str) {
        if let Some(entry) = self.aliases.iter_mut().find(|(k, _)| k == key) {
            entry.1 = value.to_string();
        } else {
            self.aliases.push((key.to_string(), value.to_string()));
        }
    }

    /// Looks up an alias.
    #[must_use]
    pub fn alias(&self, key: &str) -> Option<String> {
        self.aliases
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    /// Removes an alias.
    pub fn remove_alias(&mut self, key: &str) {
        self.aliases.retain(|(k, _)| k != key);
    }

    /// Aliases as a json object in definition order.
    #[must_use]
    pub fn aliases_value(&self) -> Value {
        let mut map = serde_json::Map::new();
        for (k, v) in &self.aliases {
            map.insert(k.clone(), Value::String(v.clone()));
        }
        Value::Object(map)
    }
}

/// Reads `~/.config/meshcore/scopes` (`<channel> <scope>` per line).
fn load_scopes() -> HashMap<String, String> {
    let mut scopes = HashMap::new();
    let path = paths::scopes_file();
    if !path.exists() {
        return scopes;
    }
    let content = match crate::util::py::read_text(&path, crate::util::py::TextRead::Lines) {
        Ok(content) => content,
        // the reference fails with these errors at startup
        Err(e) => {
            log::error!(
                "while reading scope file {}: {} ... proceeding",
                path.display(),
                e.message(&path.display().to_string())
            );
            return scopes;
        }
    };
    for line in crate::util::py::lines(&content) {
        let line = crate::util::py::strip(line);
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let parts: Vec<&str> = crate::util::py::split(line).collect();
        if let [channel, scope] = parts.as_slice() {
            scopes.insert((*channel).to_string(), (*scope).to_string());
        } else {
            // the ValueError of `channel, scope = line.split()`
            let problem = if parts.len() < 2 {
                "not enough"
            } else {
                "too many"
            };
            log::error!(
                "while reading scope file {}: {problem} values to unpack (expected 2, got {}) ... proceeding",
                path.display(),
                parts.len()
            );
        }
    }
    scopes
}

/// Everything a command needs.
pub struct App {
    /// The device connection.
    pub mc: MeshCore,
    settings: Mutex<Settings>,
    state: Mutex<State>,
    /// Attached handler processes.
    pub handlers: Handlers,
    /// Incoming messages are displayed (`msgs_subscribe` or chat mode).
    pub msgs_subscribed: AtomicBool,
    /// Counts the messages the event task has shown, so that a command can
    /// wait until the messages it fetched are on screen.
    pub messages_shown: tokio::sync::watch::Sender<u64>,
    /// The companion understands `cli` commands (protocol 14 and later).
    pub has_cli_cmd: AtomicBool,
    /// Ctrl-C at a prompt that cannot report it itself (`login` in the
    /// chat), Python's `KeyboardInterrupt`: the command handler around it
    /// reports "Cancelled", or else the chat ends.
    pub interrupted: AtomicBool,
}

impl App {
    /// Creates the application state for a connected device.
    #[must_use]
    pub fn new(mc: MeshCore, settings: Settings) -> Self {
        let state = State {
            scopes: load_scopes(),
            ..State::default()
        };
        Self {
            mc,
            settings: Mutex::new(settings),
            state: Mutex::new(state),
            handlers: Handlers::default(),
            msgs_subscribed: AtomicBool::new(false),
            messages_shown: tokio::sync::watch::Sender::new(0),
            has_cli_cmd: AtomicBool::new(false),
            interrupted: AtomicBool::new(false),
        }
    }

    /// Locks the settings.
    pub fn settings(&self) -> MutexGuard<'_, Settings> {
        self.settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Locks the session state.
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// True when the device has a `cli` command.
    #[must_use]
    pub fn has_cli(&self) -> bool {
        self.has_cli_cmd.load(Ordering::Relaxed)
    }

    /// Name of the connected node.
    #[must_use]
    pub fn self_name(&self) -> String {
        self.mc
            .self_info()
            .map(|i| vstr(&to_value(&i), "name"))
            .unwrap_or_default()
    }

    /// The node's own info as payload dict.
    #[must_use]
    pub fn self_info_value(&self) -> Value {
        self.mc.self_info().map_or(Value::Null, |i| to_value(&i))
    }

    /// Finds a contact by key prefix or name (`get_contact_from_arg`).
    pub async fn contact_from_arg(&self, arg: &str) -> Option<Contact> {
        let _ = self.mc.ensure_contacts(false).await;
        if !arg.is_empty()
            && arg.chars().all(|c| c.is_ascii_hexdigit())
            && let Some(contact) = self.mc.get_contact_by_key_prefix(arg)
        {
            return Some(contact);
        }
        self.mc.get_contact_by_name(arg)
    }

    /// A contact as the reference prints it: its payload dict, which also
    /// holds the timeout `contact_timeout` stored in it.
    #[must_use]
    pub fn contact_value(&self, contact: &Contact) -> Value {
        let mut value = to_value(contact);
        if let Some(timeout) = self
            .state()
            .timeouts
            .get(&contact.public_key.to_string())
            .copied()
            && let Value::Object(map) = &mut value
        {
            map.insert("timeout".into(), Value::from(timeout));
        }
        value
    }

    /// Timeout set with `contact_timeout` (None = suggested timeout).
    #[must_use]
    pub fn timeout_for(&self, key: &PublicKey) -> Option<Duration> {
        self.state()
            .timeouts
            .get(&key.to_string())
            .copied()
            .filter(|t| *t > 0.0)
            .map(crate::util::time::duration_from_secs)
    }

    /// Request options honouring the contact's timeout.
    #[must_use]
    pub fn request_options(&self, key: &PublicKey) -> meshcore::RequestOptions {
        meshcore::RequestOptions {
            timeout: self.timeout_for(key),
            ..meshcore::RequestOptions::default()
        }
    }

    /// Returns the channel slots, fetching them on first use.
    pub async fn channels(&self, anim: bool) -> Vec<ChannelEntry> {
        if let Some(channels) = self.state().channels.clone() {
            return channels;
        }
        if anim {
            crate::output::print("Fetching channels ");
        }
        let mut channels = Vec::new();
        for idx in 0..=u8::MAX {
            match self.mc.get_channel(idx).await {
                Ok(info) => {
                    channels.push(ChannelEntry::from_value(to_value(&info)));
                    if anim {
                        crate::output::print(".");
                    }
                }
                Err(_) => break,
            }
        }
        if anim {
            crate::output::print(" Done\n");
        }
        self.state().channels = Some(channels.clone());
        channels
    }

    /// Finds a channel by name (`get_channel_by_name`).
    pub async fn channel_by_name(&self, name: &str) -> Option<ChannelEntry> {
        self.channels(false)
            .await
            .into_iter()
            .find(|c| c.name == name)
    }

    /// Name of a channel slot when channels have been fetched.
    #[must_use]
    pub fn cached_channel_name(&self, index: u8) -> Option<String> {
        self.state()
            .channels
            .as_ref()
            .and_then(|chans| chans.iter().find(|c| c.index == index))
            .map(|c| c.name.clone())
    }

    /// Updates the cached slot after `set_channel`.
    pub fn update_channel(&self, entry: ChannelEntry) {
        let mut state = self.state();
        if let Some(channels) = state.channels.as_mut() {
            if let Some(slot) = channels.iter_mut().find(|c| c.index == entry.index) {
                *slot = entry;
            } else {
                channels.push(entry);
            }
        }
    }

    /// Sets the flood scope (`set_scope`).
    ///
    /// `"None"`, `"0"`, `"clear"` and `""` select the default scope. Returns
    /// the active scope, or `None` when scopes are unsupported or failed.
    pub async fn set_scope(&self, scope: &str) -> Option<String> {
        if self.state().no_scope {
            return None;
        }
        let scope = if matches!(scope, "None" | "0" | "clear" | "") {
            ""
        } else {
            scope
        };
        if self.state().current_scope.as_deref() == Some(scope) {
            return Some(scope.to_string());
        }
        match self.mc.set_flood_scope_str(scope).await {
            Ok(()) => {
                self.state().current_scope = Some(scope.to_string());
                Some(scope.to_string())
            }
            Err(meshcore::Error::Device(code)) => {
                if code == meshcore::ErrorCode::UnsupportedCmd {
                    self.state().no_scope = true;
                }
                None
            }
            Err(_) => None,
        }
    }

    /// Appends a message to the message log (`log_message`).
    pub fn log_message(&self, mut msg: Value) {
        let Some(file) = self.state().message_log.clone() else {
            return;
        };
        let kind = vstr(&msg, "type");
        if kind.is_empty() || kind == "DATA" {
            return;
        }
        if let Value::Object(map) = &mut msg {
            if kind == "PRIV" {
                let prefix = vstr(&Value::Object(map.clone()), "pubkey_prefix");
                let name = self
                    .mc
                    .get_contact_by_key_prefix(&prefix)
                    .map_or(prefix, |c| c.name.clone());
                map.insert("name".into(), Value::String(name.clone()));
                map.insert("sender".into(), Value::String(name));
            } else if kind == "CHAN" {
                let idx =
                    u8::try_from(vint(&Value::Object(map.clone()), "channel_idx")).unwrap_or(0);
                let sender = self
                    .cached_channel_name(idx)
                    .unwrap_or_else(|| format!("channel {idx}"));
                map.insert("sender".into(), Value::String(sender.clone()));
                map.insert("name".into(), Value::String(sender));
            }
            map.insert(
                "timestamp".into(),
                Value::from(crate::util::time::now_secs()),
            );
        }
        let line = format!("{}\n", crate::util::json::dumps(&msg));
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    }
}

/// Hex encoding of bytes.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    crate::util::text::hex_lower(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn value_accessors() {
        let v = json!({"s": "x", "i": 3, "f": 1.5, "n": null});
        assert_eq!(vstr(&v, "s"), "x");
        assert_eq!(vstr(&v, "i"), "3");
        assert_eq!(vstr(&v, "n"), "");
        assert_eq!(vint(&v, "i"), 3);
        assert_eq!(vint(&v, "f"), 1);
        assert!((vf64(&v, "f") - 1.5).abs() < f64::EPSILON);
        assert_eq!(vint(&v, "missing"), 0);
    }

    #[test]
    fn aliases_keep_order() {
        let mut state = State::default();
        state.set_alias("b", "1");
        state.set_alias("a", "2");
        state.set_alias("b", "3");
        assert_eq!(state.alias("b").as_deref(), Some("3"));
        assert_eq!(
            crate::util::json::dumps(&state.aliases_value()),
            r#"{"b": "3", "a": "2"}"#
        );
        state.remove_alias("b");
        assert_eq!(state.alias("b"), None);
    }
}
