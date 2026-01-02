//! Command line argument parsing.

use clap::{Parser, Subcommand, ValueEnum};

/// `MeshCore` CLI - Command line interface to `MeshCore` companion radios.
#[derive(Parser, Debug)]
#[command(name = "meshcore-cli-rs")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// JSON output mode (disables init file).
    #[arg(short = 'j', long, global = true)]
    pub json: bool,

    /// Debug logging.
    #[arg(short = 'D', long, global = true)]
    pub debug: bool,

    /// Serial port to use.
    #[arg(short = 's', long, value_name = "PORT")]
    pub serial: Option<String>,

    /// Baud rate for serial port.
    #[arg(short = 'b', long, value_name = "BAUD", default_value = "115200")]
    pub baudrate: u32,

    /// Disable color output.
    #[arg(short = 'c', long, value_name = "on/off", value_parser = parse_bool_arg)]
    pub color: Option<bool>,

    /// List available serial ports.
    #[arg(short = 'l', long)]
    pub list: bool,

    /// Commands to execute (can be chained).
    #[command(subcommand)]
    pub command: Option<Command>,
}

fn parse_bool_arg(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "on" | "true" | "1" | "yes" => Ok(true),
        "off" | "false" | "0" | "no" => Ok(false),
        _ => Err(format!(
            "Invalid value: {s}. Use on/off, true/false, or 1/0"
        )),
    }
}

