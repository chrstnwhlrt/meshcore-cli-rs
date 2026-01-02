//! Repeater-related commands (login, cmd, status requests, trace, etc.).

use std::time::Duration;

use meshcore::event::{Event, EventFilter};
use meshcore::protocol::PacketType;

use super::{CommandContext, current_timestamp};
use crate::error::{CliError, Result};

impl CommandContext {
    /// Executes the `login` command.
    pub async fn cmd_login(&self, name: &str, password: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;

        let event = self
            .commands()
            .await
            .send_login(&contact.public_key, password)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);

                // Wait for login response with contact-specific timeout
                let filter = EventFilter::packet_types(vec![
                    PacketType::LoginSuccess,
                    PacketType::LoginFailed,
                ]);
                let timeout_secs = self.state.lock().await.get_timeout(&contact.name, 30);
                let timeout = Duration::from_secs(timeout_secs);

                match self.wait_for_event(filter, timeout).await {
                    Ok(Event::LoginSuccess) => {
                        let mut state = self.state.lock().await;
                        state.set_logged_in(&contact.name, true);
                        self.display.print_ok("Login success");
                    }
                    Ok(Event::LoginFailed) => {
                        self.display.print_error("Login failed");
                    }
                    Ok(_) => {}
                    Err(_) => {
                        self.display.print_warning("Login response timeout");
                    }
                }
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Executes the `logout` command.
    pub async fn cmd_logout(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;

        // Check if we're logged in
        {
            let state = self.state.lock().await;
            if !state.is_logged_in(&contact.name) {
                self.display
                    .print_warning(&format!("Not logged into {}", contact.name));
            }
        }

        self.commands()
            .await
            .send_logout(&contact.public_key)
            .await?;

        let mut state = self.state.lock().await;
        state.set_logged_in(&contact.name, false);

        self.display
            .print_ok(&format!("Logged out of {}", contact.name));
        Ok(())
    }

    /// Executes the `cmd` command (send command to repeater).
    pub async fn cmd_cmd(
        &self,
        name: &str,
        command: &[String],
        wait: bool,
        timeout_secs: u64,
    ) -> Result<()> {
        let contact = self.get_contact(name).await?;
        let cmd_text = command.join(" ");
        let timestamp = current_timestamp();

        let event = self
            .commands()
            .await
            .send_command(&contact.public_key, &cmd_text, timestamp)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        // Wait for ACK if requested
        if wait {
            self.cmd_wait_ack(timeout_secs).await?;
        }

        Ok(())
    }

    /// Executes the `wmt8` command (wait for message with 8 second timeout).
    pub async fn cmd_wmt8(&self) -> Result<()> {
        self.cmd_wait_msg(8).await
    }

    /// Executes the `req_status` command.
    pub async fn cmd_req_status(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;

        let event = self
            .commands()
            .await
            .send_status_request(&contact.public_key)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);

                // Wait for status response
                let filter = EventFilter::packet_types(vec![PacketType::StatusResponse]);
                let timeout = Duration::from_secs(30);

