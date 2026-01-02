//! Contact-related commands.

use meshcore::event::Event;
use meshcore::types::ContactType;

use super::CommandContext;
use crate::error::{CliError, Result};

impl CommandContext {
    /// Executes the `contacts` / `list` command.
    pub async fn cmd_contacts(&self) -> Result<()> {
        // First refresh contacts from device
        self.commands().await.get_contacts(None).await?;

        // Then get from cache
        let contacts = self.client.lock().await.contacts().await;
        let mut contact_list: Vec<_> = contacts.values().cloned().collect();

        // Sort by name
        contact_list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        self.display.print_contacts(&contact_list);
        Ok(())
    }

    /// Executes the `reload_contacts` command.
    pub async fn cmd_reload_contacts(&self) -> Result<()> {
        // Force reload by passing None for last_modified
        self.commands().await.get_contacts(None).await?;
        self.display.print_ok("contacts reloaded");
        Ok(())
    }

    /// Executes the `contact_info` command.
    pub async fn cmd_contact_info(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;
        self.display.print_contact(&contact);

        if !self.display.is_json() {
            // Print additional info in human mode
            let type_str = match contact.device_type {
                ContactType::Node => "Node",
                ContactType::Repeater => "Repeater",
                ContactType::Room => "Room",
                ContactType::Unknown => "Unknown",
            };

            println!("  Type: {type_str}");
            println!("  Flags: 0x{:02x}", contact.flags.as_byte());

            match contact.out_path_len.cmp(&0) {
                std::cmp::Ordering::Less => println!("  Path: flood"),
                std::cmp::Ordering::Equal => println!("  Path: direct"),
                std::cmp::Ordering::Greater => {
                    let path_len = usize::try_from(contact.out_path_len).unwrap_or(0);
                    let byte_len = (path_len * 6).min(contact.out_path.len());
                    let path_hex = hex::encode(&contact.out_path[..byte_len]);
                    println!("  Path: {} hops ({})", contact.out_path_len, path_hex);
                }
            }

            if contact.last_advert > 0 {
                use chrono::{TimeZone, Utc};
                if let Some(dt) = Utc
                    .timestamp_opt(i64::from(contact.last_advert), 0)
                    .single()
                {
                    println!("  Last advert: {}", dt.format("%Y-%m-%d %H:%M:%S"));
                }
            }

            if contact.last_modified > 0 {
                use chrono::{TimeZone, Utc};
                if let Some(dt) = Utc
                    .timestamp_opt(i64::from(contact.last_modified), 0)
                    .single()
                {
                    println!("  Last modified: {}", dt.format("%Y-%m-%d %H:%M:%S"));
                }
            }
        }

        Ok(())
    }

    /// Executes the `contact_timeout` command.
    pub async fn cmd_contact_timeout(&self, name: &str, timeout: u64) -> Result<()> {
        let contact = self.get_contact(name).await?;
        let mut state = self.state.lock().await;
        state.contact_timeouts.insert(contact.name.clone(), timeout);
        self.display
            .print_ok(&format!("timeout for {} set to {}s", contact.name, timeout));
        Ok(())
    }

    /// Executes the `path` command.
    pub async fn cmd_path(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;

        if self.display.is_json() {
            self.display.print_json(&serde_json::json!({
                "name": contact.name,
                "path_len": contact.out_path_len,
                "path": if contact.out_path_len > 0 {
                    let path_len = usize::try_from(contact.out_path_len).unwrap_or(0);
                    let byte_len = (path_len * 6).min(contact.out_path.len());
                    hex::encode(&contact.out_path[..byte_len])
                } else {
                    String::new()
                },
            }));
        } else {
            match contact.out_path_len.cmp(&0) {
                std::cmp::Ordering::Less => println!("{}: flood", contact.name),
                std::cmp::Ordering::Equal => println!("{}: direct", contact.name),
                std::cmp::Ordering::Greater => {
                    let path_len = usize::try_from(contact.out_path_len).unwrap_or(0);
                    // Path is stored as 6-byte prefixes
                    let mut path_parts = Vec::new();
                    for i in 0..path_len.min(10) {
                        let start = i * 6;
                        let end = start + 6;
                        if end <= contact.out_path.len() {
                            path_parts.push(hex::encode(&contact.out_path[start..end]));
                        }
                    }
                    println!(
                        "{}: {} hops [{}]",
                        contact.name,
                        contact.out_path_len,
                        path_parts.join(" -> ")
                    );
                }
            }
        }

        Ok(())
    }

