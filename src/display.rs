//! Output display formatting.
//!
//! Handles JSON vs human-readable output formatting.

use std::io;

use chrono::{DateTime, TimeZone, Utc};
use crossterm::ExecutableCommand;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use meshcore::event::StatsData;
use meshcore::types::{BatteryStatus, Channel, Contact, ContactType, DeviceInfo, SelfInfo};
use serde::Serialize;
use serde_json::{Value, json};

/// Output mode for the CLI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutputMode {
    /// Human-readable output.
    #[default]
    Human,
    /// JSON output.
    Json,
}

/// Display configuration.
#[derive(Debug, Clone)]
pub struct Display {
    /// Output mode.
    pub mode: OutputMode,
    /// Color enabled.
    pub color: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            mode: OutputMode::Human,
            color: true,
        }
    }
}

impl Display {
    /// Creates a new display with the given mode.
    #[must_use]
    pub fn new(json: bool, color: bool) -> Self {
        Self {
            mode: if json {
                OutputMode::Json
            } else {
                OutputMode::Human
            },
            color,
        }
    }

    /// Returns true if JSON output is enabled.
    #[must_use]
    pub const fn is_json(&self) -> bool {
        matches!(self.mode, OutputMode::Json)
    }

    /// Prints a JSON value. Only prints if JSON mode is enabled.
    pub fn print_json<T: Serialize>(&self, value: &T) {
        if self.is_json() {
            if let Ok(json) = serde_json::to_string_pretty(value) {
                println!("{json}");
            }
        }
    }

    /// Prints a success message.
    pub fn print_ok(&self, message: &str) {
        if self.is_json() {
            self.print_json(&json!({ "ok": message }));
        } else {
            self.print_colored(message, Color::Green);
        }
    }

    /// Prints an error message.
    pub fn print_error(&self, message: &str) {
        if self.is_json() {
            self.print_json(&json!({ "error": message }));
        } else {
            self.print_colored(&format!("Error: {message}"), Color::Red);
        }
    }

    /// Prints a warning message.
    pub fn print_warning(&self, message: &str) {
        if self.is_json() {
            self.print_json(&json!({ "warning": message }));
        } else {
            self.print_colored(&format!("Warning: {message}"), Color::Yellow);
        }
    }

    /// Prints colored text.
    fn print_colored(&self, text: &str, color: Color) {
        if self.color {
            let mut stdout = io::stdout();
            let _ = stdout.execute(SetForegroundColor(color));
            let _ = stdout.execute(Print(text));
            let _ = stdout.execute(ResetColor);
            println!();
        } else {
            println!("{text}");
        }
    }

    /// Prints self info.
    pub fn print_self_info(&self, info: &SelfInfo) {
        if self.is_json() {
            self.print_json(&json!({
                "adv_type": info.advert_type,
                "tx_power": info.tx_power,
                "max_tx_power": info.max_tx_power,
                "public_key": info.public_key.to_hex(),
                "adv_lat": info.latitude,
                "adv_lon": info.longitude,
                "multi_acks": info.multi_acks,
                "adv_loc_policy": info.advert_loc_policy,
                "telemetry_mode_base": info.telemetry_mode.base,
                "telemetry_mode_loc": info.telemetry_mode.loc,
                "telemetry_mode_env": info.telemetry_mode.env,
                "manual_add_contacts": info.manual_add_contacts,
                "radio_freq": info.radio.frequency_mhz,
                "radio_bw": info.radio.bandwidth_khz,
                "radio_sf": info.radio.spreading_factor,
                "radio_cr": info.radio.coding_rate,
                "name": info.name,
            }));
        } else {
            println!("Name: {}", info.name);
            println!("Public Key: {}", info.public_key.to_hex());
            println!("TX Power: {} / {} dBm", info.tx_power, info.max_tx_power);
            if let (Some(lat), Some(lon)) = (info.latitude, info.longitude) {
                println!("Location: {lat:.6}, {lon:.6}");
            }
            println!(
                "Radio: {:.3} MHz, {:.1} kHz BW, SF{}, CR 4/{}",
                info.radio.frequency_mhz,
                info.radio.bandwidth_khz,
                info.radio.spreading_factor,
                info.radio.coding_rate
            );
            println!("Multi-ACKs: {}", info.multi_acks);
            println!(
                "Telemetry: base={}, loc={}, env={}",
                info.telemetry_mode.base, info.telemetry_mode.loc, info.telemetry_mode.env
            );
            println!("Manual add contacts: {}", info.manual_add_contacts);
        }
    }