                match self.wait_for_event(filter, timeout).await {
                    Ok(Event::StatusResponse(status)) => {
                        if self.display.is_json() {
                            self.display.print_json(&serde_json::json!({
                                "pubkey_prefix": hex::encode(status.pubkey_prefix),
                                "battery_mv": status.battery_mv,
                                "tx_queue_len": status.tx_queue_len,
                                "noise_floor": status.noise_floor,
                                "last_rssi": status.last_rssi,
                                "packets_received": status.packets_received,
                                "packets_sent": status.packets_sent,
                                "airtime_secs": status.airtime_secs,
                                "uptime_secs": status.uptime_secs,
                                "sent_flood": status.sent_flood,
                                "sent_direct": status.sent_direct,
                                "recv_flood": status.recv_flood,
                                "recv_direct": status.recv_direct,
                                "full_events": status.full_events,
                                "last_snr": status.last_snr,
                                "direct_dups": status.direct_dups,
                                "flood_dups": status.flood_dups,
                                "rx_airtime_secs": status.rx_airtime_secs,
                            }));
                        } else {
                            let voltage = f64::from(status.battery_mv) / 1000.0;
                            let uptime_hours = status.uptime_secs / 3600;
                            let uptime_mins = (status.uptime_secs % 3600) / 60;

                            println!("Status for {}:", contact.name);
                            println!("  Battery: {voltage:.2}V");
                            println!("  Uptime: {uptime_hours}h {uptime_mins}m");
                            println!("  TX Queue: {}", status.tx_queue_len);
                            println!("  Noise Floor: {} dBm", status.noise_floor);
                            println!("  Last RSSI: {} dBm", status.last_rssi);
                            println!("  Last SNR: {:.2} dB", status.last_snr);
                            println!(
                                "  Packets: {} sent, {} received",
                                status.packets_sent, status.packets_received
                            );
                            println!(
                                "  Flood: {} sent, {} received",
                                status.sent_flood, status.recv_flood
                            );
                            println!(
                                "  Direct: {} sent, {} received",
                                status.sent_direct, status.recv_direct
                            );
                            println!(
                                "  Airtime: {}s TX, {}s RX",
                                status.airtime_secs, status.rx_airtime_secs
                            );
                        }
                    }
                    Ok(_) => {}
                    Err(_) => {
                        self.display.print_warning("Status response timeout");
                    }
                }
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Executes the `req_neighbours` command.
    pub async fn cmd_req_neighbours(&self, name: &str) -> Result<()> {
        const PUBKEY_PREFIX_LEN: usize = 6;
        const PUBKEY_PREFIX_LEN_U8: u8 = 6;

        let contact = self.get_contact(name).await?;

        let event = self
            .commands()
            .await
            .binary_neighbours_request(&contact.public_key, 50, 0, 0, PUBKEY_PREFIX_LEN_U8)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);

                // Wait for binary response
                let filter = EventFilter::packet_types(vec![PacketType::BinaryResponse]);
                let timeout = Duration::from_secs(30);

