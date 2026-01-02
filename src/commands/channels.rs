//! Channel-related commands.

use meshcore::event::Event;
use sha2::{Digest, Sha256};

use super::CommandContext;
use crate::error::{CliError, Result};

/// Checks if a channel name indicates an empty/unused channel.
fn is_channel_empty(name: &str) -> bool {
    name.is_empty() || name.chars().all(|c| c == '\0')
}

/// Parses or generates a channel secret from an optional key string and channel name.
fn parse_channel_secret(name: &str, key: Option<&str>) -> Result<[u8; 16]> {
    if let Some(key_str) = key {
        // Try to parse as hex
        let bytes = hex::decode(key_str)
            .map_err(|_| CliError::InvalidArgument("Invalid hex key".into()))?;
        if bytes.len() != 16 {
            return Err(CliError::InvalidArgument(
                "Channel key must be 16 bytes (32 hex chars)".into(),
            ));
        }
        let mut arr = [0u8; 16];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    } else if name.starts_with('#') {
        // Auto-generate key from name hash (like Python CLI)
        let mut hasher = Sha256::new();
        hasher.update(name.as_bytes());
        let hash = hasher.finalize();
        let mut arr = [0u8; 16];
        arr.copy_from_slice(&hash[..16]);
        Ok(arr)
    } else {
        // Empty/zero key
        Ok([0u8; 16])
    }
}

impl CommandContext {
    /// Executes the `get_channels` command.
    pub async fn cmd_get_channels(&self) -> Result<()> {
        // Get channels 0-7
        for i in 0..8 {
            let event = self.commands().await.get_channel(i).await?;

            match event {
                Event::ChannelInfo(channel) => {
                    if !is_channel_empty(&channel.name) {
                        self.display.print_channel(&channel);
                    }
                }
                Event::Error { message } => {
                    // Channel might not exist, just skip
                    if !message.contains("not found") {
                        self.display
                            .print_warning(&format!("Channel {i}: {message}"));
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Executes the `get_channel` command.
    pub async fn cmd_get_channel(&self, channel: &str) -> Result<()> {
        let index = Self::get_channel_index(channel)?;
        let event = self.commands().await.get_channel(index).await?;

        match event {
            Event::ChannelInfo(channel) => {
                self.display.print_channel(&channel);
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {
                return Err(CliError::Command("Unexpected response".into()));
            }
        }

        Ok(())
    }

    /// Executes the `set_channel` command.
    pub async fn cmd_set_channel(&self, number: u8, name: &str, key: Option<&str>) -> Result<()> {
        let secret = parse_channel_secret(name, key)?;
        self.commands()
            .await
            .set_channel(number, name, &secret)
            .await?;
        self.display
            .print_ok(&format!("channel {number} set to '{name}'"));
        Ok(())
    }

    /// Executes the `remove_channel` command.
    pub async fn cmd_remove_channel(&self, channel: &str) -> Result<()> {
        let index = Self::get_channel_index(channel)?;

        // Remove channel by setting empty name and zero secret
        self.commands()
            .await
            .set_channel(index, "", &[0u8; 16])
            .await?;
        self.display.print_ok(&format!("channel {index} removed"));
        Ok(())
    }

    /// Executes the `add_channel` command.
    /// Finds the first available slot and adds the channel there.
    pub async fn cmd_add_channel(&self, name: &str, key: Option<&str>) -> Result<()> {
        // Find the first empty channel slot
        let mut free_slot: Option<u8> = None;
        for i in 0..8 {
            let event = self.commands().await.get_channel(i).await?;
            if let Event::ChannelInfo(channel) = event {
                if is_channel_empty(&channel.name) {
                    free_slot = Some(i);
                    break;
                }
            }
        }

        let slot =
            free_slot.ok_or_else(|| CliError::Command("No free channel slots available".into()))?;

        let secret = parse_channel_secret(name, key)?;
        self.commands()
            .await
            .set_channel(slot, name, &secret)
            .await?;
        self.display
            .print_ok(&format!("channel added at slot {slot}: '{name}'"));
        Ok(())
    }
}
