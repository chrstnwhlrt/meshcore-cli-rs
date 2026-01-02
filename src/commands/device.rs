//! Device-related commands (infos, ver, clock, reboot, battery, stats, etc.).

use std::time::Duration;

use meshcore::event::Event;
use meshcore::protocol::StatsType;

use super::{CommandContext, current_timestamp};
use crate::cli::StatsTypeArg;
use crate::error::{CliError, Result};

impl CommandContext {
    /// Executes the `infos` command.
    pub async fn cmd_infos(&self) -> Result<()> {
        let event = self.commands().await.app_start().await?;

        match event {
            Event::SelfInfo(info) => {
                // Update session state
                let mut state = self.state.lock().await;
                state.device_name = Some(info.name.clone());
                drop(state);

                self.display.print_self_info(&info);
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

    /// Executes the `ver` command.
    pub async fn cmd_ver(&self) -> Result<()> {
        let event = self.commands().await.device_query().await?;

        match event {
            Event::DeviceInfo(info) => {
                self.display.print_device_info(&info);
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

    /// Executes the `battery` command.
    pub async fn cmd_battery(&self) -> Result<()> {
        let event = self.commands().await.get_battery().await?;

        match event {
            Event::Battery(battery) => {
                self.display.print_battery(&battery);
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

    /// Executes the `clock` command.
    pub async fn cmd_clock(&self, sync: bool) -> Result<()> {
        if sync {
            return self.cmd_sync_time().await;
        }

        let event = self.commands().await.get_time().await?;

        match event {
            Event::CurrentTime(timestamp) => {
                self.display.print_time(timestamp);
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

    /// Executes the `sync_time` command.
    pub async fn cmd_sync_time(&self) -> Result<()> {
        let timestamp = current_timestamp();
        self.commands().await.set_time(timestamp).await?;
        self.display.print_ok("time synced");
        Ok(())
    }

    /// Executes the `time` command (set time).
    pub async fn cmd_set_time(&self, epoch: u32) -> Result<()> {
        self.commands().await.set_time(epoch).await?;
        self.display.print_ok("time set");
        Ok(())
    }

    /// Executes the `reboot` command.
    pub async fn cmd_reboot(&self) -> Result<()> {
        self.commands().await.reboot().await?;
        self.display.print_ok("rebooting");
        Ok(())
    }

    /// Executes the `sleep` command.
    pub async fn cmd_sleep(&self, secs: f64) -> Result<()> {
        tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        Ok(())
    }

    /// Executes the `wait_key` command - waits for user to press Enter.
    pub fn cmd_wait_key() {
        use std::io::{self, BufRead};
        println!("Press Enter to continue...");
        let stdin = io::stdin();
        let _ = stdin.lock().lines().next();
    }

    /// Executes the `advert` command.
    pub async fn cmd_advert(&self, flood: bool) -> Result<()> {
        self.commands().await.send_advert(flood).await?;
        self.display.print_ok(if flood {
            "flood advert sent"
        } else {
            "advert sent"
        });
        Ok(())
    }

    /// Executes the `stats` command.
    pub async fn cmd_stats(&self, stats_type: StatsTypeArg) -> Result<()> {
        let st = match stats_type {
            StatsTypeArg::Core => StatsType::Core,
            StatsTypeArg::Radio => StatsType::Radio,
            StatsTypeArg::Packets => StatsType::Packets,
        };

        let event = self.commands().await.get_stats(st).await?;

        match event {
            Event::Stats(stats) => {
                self.display.print_stats(&stats);
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

    /// Executes the `card` command (export self URI).
    pub async fn cmd_card(&self) -> Result<()> {
        let event = self.commands().await.export_contact(None).await?;

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

    /// Executes the `get` command.
    pub async fn cmd_get(&self, param: &str) -> Result<()> {
        match param.to_lowercase().as_str() {
            "help" => {
                println!("Available parameters:");
                println!("  time          - Current device time");
                println!("  battery / bat - Battery status");
                println!("  name          - Device name");
                println!("  txpower / tx  - TX power");
                println!("  radio         - Radio parameters");
                println!("  coords        - Device coordinates");
                println!("  telemetry     - Telemetry mode");
                println!("  channels      - Channel list");
                println!("  stats         - Device statistics");
                println!("  stats_radio   - Radio statistics");
                println!("  stats_packets - Packet statistics");
                println!("  fstats        - Filesystem statistics");
                println!("  vars / custom - Custom variables");
                Ok(())
            }
            "time" | "clock" => self.cmd_clock(false).await,
            "battery" | "bat" => self.cmd_battery().await,
            "stats" | "fstats" => self.cmd_stats(StatsTypeArg::Core).await,
            "stats_radio" => self.cmd_stats(StatsTypeArg::Radio).await,
            "stats_packets" => self.cmd_stats(StatsTypeArg::Packets).await,
            "vars" | "variables" | "custom" => self.cmd_get_vars().await,
            "channels" => self.cmd_get_channels().await,
            "name" | "txpower" | "tx" | "radio" | "telemetry" | "coords" => {
                // These require infos command
                self.cmd_infos().await
            }
            _ => Err(CliError::InvalidArgument(format!(
                "Unknown parameter: {param}. Use 'get help' for list."
            ))),
        }
    }

    /// Executes the `set` command.
    pub async fn cmd_set(&self, param: &str, value: &str) -> Result<()> {
        match param.to_lowercase().as_str() {
            "help" => {
                println!("Available parameters:");
                println!("  name <value>              - Device name");
                println!("  time <epoch>              - Device time");
                println!("  txpower / tx <dBm>        - TX power");
                println!("  coords <lat> <lon>        - Device coordinates");
                println!("  lat <latitude>            - Latitude only");
                println!("  lon <longitude>           - Longitude only");
                println!("  pin <pin>                 - BLE PIN");
                println!("  radio <f>,<bw>,<sf>,<cr>  - Radio parameters");
                println!("  tuning <af>,<tx_delay>    - Tuning parameters");
                println!("  manual_add_contacts on/off - Manual contact approval");
                println!("  multi_acks on/off         - Multi-ACK mode");
                println!("  telemetry_mode_base <m>   - Base telemetry (never/device/always)");
                println!("  telemetry_mode_loc <m>    - Location telemetry");
                println!("  telemetry_mode_env <m>    - Environment telemetry");
                println!("  advert_loc_policy <p>     - Advert location (none/share)");
                println!("  var <key> <value>         - Custom variable");
                Ok(())
            }
            "name" => {
                self.commands().await.set_name(value).await?;
                self.display.print_ok("name set");
                Ok(())
            }
            "time" => {
                let epoch: u32 = value
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid epoch value".into()))?;
                self.cmd_set_time(epoch).await
            }
            "txpower" | "tx_power" | "tx" => {
                let power: i32 = value
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid power value".into()))?;
                self.commands().await.set_tx_power(power).await?;
                self.display.print_ok("TX power set");
                Ok(())
            }
            "pin" => {
                let pin: u32 = value
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid PIN value".into()))?;
                self.commands().await.set_device_pin(pin).await?;
                self.display.print_ok("PIN set");
                Ok(())
            }
            "coords" | "coordinates" => {
                let parts: Vec<&str> = value.split_whitespace().collect();
                if parts.len() != 2 {
                    return Err(CliError::InvalidArgument(
                        "Usage: set coords <lat> <lon>".into(),
                    ));
                }
                let lat: f64 = parts[0]
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid latitude".into()))?;
                let lon: f64 = parts[1]
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid longitude".into()))?;
                self.commands().await.set_coords(lat, lon).await?;
                self.display.print_ok("coordinates set");
                Ok(())
            }
            "lat" | "latitude" => {
                let lat: f64 = value
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid latitude".into()))?;
                // Get current lon from device to preserve it
                let client = self.client.lock().await;
                let current_lon = client
                    .self_info()
                    .await
                    .and_then(|info| info.longitude)
                    .unwrap_or(0.0);
                drop(client);
                self.commands().await.set_coords(lat, current_lon).await?;
                self.display.print_ok("latitude set");
                Ok(())
            }
            "lon" | "longitude" => {
                let lon: f64 = value
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid longitude".into()))?;
                // Get current lat from device to preserve it
                let client = self.client.lock().await;
                let current_lat = client
                    .self_info()
                    .await
                    .and_then(|info| info.latitude)
                    .unwrap_or(0.0);
                drop(client);
                self.commands().await.set_coords(current_lat, lon).await?;
                self.display.print_ok("longitude set");
                Ok(())
            }
            "radio" => {
                // Format: freq,bw,sf,cr (comma or space separated)
                let parts: Vec<&str> = value.split([',', ' ']).filter(|s| !s.is_empty()).collect();
                if parts.len() != 4 {
                    return Err(CliError::InvalidArgument(
                        "Usage: set radio <freq_mhz>,<bw_khz>,<sf>,<cr>".into(),
                    ));
                }
                let freq: f64 = parts[0]
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid frequency".into()))?;
                let bw: f64 = parts[1]
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid bandwidth".into()))?;
                let sf: u8 = parts[2]
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid spreading factor".into()))?;
                let cr: u8 = parts[3]
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid coding rate".into()))?;
                self.commands().await.set_radio(freq, bw, sf, cr).await?;
                self.display.print_ok("radio parameters set");
                Ok(())
            }
            "tuning" => {
                // Format: af,tx_delay (comma or space separated)
                let parts: Vec<&str> = value.split([',', ' ']).filter(|s| !s.is_empty()).collect();
                if parts.len() != 2 {
                    return Err(CliError::InvalidArgument(
                        "Usage: set tuning <af>,<tx_delay>".into(),
                    ));
                }
                let af: i32 = parts[0]
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid AF value".into()))?;
                let tx_delay: i32 = parts[1]
                    .parse()
                    .map_err(|_| CliError::InvalidArgument("Invalid TX delay".into()))?;
                self.commands().await.set_tuning(tx_delay, af).await?;
                self.display.print_ok("tuning parameters set");
                Ok(())
            }
            "manual_add_contacts" => {
                let enabled = matches!(value.to_lowercase().as_str(), "on" | "true" | "yes" | "1");
                self.set_other_param(|info| info.manual_add_contacts = enabled)
                    .await?;
                self.display.print_ok(&format!(
                    "manual_add_contacts: {}",
                    if enabled { "on" } else { "off" }
                ));
                Ok(())
            }
            "multi_acks" => {
                let enabled = matches!(value.to_lowercase().as_str(), "on" | "true" | "yes" | "1");
                self.set_other_param(|info| info.multi_acks = u8::from(enabled))
                    .await?;
                self.display.print_ok(&format!(
                    "multi_acks: {}",
                    if enabled { "on" } else { "off" }
                ));
                Ok(())
            }
            "telemetry_mode_base" => {
                let mode = Self::parse_telemetry_mode(value)?;
                self.set_other_param(|info| info.telemetry_mode.base = mode)
                    .await?;
                self.display
                    .print_ok(&format!("telemetry_mode_base: {mode}"));
                Ok(())
            }
            "telemetry_mode_loc" => {
                let mode = Self::parse_telemetry_mode(value)?;
                self.set_other_param(|info| info.telemetry_mode.loc = mode)
                    .await?;
                self.display
                    .print_ok(&format!("telemetry_mode_loc: {mode}"));
                Ok(())
            }
            "telemetry_mode_env" => {
                let mode = Self::parse_telemetry_mode(value)?;
                self.set_other_param(|info| info.telemetry_mode.env = mode)
                    .await?;
                self.display
                    .print_ok(&format!("telemetry_mode_env: {mode}"));
                Ok(())
            }
            "advert_loc_policy" => {
                let policy: u8 = match value.to_lowercase().as_str() {
                    "none" | "0" => 0,
                    "share" | "1" => 1,
                    _ => {
                        return Err(CliError::InvalidArgument(
                            "Invalid policy. Use: none, share".into(),
                        ));
                    }
                };
                self.set_other_param(|info| info.advert_loc_policy = policy)
                    .await?;
                self.display.print_ok(&format!(
                    "advert_loc_policy: {}",
                    if policy == 0 { "none" } else { "share" }
                ));
                Ok(())
            }
            _ => Err(CliError::InvalidArgument(format!(
                "Unknown parameter: {param}. Use 'set help' for list."
            ))),
        }
    }

    /// Parses telemetry mode value.
    fn parse_telemetry_mode(value: &str) -> Result<u8> {
        match value.to_lowercase().as_str() {
            "never" | "0" => Ok(0),
            "device" | "1" => Ok(1),
            "always" | "2" => Ok(2),
            _ => Err(CliError::InvalidArgument(
                "Invalid mode. Use: never, device, always (or 0, 1, 2)".into(),
            )),
        }
    }

    /// Sets a single "other" parameter by reading current values, modifying, and writing back.
    async fn set_other_param<F>(&self, modify: F) -> Result<()>
    where
        F: FnOnce(&mut meshcore::SelfInfo),
    {
        // Get current self_info
        let client = self.client.lock().await;
        let mut info = client
            .self_info()
            .await
            .ok_or_else(|| CliError::Command("Device info not available".into()))?;
        drop(client);

        // Apply modification
        modify(&mut info);

        // Encode telemetry mode as single byte
        let telemetry_byte = (info.telemetry_mode.env << 4)
            | (info.telemetry_mode.loc << 2)
            | info.telemetry_mode.base;

        // Set other params
        self.commands()
            .await
            .set_other_params(
                info.manual_add_contacts,
                telemetry_byte,
                info.advert_loc_policy,
                info.multi_acks,
            )
            .await?;

        Ok(())
    }

    /// Executes the `get_vars` command.
    pub async fn cmd_get_vars(&self) -> Result<()> {
        let event = self.commands().await.get_custom_vars().await?;

        match event {
            Event::CustomVars(vars) => {
                if self.display.is_json() {
                    // Parse comma-separated key:value pairs
                    let mut map = serde_json::Map::new();
                    for pair in vars.split(',') {
                        if let Some((k, v)) = pair.split_once(':') {
                            map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
                        }
                    }
                    self.display.print_json(&serde_json::Value::Object(map));
                } else if vars.is_empty() {
                    println!("No custom variables set");
                } else {
                    for pair in vars.split(',') {
                        println!("{pair}");
                    }
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

    /// Executes the `set_var` command.
    pub async fn cmd_set_var(&self, key: &str, value: &str) -> Result<()> {
        self.commands().await.set_custom_var(key, value).await?;
        self.display.print_ok("variable set");
        Ok(())
    }

    /// Executes the `export_key` command.
    pub async fn cmd_export_key(&self) -> Result<()> {
        let event = self.commands().await.export_private_key().await?;

        match event {
            Event::PrivateKey(key) => {
                let hex = hex::encode(key);
                if self.display.is_json() {
                    self.display
                        .print_json(&serde_json::json!({ "private_key": hex }));
                } else {
                    println!("{hex}");
                }
            }
            Event::Disabled => {
                return Err(CliError::Command("Private key export is disabled".into()));
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

    /// Executes the `import_key` command.
    pub async fn cmd_import_key(&self, key_hex: &str) -> Result<()> {
        let key_bytes = hex::decode(key_hex)
            .map_err(|_| CliError::InvalidArgument("Invalid hex key".into()))?;

        if key_bytes.len() != 32 {
            return Err(CliError::InvalidArgument(
                "Key must be exactly 32 bytes (64 hex chars)".into(),
            ));
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&key_bytes);

        self.commands().await.import_private_key(&key).await?;
        self.display.print_ok("private key imported");
        Ok(())
    }

    /// Executes the `scope` command.
    pub async fn cmd_scope(&self, scope: &str) -> Result<()> {
        if scope == "*" {
            self.commands().await.clear_flood_scope().await?;
            let mut state = self.state.lock().await;
            state.flood_scope = None;
        } else {
            // Hash the scope topic
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(scope.as_bytes());
            let hash = hasher.finalize();
            let mut key = [0u8; 16];
            key.copy_from_slice(&hash[..16]);

            self.commands().await.set_flood_scope(&key).await?;
            let mut state = self.state.lock().await;
            state.flood_scope = Some(scope.to_string());
        }

        self.display.print_ok(&format!("scope set to {scope}"));
        Ok(())
    }

    /// Executes the `node_discover` command.
    pub async fn cmd_node_discover(&self, filter: u8) -> Result<()> {
        self.commands()
            .await
            .node_discover(filter, true, None, None)
            .await?;
        self.display.print_ok("discovery started");
        Ok(())
    }

    /// Executes the `self_telemetry` command.
    pub async fn cmd_self_telemetry(&self) -> Result<()> {
        let event = self.commands().await.get_self_telemetry().await?;

        match event {
            Event::TelemetryResponse(telemetry) => {
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
                    self.display.print_json(&serde_json::json!({
                        "readings": readings,
                    }));
                } else {
                    println!("Local telemetry:");
                    for reading in &telemetry.readings {
                        println!("  Channel {}: {:?}", reading.channel, reading.value);
                    }
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

    /// Executes the `script` command (interactive mode version).
    /// Runs a script file containing commands.
    pub async fn cmd_script(&self, filename: &str) -> Result<()> {
        let content = std::fs::read_to_string(filename).map_err(|e| CliError::Script {
            line: 0,
            message: format!("Failed to read script: {e}"),
        })?;

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse as interactive command
            let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
            let cmd = parts[0].to_lowercase();
            let args = parts.get(1).unwrap_or(&"");

            // Execute using the interactive mode command handler
            if let Err(e) = self.execute_interactive_cmd(&cmd, args).await {
                return Err(CliError::Script {
                    line: line_num + 1,
                    message: e.to_string(),
                });
            }
        }

        Ok(())
    }

    /// Executes a command in interactive mode style.
    async fn execute_interactive_cmd(&self, cmd: &str, args: &str) -> Result<()> {
        let args_vec: Vec<String> = if args.is_empty() {
            Vec::new()
        } else {
            args.split_whitespace().map(String::from).collect()
        };

        match cmd {
            "infos" | "i" => self.cmd_infos().await,
            "ver" | "v" => self.cmd_ver().await,
            "battery" => self.cmd_battery().await,
            "clock" => self.cmd_clock(false).await,
            "sync_time" | "st" => self.cmd_sync_time().await,
            "reboot" => self.cmd_reboot().await,
            "advert" | "a" => self.cmd_advert(false).await,
            "floodadv" => self.cmd_advert(true).await,
            "card" | "e" => self.cmd_card().await,
            "self_telemetry" | "t" => self.cmd_self_telemetry().await,
            "contacts" | "list" | "lc" => self.cmd_contacts().await,
            "reload_contacts" | "rc" => self.cmd_reload_contacts().await,
            "contact_info" | "ci" if !args.is_empty() => self.cmd_contact_info(args.trim()).await,
            "path" if !args.is_empty() => self.cmd_path(args.trim()).await,
            "disc_path" | "dp" if !args.is_empty() => self.cmd_disc_path(args.trim()).await,
            "reset_path" | "rp" if !args.is_empty() => self.cmd_reset_path(args.trim()).await,
            "pending_contacts" => self.cmd_pending_contacts().await,
            "flush_pending" => self.cmd_flush_pending().await,
            "add_pending" if !args.is_empty() => self.cmd_add_pending(args.trim()).await,
            "change_path" | "cp" if args_vec.len() >= 2 => {
                self.cmd_change_path(&args_vec[0], &args_vec[1]).await
            }
            "change_flags" | "cf" if args_vec.len() >= 2 => {
                self.cmd_change_flags(&args_vec[0], &args_vec[1]).await
            }
            "share_contact" | "sc" if !args.is_empty() => self.cmd_share_contact(args.trim()).await,
            "export_contact" | "ec" => {
                let contact = if args.is_empty() {
                    None
                } else {
                    Some(args.trim())
                };
                self.cmd_export_contact(contact).await
            }
            "import_contact" | "ic" if !args.is_empty() => {
                self.cmd_import_contact(args.trim()).await
            }
            "remove_contact" if !args.is_empty() => self.cmd_remove_contact(args.trim()).await,
            "msg" | "m" | "{" if args_vec.len() >= 2 => {
                self.cmd_msg(&args_vec[0], &args_vec[1..], false, 30).await
            }
            "recv" | "r" => self.cmd_recv().await,
            "sync_msgs" | "sm" => self.cmd_sync_msgs().await,
            "wait_ack" | "wa" | "}" => {
                let timeout = args_vec.first().and_then(|s| s.parse().ok()).unwrap_or(30);
                self.cmd_wait_ack(timeout).await
            }
            "wait_msg" | "wm" => {
                let timeout = args_vec.first().and_then(|s| s.parse().ok()).unwrap_or(30);
                self.cmd_wait_msg(timeout).await
            }
            "trywait_msg" | "wmt" if !args.is_empty() => {
                let timeout: u64 = args.trim().parse().unwrap_or(8);
                self.cmd_trywait_msg(timeout).await
            }
            "chan" | "ch" if args_vec.len() >= 2 => {
                let channel: u8 = args_vec[0].parse().unwrap_or(0);
                self.cmd_chan(channel, &args_vec[1..]).await
            }
            "public" | "dch" if !args.is_empty() => self.cmd_public(&[args.to_string()]).await,
            "login" | "l" if args_vec.len() >= 2 => {
                self.cmd_login(&args_vec[0], &args_vec[1]).await
            }
            "logout" if !args.is_empty() => self.cmd_logout(args.trim()).await,
            "cmd" | "c" | "[" if args_vec.len() >= 2 => {
                self.cmd_cmd(&args_vec[0], &args_vec[1..], false, 30).await
            }
            "req_status" | "rs" if !args.is_empty() => self.cmd_req_status(args.trim()).await,
            "wmt8" | "]" => self.cmd_wmt8().await,
            "trace" | "tr" if !args.is_empty() => self.cmd_trace(args.trim()).await,
            "req_binary" | "rb" if args_vec.len() >= 2 => {
                self.cmd_req_binary(&args_vec[0], &args_vec[1]).await
            }
            "req_neighbours" | "rn" if !args.is_empty() => {
                self.cmd_req_neighbours(args.trim()).await
            }
            "req_telemetry" | "rt" if !args.is_empty() => self.cmd_req_telemetry(args.trim()).await,
            "req_mma" | "rm" if !args.is_empty() => self.cmd_req_mma(args.trim()).await,
            "req_acl" if !args.is_empty() => self.cmd_req_acl(args.trim()).await,
            "get_channels" | "gc" => self.cmd_get_channels().await,
            "get_channel" if !args.is_empty() => self.cmd_get_channel(args.trim()).await,
            "set_channel" if args_vec.len() >= 3 => {
                let num: u8 = args_vec[0].parse().unwrap_or(0);
                let key = args_vec.get(2).map(String::as_str);
                self.cmd_set_channel(num, &args_vec[1], key).await
            }
            "add_channel" if !args.is_empty() => {
                let key = args_vec.get(1).map(String::as_str);
                self.cmd_add_channel(&args_vec[0], key).await
            }
            "remove_channel" if !args.is_empty() => self.cmd_remove_channel(args.trim()).await,
            "scope" if !args.is_empty() => self.cmd_scope(args.trim()).await,
            "node_discover" | "nd" => {
                let filter: u8 = args.trim().parse().unwrap_or(0);
                self.cmd_node_discover(filter).await
            }
            "contact_timeout" if args_vec.len() >= 2 => {
                let timeout: u64 = args_vec[1].parse().unwrap_or(30);
                self.cmd_contact_timeout(&args_vec[0], timeout).await
            }
            "time" if !args.is_empty() => {
                let epoch: u32 = args.trim().parse().unwrap_or(0);
                self.cmd_set_time(epoch).await
            }
            "get" if !args.is_empty() => self.cmd_get(args.trim()).await,
            "set" if args_vec.len() >= 2 => {
                self.cmd_set(&args_vec[0], &args_vec[1..].join(" ")).await
            }
            "stats" => {
                let st = match args.trim() {
                    "radio" => crate::cli::StatsTypeArg::Radio,
                    "packets" => crate::cli::StatsTypeArg::Packets,
                    _ => crate::cli::StatsTypeArg::Core,
                };
                self.cmd_stats(st).await
            }
            "sleep" | "s" => {
                let secs: f64 = args.trim().parse().unwrap_or(1.0);
                self.cmd_sleep(secs).await
            }
            "export_key" => self.cmd_export_key().await,
            "import_key" if !args.is_empty() => self.cmd_import_key(args.trim()).await,
            "get_vars" => self.cmd_get_vars().await,
            "set_var" if args_vec.len() >= 2 => {
                self.cmd_set_var(&args_vec[0], &args_vec[1..].join(" "))
                    .await
            }
            _ => Err(CliError::Command(format!("Unknown command: {cmd}"))),
        }
    }

    /// Executes the `apply_to` command.
    pub async fn cmd_apply_to(&self, filter: &str, commands: &[String]) -> Result<()> {
        use meshcore::types::ContactType;
        use std::time::{SystemTime, UNIX_EPOCH};

        // Get all contacts
        let client = self.client.lock().await;
        let contacts = client.contacts().await;
        drop(client);
        let mut matching: Vec<_> = contacts.values().cloned().collect();

        // Parse filter criteria
        let mut contact_type: Option<u8> = None;
        let mut min_hops: Option<i8> = None;
        let mut max_hops: Option<i8> = None;
        let mut upd_before: Option<u32> = None;
        let mut upd_after: Option<u32> = None;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u32::try_from(d.as_secs()).unwrap_or(u32::MAX))
            .unwrap_or(0);

        // Apply filters
        for filter_part in filter.split(',') {
            let filter_part = filter_part.trim();
            if filter_part.is_empty() || filter_part == "all" {
                continue;
            }

            if let Some(val) = filter_part.strip_prefix("t=") {
                contact_type = val.parse().ok();
            } else if filter_part == "d" {
                min_hops = Some(0);
            } else if filter_part == "f" {
                max_hops = Some(-1);
            } else if let Some(val) = filter_part.strip_prefix("h>") {
                min_hops = val.parse::<i8>().ok().map(|v| v + 1);
            } else if let Some(val) = filter_part.strip_prefix("h<") {
                max_hops = val.parse::<i8>().ok().map(|v| v - 1);
            } else if let Some(val) = filter_part.strip_prefix("h=") {
                let parsed = val.parse::<i8>().ok();
                min_hops = parsed;
                max_hops = parsed;
            } else if let Some(val) = filter_part.strip_prefix("u<") {
                let time_offset = super::parse_time_value(val);
                upd_before = Some(now.saturating_sub(time_offset));
            } else if let Some(val) = filter_part.strip_prefix("u>") {
                let time_offset = super::parse_time_value(val);
                upd_after = Some(now.saturating_sub(time_offset));
            }
        }

        // Filter contacts
        matching.retain(|c| {
            let ctype = match c.device_type {
                ContactType::Unknown => 0,
                ContactType::Node => 1,
                ContactType::Repeater => 2,
                ContactType::Room => 3,
            };

            if let Some(t) = contact_type {
                if ctype != t {
                    return false;
                }
            }
            if let Some(min) = min_hops {
                if c.out_path_len < min {
                    return false;
                }
            }
            if let Some(max) = max_hops {
                if c.out_path_len > max {
                    return false;
                }
            }
            if let Some(before) = upd_before {
                if c.last_modified >= before {
                    return false;
                }
            }
            if let Some(after) = upd_after {
                if c.last_modified <= after {
                    return false;
                }
            }
            true
        });

        // Execute commands on matching contacts
        let cmd_line = commands.join(" ");
        let count = matching.len();

        for contact in matching {
            println!("Applying to {}...", contact.name);

            if cmd_line == "remove_contact" {
                if let Err(e) = self.cmd_remove_contact(&contact.name).await {
                    self.display
                        .print_error(&format!("{}: {}", contact.name, e));
                }
            } else if cmd_line.starts_with("send ") || cmd_line.starts_with('"') {
                let msg = if let Some(stripped) = cmd_line.strip_prefix("send ") {
                    stripped
                } else {
                    cmd_line.trim_start_matches('"').trim_end_matches('"')
                };
                let message = vec![msg.to_string()];
                if let Err(e) = self.cmd_msg(&contact.name, &message, false, 30).await {
                    self.display
                        .print_error(&format!("{}: {}", contact.name, e));
                }
            } else {
                let ctype = matches!(
                    contact.device_type,
                    ContactType::Repeater | ContactType::Room
                );
                if ctype {
                    let cmd_parts: Vec<String> =
                        cmd_line.split_whitespace().map(String::from).collect();
                    if let Err(e) = self.cmd_cmd(&contact.name, &cmd_parts, false, 30).await {
                        self.display
                            .print_error(&format!("{}: {}", contact.name, e));
                    }
                } else {
                    self.display.print_warning(&format!(
                        "Can't send '{}' to {} (not a repeater)",
                        cmd_line, contact.name
                    ));
                }
            }
        }

        println!("{count} contacts matched filter");
        Ok(())
    }
}