    /// Executes the `disc_path` command.
    pub async fn cmd_disc_path(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;

        let event = self
            .commands()
            .await
            .path_discovery(&contact.public_key)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);
                self.display.print_ok("path discovery started");
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Executes the `reset_path` command.
    pub async fn cmd_reset_path(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;
        self.commands()
            .await
            .reset_path(&contact.public_key)
            .await?;
        self.display
            .print_ok(&format!("path to {} reset to flood", contact.name));
        Ok(())
    }

    /// Executes the `change_path` command.
    pub async fn cmd_change_path(&self, name: &str, path: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;

        // Parse the path (comma-separated hex prefixes)
        let mut path_bytes = Vec::new();
        for part in path.split(',') {
            let hex_str = part.trim();
            if hex_str.is_empty() {
                continue;
            }
            let bytes = hex::decode(hex_str).map_err(|_| {
                CliError::InvalidArgument(format!("Invalid hex in path: {hex_str}"))
            })?;
            if bytes.len() != 6 {
                return Err(CliError::InvalidArgument(
                    "Each path element must be 6 bytes (12 hex chars)".into(),
                ));
            }
            path_bytes.extend_from_slice(&bytes);
        }

        let path_len = i8::try_from((path_bytes.len() / 6).min(127)).unwrap_or(0);

        // Update the contact with new path
        let params = meshcore::ContactUpdateParams {
            public_key: &contact.public_key,
            contact_type: match contact.device_type {
                ContactType::Unknown => 0,
                ContactType::Node => 1,
                ContactType::Repeater => 2,
                ContactType::Room => 3,
            },
            flags: contact.flags.as_byte(),
            path_len,
            path: &path_bytes,
            name: &contact.name,
            last_advert: contact.last_advert,
            latitude: contact.latitude,
            longitude: contact.longitude,
        };
        self.commands().await.update_contact(&params).await?;

        self.display
            .print_ok(&format!("path to {} changed", contact.name));
        Ok(())
    }

    /// Executes the `change_flags` command.
    pub async fn cmd_change_flags(&self, name: &str, flags_str: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;
        let mut flags = contact.flags.as_byte();

        // Parse flags
        for flag in flags_str.split(',') {
            let flag = flag.trim();
            let (add, name) = if let Some(stripped) = flag.strip_prefix('-') {
                (false, stripped)
            } else if let Some(stripped) = flag.strip_prefix('+') {
                (true, stripped)
            } else {
                (true, flag)
            };

            let bit = match name.to_lowercase().as_str() {
                "trusted" => 0x01,
                "hidden" => 0x02,
                "tel_l" | "tel_loc" => 0x04,
                "tel_a" | "tel_all" => 0x08,
                "star" | "starred" => 0x10,
                _ => {
                    return Err(CliError::InvalidArgument(format!("Unknown flag: {name}")));
                }
            };

            if add {
                flags |= bit;
            } else {
                flags &= !bit;
            }
        }

        // Update the contact with new flags
        let path_len = contact.out_path_len;
        let path_bytes: &[u8] = if path_len > 0 {
            let byte_len = usize::try_from(path_len).unwrap_or(0) * 6;
            &contact.out_path[..byte_len.min(contact.out_path.len())]
        } else {
            &[]
        };

        let params = meshcore::ContactUpdateParams {
            public_key: &contact.public_key,
            contact_type: match contact.device_type {
                ContactType::Unknown => 0,
                ContactType::Node => 1,
                ContactType::Repeater => 2,
                ContactType::Room => 3,
            },
            flags,
            path_len,
            path: path_bytes,
            name: &contact.name,
            last_advert: contact.last_advert,
            latitude: contact.latitude,
            longitude: contact.longitude,
        };
        self.commands().await.update_contact(&params).await?;

        self.display
            .print_ok(&format!("flags for {} updated", contact.name));
        Ok(())
    }

    /// Executes the `share_contact` command.
    pub async fn cmd_share_contact(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;
        self.commands()
            .await
            .share_contact(&contact.public_key)
            .await?;
        self.display.print_ok(&format!("{} shared", contact.name));
        Ok(())
    }