                match self.wait_for_event(filter, timeout).await {
                    Ok(Event::BinaryResponse(data)) => {
                        // Parse neighbours response
                        // Format: [neighbours_count: u16 LE][results_count: u16 LE][entries...]
                        // Each entry: [pubkey_prefix: 6 bytes][secs_ago: i32 LE][snr: i8]
                        if data.len() < 4 {
                            self.display.print_error("Invalid neighbours response");
                            return Ok(());
                        }

                        let neighbours_count = i16::from_le_bytes([data[0], data[1]]);
                        let results_count = i16::from_le_bytes([data[2], data[3]]);

                        let entry_size = PUBKEY_PREFIX_LEN + 4 + 1; // 6 + 4 + 1 = 11 bytes per entry
                        let mut neighbours = Vec::new();
                        let mut offset = 4;

                        // Safely convert to usize, treating negative as 0
                        let count = usize::try_from(results_count).unwrap_or(0);
                        for _ in 0..count {
                            if offset + entry_size > data.len() {
                                break;
                            }

                            let pubkey_prefix =
                                hex::encode(&data[offset..offset + PUBKEY_PREFIX_LEN]);
                            offset += PUBKEY_PREFIX_LEN;

                            let secs_ago = i32::from_le_bytes([
                                data[offset],
                                data[offset + 1],
                                data[offset + 2],
                                data[offset + 3],
                            ]);
                            offset += 4;

                            let snr_raw = i8::from_ne_bytes([data[offset]]);
                            let snr = f32::from(snr_raw) / 4.0;
                            offset += 1;

                            neighbours.push((pubkey_prefix, secs_ago, snr));
                        }

                        if self.display.is_json() {
                            let neighbour_list: Vec<_> = neighbours
                                .iter()
                                .map(|(pk, secs, snr)| {
                                    serde_json::json!({
                                        "pubkey": pk,
                                        "secs_ago": secs,
                                        "snr": snr,
                                    })
                                })
                                .collect();
                            self.display.print_json(&serde_json::json!({
                                "neighbours_count": neighbours_count,
                                "results_count": results_count,
                                "neighbours": neighbour_list,
                            }));
                        } else {
                            println!(
                                "Got {} neighbours out of {} from {}:",
                                results_count, neighbours_count, contact.name
                            );

                            // Get known contacts for name lookup
                            let known_contacts = self.client.lock().await.contacts().await;

                            for (pubkey, secs_ago, snr) in &neighbours {
                                // Try to find contact by public key prefix
                                let name = known_contacts
                                    .values()
                                    .find(|c| c.public_key.to_hex().starts_with(pubkey))
                                    .map_or_else(|| format!("[{pubkey}]"), |c| c.name.clone());

                                // Format time ago
                                let time_str = Self::format_time_ago(*secs_ago);

                                println!("  {name:<20} {time_str}, {snr:.1} dB SNR");
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(_) => {
                        self.display.print_warning("Neighbours response timeout");
                    }
                }
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Formats seconds into human-readable time ago string.
    fn format_time_ago(secs: i32) -> String {
        let Ok(secs) = u32::try_from(secs) else {
            return "unknown".to_string();
        };
        if secs >= 86400 {
            let days = secs / 86400;
            format!("{days}d ago")
        } else if secs >= 3600 {
            let hours = secs / 3600;
            format!("{hours}h ago")
        } else if secs >= 60 {
            let mins = secs / 60;
            format!("{mins}m ago")
        } else {
            format!("{secs}s ago")
        }
    }

    /// Executes the `req_telemetry` command.
    pub async fn cmd_req_telemetry(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;

        let event = self
            .commands()
            .await
            .binary_telemetry_request(&contact.public_key)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);

                // Wait for binary response (telemetry comes as BinaryResponse, not TelemetryResponse)
                let filter = EventFilter::packet_types(vec![
                    PacketType::BinaryResponse,
                    PacketType::TelemetryResponse,
                ]);
                let timeout = Duration::from_secs(30);

                match self.wait_for_event(filter, timeout).await {
                    Ok(Event::BinaryResponse(data)) => {
                        // BinaryResponse format: skip(1) + tag(4) + lpp_data = 5 bytes header
                        if data.len() > 5 {
                            let lpp_data = &data[5..];
                            let telemetry = meshcore::types::Telemetry::parse_lpp(lpp_data);
                            self.print_telemetry(&contact.name, &telemetry);
                        } else {
                            self.display.print_warning("Invalid telemetry response");
                        }
                    }
                    Ok(Event::TelemetryResponse(telemetry)) => {
                        self.print_telemetry(&contact.name, &telemetry);
                    }
                    Ok(_) => {}
                    Err(_) => {
                        self.display.print_warning("Telemetry response timeout");
                    }
                }
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Executes the `req_mma` command.
    pub async fn cmd_req_mma(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;

        let event = self
            .commands()
            .await
            .binary_mma_request(&contact.public_key)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);
                self.display.print_ok("MMA request sent");
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Executes the `req_acl` command.
    pub async fn cmd_req_acl(&self, name: &str) -> Result<()> {
        let contact = self.get_contact(name).await?;

        let event = self
            .commands()
            .await
            .binary_acl_request(&contact.public_key)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);
                self.display.print_ok("ACL request sent");
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Executes the `trace` command.
    pub async fn cmd_trace(&self, path: &str) -> Result<()> {
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
            path_bytes.extend_from_slice(&bytes);
        }

        // Use auth code 0 for now (would need to be configured)
        let event = self
            .commands()
            .await
            .send_trace(0, None, 0, &path_bytes)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);
                self.display.print_ok("Trace started");
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Executes the `req_binary` command.
    /// Sends raw binary data to a contact and waits for a response.
    pub async fn cmd_req_binary(&self, name: &str, hex_data: &str) -> Result<()> {
        use meshcore::protocol::BinaryReqType;

        let contact = self.get_contact(name).await?;

        // Parse hex data
        let data = hex::decode(hex_data)
            .map_err(|_| CliError::InvalidArgument("Invalid hex data".into()))?;

        if data.is_empty() {
            return Err(CliError::InvalidArgument("Data cannot be empty".into()));
        }

        // First byte is the request type, rest is payload
        let req_type_byte = data[0];
        let payload = &data[1..];

        // Try to convert to known binary request type
        let req_type = match req_type_byte {
            0x01 => BinaryReqType::Status,
            0x02 => BinaryReqType::KeepAlive,
            0x03 => BinaryReqType::Telemetry,
            0x04 => BinaryReqType::Mma,
            0x05 => BinaryReqType::Acl,
            0x06 => BinaryReqType::Neighbours,
            _ => {
                return Err(CliError::InvalidArgument(format!(
                    "Unknown binary request type: 0x{req_type_byte:02x}. Valid types: 01=Status, 02=KeepAlive, 03=Telemetry, 04=MMA, 05=ACL, 06=Neighbours"
                )));
            }
        };

        let event = self
            .commands()
            .await
            .binary_request(&contact.public_key, req_type, payload)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);

                // Wait for binary response
                let filter = EventFilter::packet_types(vec![PacketType::BinaryResponse]);
                let timeout = Duration::from_secs(30);

                match self.wait_for_event(filter, timeout).await {
                    Ok(Event::BinaryResponse(data)) => {
                        if self.display.is_json() {
                            self.display.print_json(&serde_json::json!({
                                "data": hex::encode(&data),
                                "length": data.len(),
                            }));
                        } else {
                            println!("Binary response ({} bytes):", data.len());
                            println!("{}", hex::encode(&data));
                        }
                    }
                    Ok(_) => {}
                    Err(_) => {
                        self.display.print_warning("Binary response timeout");
                    }
                }
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Prints telemetry data in appropriate format (JSON or human-readable).
    fn print_telemetry(&self, name: &str, telemetry: &meshcore::types::Telemetry) {
        if self.display.is_json() {
            let readings: Vec<_> = telemetry
                .readings
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "channel": r.channel,
                        "type": r.lpp_type,
                        "value": format!("{:?}", r.value),
                    })
                })
                .collect();
            self.display
                .print_json(&serde_json::json!({"readings": readings}));
        } else {
            println!("Telemetry from {name}:");
            for reading in &telemetry.readings {
                println!("  Channel {}: {:?}", reading.channel, reading.value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CommandContext;

    #[test]
    fn test_format_time_ago_seconds() {
        assert_eq!(CommandContext::format_time_ago(0), "0s ago");
        assert_eq!(CommandContext::format_time_ago(30), "30s ago");
        assert_eq!(CommandContext::format_time_ago(59), "59s ago");
    }

    #[test]
    fn test_format_time_ago_minutes() {
        assert_eq!(CommandContext::format_time_ago(60), "1m ago");
        assert_eq!(CommandContext::format_time_ago(120), "2m ago");
        assert_eq!(CommandContext::format_time_ago(3599), "59m ago");
    }

    #[test]
    fn test_format_time_ago_hours() {
        assert_eq!(CommandContext::format_time_ago(3600), "1h ago");
        assert_eq!(CommandContext::format_time_ago(7200), "2h ago");
        assert_eq!(CommandContext::format_time_ago(86399), "23h ago");
    }

    #[test]
    fn test_format_time_ago_days() {
        assert_eq!(CommandContext::format_time_ago(86_400), "1d ago");
        assert_eq!(CommandContext::format_time_ago(172_800), "2d ago");
        assert_eq!(CommandContext::format_time_ago(604_800), "7d ago");
    }

    #[test]
    fn test_format_time_ago_negative() {
        assert_eq!(CommandContext::format_time_ago(-1), "unknown");
        assert_eq!(CommandContext::format_time_ago(-100), "unknown");
    }
}
