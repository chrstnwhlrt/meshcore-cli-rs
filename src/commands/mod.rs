//! Command implementations.

pub mod channels;
pub mod contacts;
pub mod device;
pub mod messaging;
pub mod repeater;

use std::sync::Arc;
use std::time::Duration;

use meshcore::MeshCore;
use meshcore::event::{Event, EventFilter, Subscription};
use meshcore::transport::serial::SerialTransport;
use tokio::sync::Mutex;

use crate::config::SessionState;
use crate::display::Display;
use crate::error::{CliError, Result};

/// Command context shared between command handlers.
pub struct CommandContext {
    /// The `MeshCore` client (wrapped for interior mutability).
    pub client: Arc<Mutex<MeshCore<SerialTransport>>>,
    /// Display configuration.
    pub display: Display,
    /// Session state.
    pub state: Arc<Mutex<SessionState>>,
    /// Device name (from initial connection).
    pub device_name: Option<String>,
}

impl CommandContext {
    /// Creates a new command context.
    pub fn new(
        client: MeshCore<SerialTransport>,
        display: Display,
        device_name: Option<String>,
    ) -> Self {
        Self {
            client: Arc::new(Mutex::new(client)),
            display,
            state: Arc::new(Mutex::new(SessionState::new())),
            device_name,
        }
    }

    /// Gets the command handler.
    pub async fn commands(
        &self,
    ) -> impl std::ops::Deref<Target = meshcore::commands::CommandHandler<SerialTransport>> + '_
    {
        struct CommandsGuard<'a> {
            guard: tokio::sync::MutexGuard<'a, MeshCore<SerialTransport>>,
        }
        impl std::ops::Deref for CommandsGuard<'_> {
            type Target = meshcore::commands::CommandHandler<SerialTransport>;
            fn deref(&self) -> &Self::Target {
                self.guard.commands()
            }
        }
        CommandsGuard {
            guard: self.client.lock().await,
        }
    }

    /// Gets a subscription to events.
    pub async fn subscribe(&self) -> Subscription {
        self.client.lock().await.subscribe()
    }

    /// Gets a contact by name or public key prefix.
    pub async fn get_contact(&self, name_or_key: &str) -> Result<meshcore::types::Contact> {
        let client = self.client.lock().await;
        let contacts = client.contacts().await;

        // First try exact name match
        if let Some(contact) = contacts
            .values()
            .find(|c| c.name.eq_ignore_ascii_case(name_or_key))
        {
            return Ok(contact.clone());
        }

        // Then try public key prefix match
        let prefix = name_or_key.to_lowercase();
        if let Some(contact) = contacts
            .values()
            .find(|c| c.public_key.to_hex().starts_with(&prefix))
        {
            return Ok(contact.clone());
        }

        Err(CliError::ContactNotFound(name_or_key.to_string()))
    }

    /// Gets a channel by number or name.
    pub fn get_channel_index(channel: &str) -> Result<u8> {
        parse_channel_index(channel)
    }

    /// Waits for an event with timeout.
    pub async fn wait_for_event(&self, filter: EventFilter, timeout: Duration) -> Result<Event> {
        let mut subscription = self.subscribe().await;

        let result = tokio::time::timeout(timeout, async {
            loop {
                if let Some(event) = subscription.recv().await {
                    if filter.matches(&event) {
                        return Some(event);
                    }
                } else {
                    return None;
                }
            }
        })
        .await;

        match result {
            Ok(Some(event)) => Ok(event),
            Ok(None) | Err(_) => Err(CliError::Timeout("event".into())),
        }
    }
}

/// Gets the current Unix timestamp.
#[must_use]
pub fn current_timestamp() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u32::try_from(d.as_secs()).unwrap_or(u32::MAX))
        .unwrap_or(0)
}

/// Parses a channel index from a string (number or name).
fn parse_channel_index(channel: &str) -> Result<u8> {
    // Try parsing as number first
    if let Ok(index) = channel.parse::<u8>() {
        return Ok(index);
    }

    // Otherwise, return an error (channel name lookup not implemented)
    Err(CliError::ChannelNotFound(channel.to_string()))
}

/// Parses a time value string into seconds.
///
/// Supports suffixes: `d` (days), `h` (hours), `m` (minutes), `s` (seconds).
/// Without suffix, the value is treated as seconds.
#[must_use]
pub fn parse_time_value(s: &str) -> u32 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }

    if let Some(num_str) = s.strip_suffix('d') {
        num_str.parse::<u32>().unwrap_or(0) * 86400
    } else if let Some(num_str) = s.strip_suffix('h') {
        num_str.parse::<u32>().unwrap_or(0) * 3600
    } else if let Some(num_str) = s.strip_suffix('m') {
        num_str.parse::<u32>().unwrap_or(0) * 60
    } else if let Some(num_str) = s.strip_suffix('s') {
        num_str.parse::<u32>().unwrap_or(0)
    } else {
        s.parse::<u32>().unwrap_or(0)
    }
}

/// Looks up a contact name from a public key prefix.
///
/// Returns the contact name if found, or the hex-encoded prefix otherwise.
pub fn lookup_sender_name(
    contacts: &std::collections::HashMap<meshcore::types::PublicKey, meshcore::types::Contact>,
    sender_prefix: &[u8],
) -> String {
    let prefix_hex = hex::encode(sender_prefix);
    contacts
        .values()
        .find(|c| c.public_key.to_hex().starts_with(&prefix_hex))
        .map_or(prefix_hex, |c| c.name.clone())
}