/// CLI commands.
#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    // ==================== General Commands ====================
    /// Enter interactive chat mode.
    #[command(visible_aliases = ["interactive", "im"])]
    Chat,

    /// Enter chat with a specific contact.
    #[command(visible_aliases = ["to", "imto"], name = "chat_to")]
    ChatTo {
        /// Contact name or public key prefix.
        contact: String,
    },

    /// Execute commands from a script file.
    Script {
        /// Script file path.
        filename: String,
    },

    /// Print device information.
    #[command(visible_alias = "i")]
    Infos,

    /// Get local telemetry data.
    #[command(visible_alias = "t", name = "self_telemetry")]
    SelfTelemetry,

    /// Export this node's URI (contact card).
    #[command(visible_alias = "e")]
    Card,

    /// Print firmware version.
    #[command(visible_aliases = ["v", "q", "query"])]
    Ver,

    /// Reboot the device.
    Reboot,

    /// Sleep for a given duration.
    #[command(visible_alias = "s")]
    Sleep {
        /// Duration in seconds.
        secs: f64,
    },

    /// Wait for user to press Enter.
    #[command(visible_alias = "wk", name = "wait_key")]
    WaitKey,

    /// Apply commands to contacts matching a filter.
    #[command(visible_alias = "at", name = "apply_to")]
    ApplyTo {
        /// Filter expression (e.g., "t=2,d" for direct repeaters).
        filter: String,
        /// Commands to apply.
        #[arg(trailing_var_arg = true)]
        commands: Vec<String>,
    },

    // ==================== Messaging Commands ====================
    /// Send a private message.
    #[command(visible_alias = "m")]
    Msg {
        /// Recipient name or public key prefix.
        name: String,
        /// Message text.
        #[arg(trailing_var_arg = true)]
        message: Vec<String>,
        /// Wait for ACK after sending.
        #[arg(short, long)]
        wait: bool,
        /// Timeout in seconds when waiting for ACK.
        #[arg(short, long, default_value = "30")]
        timeout: u64,
    },

    /// Wait for ACK.
    #[command(visible_alias = "wa", name = "wait_ack")]
    WaitAck {
        /// Timeout in seconds.
        #[arg(default_value = "30")]
        timeout: u64,
    },

    /// Send a channel message.
    #[command(visible_alias = "ch")]
    Chan {
        /// Channel number (0-7).
        channel: u8,
        /// Message text.
        #[arg(trailing_var_arg = true)]
        message: Vec<String>,
    },

    /// Send a message to the public channel (0).
    #[command(visible_alias = "dch")]
    Public {
        /// Message text.
        #[arg(trailing_var_arg = true)]
        message: Vec<String>,
    },

    /// Read the next message.
    #[command(visible_alias = "r")]
    Recv,

    /// Wait for a message and read it.
    #[command(visible_alias = "wm", name = "wait_msg")]
    WaitMsg {
        /// Timeout in seconds.
        #[arg(default_value = "30")]
        timeout: u64,
    },

    /// Try wait for a message with configurable timeout.
    #[command(visible_alias = "wmt", name = "trywait_msg")]
    TrywaitMsg {
        /// Timeout in seconds.
        timeout: u64,
    },

    /// Get all unread messages from the device.
    #[command(visible_alias = "sm", name = "sync_msgs")]
    SyncMsgs,

    /// Subscribe to incoming messages (display as they arrive).
    #[command(visible_alias = "ms", name = "msgs_subscribe")]
    MsgsSubscribe,

    /// Get all channel information.
    #[command(visible_alias = "gc", name = "get_channels")]
    GetChannels,

    /// Get channel information by number or name.
    #[command(name = "get_channel")]
    GetChannel {
        /// Channel number or name.
        channel: String,
    },

    /// Set channel information.
    #[command(name = "set_channel")]
    SetChannel {
        /// Channel number.
        number: u8,
        /// Channel name.
        name: String,
        /// Channel key (optional, derived from name if starts with #).
        key: Option<String>,
    },

    /// Remove a channel.
    #[command(name = "remove_channel")]
    RemoveChannel {
        /// Channel number or name.
        channel: String,
    },

    /// Add a new channel (auto-assigns to first free slot).
    #[command(name = "add_channel")]
    AddChannel {
        /// Channel name.
        name: String,
        /// Channel key (optional, derived from name if starts with #).
        key: Option<String>,
    },

    /// Set flood scope.
    Scope {
        /// Scope topic or "*" for global.
        scope: String,
    },

    // ==================== Management Commands ====================
    /// Send an advertisement.
    #[command(visible_alias = "a")]
    Advert,

    /// Send a flood advertisement.
    #[command(visible_alias = "flood_advert", name = "floodadv")]
    FloodAdv,

    /// Get a device parameter.
    Get {
        /// Parameter name (use "help" for list).
        param: String,
    },

    /// Set a device parameter.
    Set {
        /// Parameter name (use "help" for list).
        param: String,
        /// Parameter value.
        value: String,
    },

    /// Set device time.
    Time {
        /// Unix epoch timestamp.
        epoch: u32,
    },

    /// Get current device time.
    Clock {
        /// Sync device clock to system time.
        #[arg(long)]
        sync: bool,
    },

    /// Sync device clock to system time.
    #[command(visible_alias = "st", name = "sync_time")]
    SyncTime,

    /// Discover nodes by type.
    #[command(visible_alias = "nd", name = "node_discover")]
    NodeDiscover {
        /// Filter by device type.
        #[arg(default_value = "0")]
        filter: u8,
    },

    // ==================== Contact Commands ====================
    /// Get contact list.
    #[command(visible_alias = "lc")]
    Contacts,

    /// Alias for contacts.
    List,

    /// Force reload all contacts.
    #[command(visible_alias = "rc", name = "reload_contacts")]
    ReloadContacts,

    /// Print contact information.
    #[command(visible_alias = "ci", name = "contact_info")]
    ContactInfo {
        /// Contact name or public key prefix.
        contact: String,
    },

    /// Set temporary timeout for a contact.
    #[command(name = "contact_timeout")]
    ContactTimeout {
        /// Contact name or public key prefix.
        contact: String,
        /// Timeout in seconds.
        timeout: u64,
    },

    /// Share a contact with others.
    #[command(visible_alias = "sc", name = "share_contact")]
    ShareContact {
        /// Contact name or public key prefix.
        contact: String,
    },

    /// Export a contact's URI.
    #[command(visible_alias = "ec", name = "export_contact")]
    ExportContact {
        /// Contact name or public key prefix (empty for self).
        contact: Option<String>,
    },

    /// Import a contact from URI.
    #[command(visible_alias = "ic", name = "import_contact")]
    ImportContact {
        /// Contact URI.
        uri: String,
    },

    /// Remove a contact.
    #[command(name = "remove_contact")]
    RemoveContact {
        /// Contact name or public key prefix.
        contact: String,
    },

    /// Display path to a contact.
    Path {
        /// Contact name or public key prefix.
        contact: String,
    },

    /// Discover path to a contact.
    #[command(visible_alias = "dp", name = "disc_path")]
    DiscPath {
        /// Contact name or public key prefix.
        contact: String,
    },

    /// Reset path to a contact (use flood).
    #[command(visible_alias = "rp", name = "reset_path")]
    ResetPath {
        /// Contact name or public key prefix.
        contact: String,
    },

    /// Change path to a contact.
    #[command(visible_alias = "cp", name = "change_path")]
    ChangePath {
        /// Contact name or public key prefix.
        contact: String,
        /// New path (comma-separated public key prefixes).
        path: String,
    },

    /// Change contact flags.
    #[command(visible_alias = "cf", name = "change_flags")]
    ChangeFlags {
        /// Contact name or public key prefix.
        contact: String,
        /// Flags to set/unset (e.g., "trusted", "hidden", "star").
        flags: String,
    },

    /// Request telemetry from a contact.
    #[command(visible_alias = "rt", name = "req_telemetry")]
    ReqTelemetry {
        /// Contact name or public key prefix.
        contact: String,
    },

    /// Request min/max/avg data from a sensor.
    #[command(visible_alias = "rm", name = "req_mma")]
    ReqMma {
        /// Contact name or public key prefix.
        contact: String,
    },

    /// Request ACL from a sensor.
    #[command(name = "req_acl")]
    ReqAcl {
        /// Contact name or public key prefix.
        contact: String,
    },

    /// Show pending contacts.
    #[command(name = "pending_contacts")]
    PendingContacts,

    /// Add a pending contact.
    #[command(name = "add_pending")]
    AddPending {
        /// Pending contact key or name.
        pending: String,
    },

    /// Flush all pending contacts.
    #[command(name = "flush_pending")]
    FlushPending,

    // ==================== Repeater Commands ====================
    /// Login to a repeater.
    #[command(visible_alias = "l")]
    Login {
        /// Repeater name or public key prefix.
        name: String,
        /// Password.
        password: String,
    },

    /// Logout from a repeater.
    Logout {
        /// Repeater name or public key prefix.
        name: String,
    },

    /// Send a command to a repeater.
    #[command(visible_alias = "c")]
    Cmd {
        /// Repeater name or public key prefix.
        name: String,
        /// Command to send.
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
        /// Wait for ACK after sending.
        #[arg(short, long)]
        wait: bool,
        /// Timeout in seconds when waiting for ACK.
        #[arg(short, long, default_value = "30")]
        timeout: u64,
    },

    /// Wait for message with 8 second timeout.
    #[command(name = "wmt8")]
    Wmt8,

    /// Request status from a node.
    #[command(visible_alias = "rs", name = "req_status")]
    ReqStatus {
        /// Node name or public key prefix.
        name: String,
    },

    /// Request neighbours from a node.
    #[command(visible_alias = "rn", name = "req_neighbours")]
    ReqNeighbours {
        /// Node name or public key prefix.
        name: String,
    },

    /// Send a raw binary request to a node.
    #[command(visible_alias = "rb", name = "req_binary")]
    ReqBinary {
        /// Node name or public key prefix.
        name: String,
        /// Hex data (first byte is request type: 01=Status, 02=KeepAlive, 03=Telemetry, 04=MMA, 05=ACL, 06=Neighbours).
        data: String,
    },

    /// Run a trace through the specified path.
    #[command(visible_alias = "tr")]
    Trace {
        /// Comma-separated path of public key prefixes.
        path: String,
    },

    // ==================== Advanced Commands ====================
    /// Get battery status.
    Battery,

    /// Get device statistics.
    Stats {
        /// Stats type.
        #[arg(value_enum, default_value = "core")]
        stats_type: StatsTypeArg,
    },

    /// Export private key.
    #[command(name = "export_key")]
    ExportKey,

    /// Import private key.
    #[command(name = "import_key")]
    ImportKey {
        /// Key in hex format (64 bytes).
        key: String,
    },

    /// Get custom variables.
    #[command(name = "get_vars")]
    GetVars,

    /// Set a custom variable.
    #[command(name = "set_var")]
    SetVar {
        /// Variable name.
        key: String,
        /// Variable value.
        value: String,
    },
}

/// Statistics type argument.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum StatsTypeArg {
    /// Core statistics (battery, uptime, errors).
    Core,
    /// Radio statistics (RSSI, SNR, airtime).
    Radio,
    /// Packet statistics (sent, received, flood/direct).
    Packets,
}
