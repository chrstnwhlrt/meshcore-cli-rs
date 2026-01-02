//! Configuration management for meshcore-cli-rs.
//!
//! Compatible with the Python meshcore-cli configuration in `~/.config/meshcore`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Configuration directory name (compatible with Python CLI).
const CONFIG_DIR: &str = "meshcore";

/// History file name.
const HISTORY_FILE: &str = "history";

/// Init script file name.
const INIT_FILE: &str = "init";

/// CLI configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Default serial port.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_port: Option<String>,

    /// Default baud rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_baudrate: Option<u32>,

    /// Contact-specific timeouts.
    #[serde(default)]
    pub contact_timeouts: HashMap<String, u64>,

    /// Color output enabled.
    #[serde(default = "default_true")]
    pub color: bool,

    /// Channel echoes enabled.
    #[serde(default)]
    pub channel_echoes: bool,

    /// Auto-update contacts.
    #[serde(default = "default_true")]
    pub auto_update_contacts: bool,
}

fn default_true() -> bool {
    true
}

impl Config {
    /// Gets the configuration directory path.
    #[must_use]
    pub fn config_dir() -> Option<PathBuf> {
        // Use XDG config (Linux/macOS) or platform-specific config dir
        ProjectDirs::from("", "", CONFIG_DIR).map(|p| p.config_dir().to_path_buf())
    }

    /// Gets the history file path.
    #[must_use]
    pub fn history_file() -> Option<PathBuf> {
        Self::config_dir().map(|p| p.join(HISTORY_FILE))
    }

    /// Gets the init script path.
    #[must_use]
    pub fn init_file() -> Option<PathBuf> {
        Self::config_dir().map(|p| p.join(INIT_FILE))
    }

    /// Gets the device-specific init script path.
    #[must_use]
    pub fn device_init_file(device_name: &str) -> Option<PathBuf> {
        Self::config_dir().map(|p| p.join(format!("{device_name}.init")))
    }

    /// Reads script lines from a file path.
    fn read_script_from_path(path: Option<PathBuf>) -> Result<Vec<String>> {
        let path = match path {
            Some(p) if p.exists() => p,
            _ => return Ok(Vec::new()),
        };

        let content = fs::read_to_string(&path)?;
        Ok(content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect())
    }

    /// Reads init script lines.
    pub fn read_init_script() -> Result<Vec<String>> {
        Self::read_script_from_path(Self::init_file())
    }

    /// Reads device-specific init script lines.
    pub fn read_device_init_script(device_name: &str) -> Result<Vec<String>> {
        Self::read_script_from_path(Self::device_init_file(device_name))
    }
}

/// Runtime state that persists during a session.
#[derive(Debug, Default)]
pub struct SessionState {
    /// Current device name.
    pub device_name: Option<String>,

    /// Current target contact (for `to` command).
    pub current_contact: Option<String>,

    /// Previous contact (for `to ..`).
    pub previous_contact: Option<String>,

    /// Last message sender (for `to !`).
    pub last_sender: Option<String>,

    /// Logged-in repeaters.
    pub logged_in: HashMap<String, bool>,

    /// Pending contacts (for manual contact adding).
    pub pending_contacts: HashMap<String, PendingContact>,

    /// Current flood scope.
    pub flood_scope: Option<String>,

    /// Contact-specific timeouts (overrides config).
    pub contact_timeouts: HashMap<String, u64>,
}

/// A pending contact waiting for manual approval.
#[derive(Debug, Clone)]
pub struct PendingContact {
    /// Public key hex.
    pub public_key: String,

    /// Contact name (if known).
    pub name: Option<String>,

    /// Full contact data (if available from `NewContactAdvert` event).
    pub contact: Option<meshcore::types::Contact>,
}

impl SessionState {
    /// Creates a new session state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the current contact (updates previous).
    pub fn set_contact(&mut self, contact: Option<String>) {
        if self.current_contact != contact {
            self.previous_contact = self.current_contact.take();
            self.current_contact = contact;
        }
    }

    /// Swaps current and previous contact (for `to ..`).
    pub fn swap_contacts(&mut self) {
        std::mem::swap(&mut self.current_contact, &mut self.previous_contact);
    }