    /// Prints device info.
    pub fn print_device_info(&self, info: &DeviceInfo) {
        if self.is_json() {
            self.print_json(&json!({
                "firmware_version": info.firmware_version,
                "max_contacts": info.max_contacts,
                "max_channels": info.max_channels,
                "ble_pin": info.ble_pin,
                "build": info.build,
                "model": info.model,
                "version": info.version,
            }));
        } else {
            println!("Firmware Version: {}", info.firmware_version);
            if let Some(version) = &info.version {
                println!("Version: {version}");
            }
            if let Some(model) = &info.model {
                println!("Model: {model}");
            }
            if let Some(build) = &info.build {
                println!("Build: {build}");
            }
            if let Some(max_contacts) = info.max_contacts {
                println!("Max Contacts: {max_contacts}");
            }
            if let Some(max_channels) = info.max_channels {
                println!("Max Channels: {max_channels}");
            }
            if let Some(pin) = info.ble_pin {
                println!("BLE PIN: {pin:06}");
            }
        }
    }

    /// Prints battery status.
    pub fn print_battery(&self, battery: &BatteryStatus) {
        if self.is_json() {
            self.print_json(&json!({
                "millivolts": battery.millivolts,
                "voltage": f64::from(battery.millivolts) / 1000.0,
                "used_kb": battery.used_kb,
                "total_kb": battery.total_kb,
            }));
        } else {
            let voltage = f64::from(battery.millivolts) / 1000.0;
            println!("Battery: {voltage:.2}V ({} mV)", battery.millivolts);
            if let (Some(used), Some(total)) = (battery.used_kb, battery.total_kb) {
                let percent = if total > 0 {
                    (f64::from(used) / f64::from(total)) * 100.0
                } else {
                    0.0
                };
                println!("Storage: {used} / {total} KB ({percent:.1}% used)");
            }
        }
    }

    /// Prints current time.
    pub fn print_time(&self, timestamp: u32) {
        if self.is_json() {
            self.print_json(&json!({ "time": timestamp }));
        } else {
            let dt: DateTime<Utc> = Utc
                .timestamp_opt(i64::from(timestamp), 0)
                .single()
                .unwrap_or_else(Utc::now);
            println!(
                "Current time: {} ({timestamp})",
                dt.format("%Y-%m-%d %H:%M:%S")
            );
        }
    }

    /// Prints a contact.
    pub fn print_contact(&self, contact: &Contact) {
        if self.is_json() {
            self.print_json(&contact_to_json(contact));
        } else {
            let type_str = match contact.device_type {
                ContactType::Node => "Node",
                ContactType::Repeater => "Repeater",
                ContactType::Room => "Room",
                ContactType::Unknown => "Unknown",
            };

            let path_str = match contact.out_path_len.cmp(&0) {
                std::cmp::Ordering::Less => "flood".to_string(),
                std::cmp::Ordering::Equal => "direct".to_string(),
                std::cmp::Ordering::Greater => format!("{} hops", contact.out_path_len),
            };

            println!(
                "{} ({}) - {} [{}]",
                contact.name,
                type_str,
                contact.public_key.to_hex(),
                path_str
            );
            if let (Some(lat), Some(lon)) = (contact.latitude, contact.longitude) {
                println!("  Location: {lat:.6}, {lon:.6}");
            }
        }
    }

    /// Prints a contact list.
    pub fn print_contacts(&self, contacts: &[Contact]) {
        if self.is_json() {
            let json_contacts: Vec<Value> = contacts.iter().map(contact_to_json).collect();
            self.print_json(&json_contacts);
        } else {
            for contact in contacts {
                self.print_contact(contact);
            }
            println!("\nTotal: {} contacts", contacts.len());
        }
    }

    /// Prints a channel.
    pub fn print_channel(&self, channel: &Channel) {
        if self.is_json() {
            self.print_json(&json!({
                "index": channel.index,
                "name": channel.name,
                "secret": hex::encode(channel.secret),
            }));
        } else {
            println!(
                "Channel {}: {} (secret: {})",
                channel.index,
                channel.name,
                hex::encode(channel.secret)
            );
        }
    }