    /// Executes the `export_contact` command.
    pub async fn cmd_export_contact(&self, name: Option<&str>) -> Result<()> {
        let key = if let Some(n) = name {
            let contact = self.get_contact(n).await?;
            Some(contact.public_key)
        } else {
            None
        };

        let event = self.commands().await.export_contact(key.as_ref()).await?;

        match event {
            Event::ContactUri(uri) => {
                if self.display.is_json() {
                    self.display.print_json(&serde_json::json!({ "uri": uri }));
                } else {
                    println!("{uri}");
                }
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

    /// Executes the `import_contact` command.
    pub async fn cmd_import_contact(&self, uri: &str) -> Result<()> {
        use base64::{Engine, engine::general_purpose::STANDARD};

        // Extract card data from URI
        // Format: mc://...#<base64_data> or just base64 data
        let data_str = if uri.contains('#') {
            uri.split('#').next_back().unwrap_or(uri)
        } else if uri.starts_with("mc://") {
            return Err(CliError::InvalidArgument("Invalid URI format".into()));
        } else {
            uri
        };

        // Try base64 decode
        let card_data = STANDARD
            .decode(data_str)
            .map_err(|_| CliError::InvalidArgument("Invalid base64 data in URI".into()))?;

        self.commands().await.import_contact(&card_data).await?;
        self.display.print_ok("contact imported");
        Ok(())
    }

    /// Executes the `remove_contact` command.
    pub async fn cmd_remove_contact(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;
        self.commands()
            .await
            .remove_contact(&contact.public_key)
            .await?;
        self.display.print_ok(&format!("{} removed", contact.name));
        Ok(())
    }

    /// Executes the `pending_contacts` command.
    pub async fn cmd_pending_contacts(&self) -> Result<()> {
        let state = self.state.lock().await;

        if self.display.is_json() {
            let pending: Vec<_> = state
                .pending_contacts
                .values()
                .map(|p| {
                    serde_json::json!({
                        "public_key": p.public_key,
                        "name": p.name,
                    })
                })
                .collect();
            self.display.print_json(&pending);
        } else if state.pending_contacts.is_empty() {
            println!("No pending contacts");
        } else {
            for pending in state.pending_contacts.values() {
                if let Some(name) = &pending.name {
                    println!("{} ({})", name, pending.public_key);
                } else {
                    println!("{}", pending.public_key);
                }
            }
        }

        Ok(())
    }

    /// Executes the `add_pending` command.
    pub async fn cmd_add_pending(&self, pending_id: &str) -> Result<()> {
        let state = self.state.lock().await;

        // Find the pending contact by key or name
        let pending = state
            .pending_contacts
            .values()
            .find(|p| {
                p.public_key.starts_with(pending_id)
                    || p.name
                        .as_ref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(pending_id))
            })
            .cloned();

        drop(state);

        let pending = pending.ok_or_else(|| {
            CliError::ContactNotFound(format!("Pending contact not found: {pending_id}"))
        })?;

        // We need the full contact data to add it
        let contact = pending.contact.ok_or_else(|| {
            CliError::InvalidArgument(
                "Pending contact has no full data. Only contacts from NewContactAdvert can be added.".into()
            )
        })?;

        // Add the contact to the device using update_contact
        let ctype = match contact.device_type {
            ContactType::Unknown => 0,
            ContactType::Node => 1,
            ContactType::Repeater => 2,
            ContactType::Room => 3,
        };

        let path_len = contact.out_path_len;
        let path_bytes: &[u8] = if path_len > 0 {
            let byte_len = usize::try_from(path_len).unwrap_or(0) * 6;
            &contact.out_path[..byte_len.min(contact.out_path.len())]
        } else {
            &[]
        };

        let params = meshcore::ContactUpdateParams {
            public_key: &contact.public_key,
            contact_type: ctype,
            flags: contact.flags.as_byte(),
            path_len,
            path: path_bytes,
            name: &contact.name,
            last_advert: contact.last_advert,
            latitude: contact.latitude,
            longitude: contact.longitude,
        };
        self.commands().await.update_contact(&params).await?;

        self.display
            .print_ok(&format!("Added contact: {}", contact.name));

        // Remove from pending list
        let mut state = self.state.lock().await;
        state.pending_contacts.remove(&pending.public_key);

        Ok(())
    }

    /// Executes the `flush_pending` command.
    pub async fn cmd_flush_pending(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        let count = state.pending_contacts.len();
        state.clear_pending();
        self.display
            .print_ok(&format!("flushed {count} pending contacts"));
        Ok(())
    }
}