    /// Checks if logged into a repeater.
    #[must_use]
    pub fn is_logged_in(&self, name: &str) -> bool {
        self.logged_in.get(name).copied().unwrap_or(false)
    }

    /// Sets login state for a repeater.
    pub fn set_logged_in(&mut self, name: &str, logged_in: bool) {
        self.logged_in.insert(name.to_string(), logged_in);
    }

    /// Adds a pending contact.
    pub fn add_pending(&mut self, public_key: String, name: Option<String>) {
        self.pending_contacts.insert(
            public_key.clone(),
            PendingContact {
                public_key,
                name,
                contact: None,
            },
        );
    }

    /// Adds a pending contact with full contact data.
    pub fn add_pending_contact(&mut self, contact: meshcore::types::Contact) {
        let public_key = contact.public_key.to_hex();
        self.pending_contacts.insert(
            public_key.clone(),
            PendingContact {
                public_key,
                name: Some(contact.name.clone()),
                contact: Some(contact),
            },
        );
    }

    /// Clears all pending contacts.
    pub fn clear_pending(&mut self) {
        self.pending_contacts.clear();
    }

    /// Gets timeout for a contact.
    #[must_use]
    pub fn get_timeout(&self, contact: &str, default: u64) -> u64 {
        self.contact_timeouts
            .get(contact)
            .copied()
            .unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_dir() {
        let dir = Config::config_dir();
        assert!(dir.is_some());
        let path = dir.unwrap();
        assert!(path.to_string_lossy().contains("meshcore"));
    }

    #[test]
    fn test_history_file() {
        let path = Config::history_file();
        assert!(path.is_some());
        assert!(path.unwrap().to_string_lossy().ends_with("history"));
    }

    #[test]
    fn test_init_file() {
        let path = Config::init_file();
        assert!(path.is_some());
        assert!(path.unwrap().to_string_lossy().ends_with("init"));
    }

    #[test]
    fn test_device_init_file() {
        let path = Config::device_init_file("mydevice");
        assert!(path.is_some());
        assert!(path.unwrap().to_string_lossy().ends_with("mydevice.init"));
    }

    #[test]
    fn test_session_state_new() {
        let state = SessionState::new();
        assert!(state.current_contact.is_none());
        assert!(state.previous_contact.is_none());
        assert!(state.last_sender.is_none());
    }

    #[test]
    fn test_session_state_set_contact() {
        let mut state = SessionState::new();
        state.set_contact(Some("Alice".to_string()));
        assert_eq!(state.current_contact, Some("Alice".to_string()));
        assert!(state.previous_contact.is_none());

        state.set_contact(Some("Bob".to_string()));
        assert_eq!(state.current_contact, Some("Bob".to_string()));
        assert_eq!(state.previous_contact, Some("Alice".to_string()));
    }

    #[test]
    fn test_session_state_swap_contacts() {
        let mut state = SessionState::new();
        state.set_contact(Some("Alice".to_string()));
        state.set_contact(Some("Bob".to_string()));

        state.swap_contacts();
        assert_eq!(state.current_contact, Some("Alice".to_string()));
        assert_eq!(state.previous_contact, Some("Bob".to_string()));
    }

    #[test]
    fn test_session_state_logged_in() {
        let mut state = SessionState::new();
        assert!(!state.is_logged_in("repeater1"));

        state.set_logged_in("repeater1", true);
        assert!(state.is_logged_in("repeater1"));

        state.set_logged_in("repeater1", false);
        assert!(!state.is_logged_in("repeater1"));
    }

    #[test]
    fn test_session_state_timeout() {
        let mut state = SessionState::new();
        assert_eq!(state.get_timeout("contact1", 30), 30);

        state.contact_timeouts.insert("contact1".to_string(), 60);
        assert_eq!(state.get_timeout("contact1", 30), 60);
    }

    #[test]
    fn test_session_state_pending() {
        let mut state = SessionState::new();
        assert!(state.pending_contacts.is_empty());

        state.add_pending("abc123".to_string(), Some("Alice".to_string()));
        assert_eq!(state.pending_contacts.len(), 1);
        assert!(state.pending_contacts.contains_key("abc123"));

        state.clear_pending();
        assert!(state.pending_contacts.is_empty());
    }
}