    /// Prints statistics.
    pub fn print_stats(&self, stats: &StatsData) {
        match stats {
            StatsData::Core(s) => {
                if self.is_json() {
                    self.print_json(&json!({
                        "type": "core",
                        "battery_mv": s.battery_mv,
                        "uptime_secs": s.uptime_secs,
                        "errors": s.errors,
                        "queue_len": s.queue_len,
                    }));
                } else {
                    let voltage = f64::from(s.battery_mv) / 1000.0;
                    let uptime_hours = s.uptime_secs / 3600;
                    let uptime_mins = (s.uptime_secs % 3600) / 60;
                    println!("Core Statistics:");
                    println!("  Battery: {voltage:.2}V");
                    println!("  Uptime: {uptime_hours}h {uptime_mins}m");
                    println!("  Errors: {}", s.errors);
                    println!("  Queue: {}", s.queue_len);
                }
            }
            StatsData::Radio(s) => {
                if self.is_json() {
                    self.print_json(&json!({
                        "type": "radio",
                        "noise_floor": s.noise_floor,
                        "rssi": s.rssi,
                        "snr": s.snr,
                        "tx_airtime_secs": s.tx_airtime_secs,
                        "rx_airtime_secs": s.rx_airtime_secs,
                    }));
                } else {
                    println!("Radio Statistics:");
                    println!("  Noise Floor: {} dBm", s.noise_floor);
                    println!("  Last RSSI: {} dBm", s.rssi);
                    println!("  Last SNR: {:.2} dB", s.snr);
                    println!("  TX Airtime: {}s", s.tx_airtime_secs);
                    println!("  RX Airtime: {}s", s.rx_airtime_secs);
                }
            }
            StatsData::Packets(s) => {
                if self.is_json() {
                    self.print_json(&json!({
                        "type": "packets",
                        "received": s.received,
                        "sent": s.sent,
                        "flood_tx": s.flood_tx,
                        "direct_tx": s.direct_tx,
                        "flood_rx": s.flood_rx,
                        "direct_rx": s.direct_rx,
                    }));
                } else {
                    println!("Packet Statistics:");
                    println!(
                        "  Received: {} (flood: {}, direct: {})",
                        s.received, s.flood_rx, s.direct_rx
                    );
                    println!(
                        "  Sent: {} (flood: {}, direct: {})",
                        s.sent, s.flood_tx, s.direct_tx
                    );
                }
            }
        }
    }

    /// Prints a message.
    pub fn print_message(
        &self,
        sender: &str,
        text: &str,
        is_command: bool,
        snr: Option<f32>,
        rssi: Option<i8>,
    ) {
        if self.is_json() {
            self.print_json(&json!({
                "sender": sender,
                "text": text,
                "is_command": is_command,
                "snr": snr,
                "rssi": rssi,
            }));
        } else {
            let signal = match (snr, rssi) {
                (Some(s), Some(r)) => format!(" [{s:.2}/{r}]"),
                (Some(s), None) => format!(" [{s:.2}]"),
                _ => String::new(),
            };
            let prefix = if is_command { "$" } else { "" };
            println!("{sender}{signal}: {prefix}{text}");
        }
    }

    /// Prints message sent confirmation.
    pub fn print_msg_sent(&self, expected_ack: u32, timeout_ms: u32) {
        if self.is_json() {
            self.print_json(&json!({
                "type": 0,
                "expected_ack": format!("{expected_ack:08x}"),
                "suggested_timeout": timeout_ms,
            }));
        } else {
            // Human mode: just silently wait for ack usually
        }
    }

    /// Prints ACK received.
    pub fn print_ack(&self, code: u32) {
        if self.is_json() {
            self.print_json(&json!({
                "code": format!("{code:08x}"),
            }));
        } else {
            self.print_colored("Msg acked", Color::Green);
        }
    }

    /// Prints no more messages.
    pub fn print_no_more_messages(&self) {
        if self.is_json() {
            self.print_json(&json!({ "no_more_messages": true }));
        } else {
            println!("No more messages");
        }
    }
}

/// Converts a contact to JSON value.
fn contact_to_json(contact: &Contact) -> Value {
    json!({
        "name": contact.name,
        "public_key": contact.public_key.to_hex(),
        "type": match contact.device_type {
            ContactType::Unknown => 0,
            ContactType::Node => 1,
            ContactType::Repeater => 2,
            ContactType::Room => 3,
        },
        "type_name": match contact.device_type {
            ContactType::Unknown => "unknown",
            ContactType::Node => "node",
            ContactType::Repeater => "repeater",
            ContactType::Room => "room",
        },
        "flags": contact.flags.as_byte(),
        "path_len": contact.out_path_len,
        "path": hex::encode(&contact.out_path),
        "latitude": contact.latitude,
        "longitude": contact.longitude,
        "last_advert": contact.last_advert,
        "last_modified": contact.last_modified,
    })
}
