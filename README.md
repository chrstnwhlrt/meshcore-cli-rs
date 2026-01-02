# meshcore-cli-rs

A Rust command-line interface to MeshCore companion radios over Serial/USB.

## Disclaimer

**This project is an independent, community-driven port of [meshcore-cli](https://github.com/meshcore-dev/meshcore-cli) to Rust.**

- This project is **not affiliated with, endorsed by, or officially associated with MeshCore** or its developers in any way.
- This is provided **as-is, without any warranty** of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and noninfringement.
- **No guarantee is made regarding functionality, correctness, reliability, or compatibility** with any particular MeshCore device or firmware version.
- Use at your own risk. The authors are not responsible for any damage to hardware, data loss, or other issues that may arise from using this software.
- Protocol implementation is based on reverse engineering and may be incomplete or incorrect.

## About

meshcore-cli-rs connects to your MeshCore companion radio node over Serial/USB and provides a terminal-based interface to interact with it. You can:

- Send commands as CLI arguments (for scripting)
- Use interactive mode with readline support and tab completion
- Chain multiple commands in a single invocation
- Output in human-readable or JSON format

**Note**: This tool only works with companion radios (via Serial/USB). You cannot connect directly to a repeater's serial interface.

## Installation

### From Source

```bash
git clone https://github.com/chrstnwhlrt/meshcore-cli-rs
cd meshcore-cli-rs
cargo build --release
```

The binary will be at `target/release/meshcore-cli-rs`.

### Using Nix

```bash
nix build github:chrstnwhlrt/meshcore-cli-rs
./result/bin/meshcore-cli-rs
```

## Usage

```bash
meshcore-cli-rs [OPTIONS] [COMMAND]
```

### Options

| Option | Description |
|--------|-------------|
| `-s <PORT>` | Serial port to use (e.g., `/dev/ttyUSB0`) |
| `-b <BAUD>` | Baud rate (default: 115200) |
| `-j` | JSON output mode (disables init scripts) |
| `-D` | Enable debug logging |
| `-l` | List available serial ports and exit |
| `-c <on/off>` | Enable/disable colored output |

### Configuration

Configuration files are stored in `~/.config/meshcore/`:

- `init` - Global init script, executed before commands
- `<device-name>.init` - Per-device init script (useful for setting contact timeouts)
- Command history is preserved between sessions

## Commands Reference

### General Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `chat` | `interactive`, `im` | Enter interactive chat mode |
| `chat_to <contact>` | `to`, `imto` | Enter chat with specific contact |
| `script <file>` | | Execute commands from file |
| `infos` | `i` | Print device information |
| `self_telemetry` | `t` | Print own telemetry data |
| `card` | `e` | Export this node's URI (contact card) |
| `ver` | `v`, `q` | Print firmware version |
| `reboot` | | Reboot the device |
| `sleep <secs>` | `s` | Sleep for given duration |
| `wait_key` | `wk` | Wait until user presses Enter |
| `apply_to <filter> <cmds>` | `at` | Apply commands to matching contacts |

### Messaging Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `msg <name> <text>` | `m` | Send private message |
| `wait_ack [timeout]` | `wa` | Wait for acknowledgment |
| `chan <n> <text>` | `ch` | Send message to channel number |
| `public <text>` | `dch` | Send to public channel (0) |
| `recv` | `r` | Read next message |
| `wait_msg [timeout]` | `wm` | Wait for a message |
| `sync_msgs` | `sm` | Get all unread messages |
| `trywait_msg <timeout>` | `wmt` | Try wait for message with timeout |
| `msgs_subscribe` | `ms` | Display messages as they arrive |
| `get_channels` | `gc` | Print all channel info |
| `get_channel <n>` | | Get channel by number/name |
| `set_channel <n> <name> [key]` | | Set channel configuration |
| `remove_channel <n>` | | Remove a channel |
| `add_channel <name> [key]` | | Add channel to first free slot |
| `scope <topic>` | | Set flood scope |

### Device Management

| Command | Alias | Description |
|---------|-------|-------------|
| `advert` | `a` | Send advertisement |
| `floodadv` | | Send flood advertisement |
| `get <param>` | | Get device parameter |
| `set <param> <value>` | | Set device parameter |
| `time <epoch>` | | Set device time |
| `clock [--sync]` | | Get/sync device clock |
| `sync_time` | `st` | Sync device clock to system |
| `node_discover [filter]` | `nd` | Discover nodes by type |
| `battery` | | Get battery status |
| `stats [core/radio/packets]` | | Get device statistics |

### Contact Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `contacts` / `list` | `lc` | Get contact list |
| `reload_contacts` | `rc` | Force reload all contacts |
| `contact_info <ct>` | `ci` | Print contact information |
| `contact_timeout <ct> <secs>` | | Set temporary timeout for contact |
| `share_contact <ct>` | `sc` | Share contact with others |
| `export_contact [ct]` | `ec` | Get contact's URI (or self) |
| `import_contact <uri>` | `ic` | Import contact from URI |
| `remove_contact <ct>` | | Remove a contact |
| `path <ct>` | | Display path to contact |
| `disc_path <ct>` | `dp` | Discover and display new path |
| `reset_path <ct>` | `rp` | Reset path to flood |
| `change_path <ct> <path>` | `cp` | Change path to contact |
| `change_flags <ct> <flags>` | `cf` | Change contact flags |
| `req_telemetry <ct>` | `rt` | Request telemetry from contact |
| `req_mma <ct>` | `rm` | Request min/max/avg data |
| `req_acl <ct>` | | Request access control list |
| `pending_contacts` | | Show pending contacts |
| `add_pending <key>` | | Add pending contact |
| `flush_pending` | | Flush pending contact list |

### Repeater Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `login <name> <pwd>` | `l` | Login to repeater |
| `logout <name>` | | Logout from repeater |
| `cmd <name> <cmd>` | `c` | Send command to repeater |
| `wmt8` | | Wait for message (8s timeout) |
| `req_status <name>` | `rs` | Request status from node |
| `req_neighbours <name>` | `rn` | Request neighbours list |
| `req_binary <name> <hex>` | `rb` | Send raw binary request |
| `trace <path>` | `tr` | Run trace through path |

### Advanced Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `export_key` | | Export private key |
| `import_key <hex>` | | Import private key |
| `get_vars` | | Get custom variables |
| `set_var <key> <value>` | | Set custom variable |

## Interactive Mode

Interactive mode (chat) provides a readline-like experience with:

- Command history (up/down arrows)
- Tab completion for commands and contacts
- Color-coded output
- Contact switching with `to` command

### Interactive Commands

```
to <dest>     # Switch to contact or channel
to /          # Go to root (your node)
to ~          # Go to root (alias)
to ..         # Switch to previous contact
to !          # Switch to last message sender
/<cmd>        # Execute command on root
/<dest>/<cmd> # Execute command on specific dest
/<dest> <msg> # Send message to dest
quit / q      # Exit interactive mode
```

### Contact Types in Interactive Mode

- **Chat nodes**: Sending text sends a message by default
- **Repeaters/Rooms**: Text is sent as commands; prefix with `"` for messages

## Apply To (Batch Commands)

The `apply_to` command executes commands on contacts matching a filter:

```bash
meshcore-cli-rs -s /dev/ttyUSB0 apply_to "t=2,d" "login password"
```

### Filter Syntax

| Filter | Description |
|--------|-------------|
| `t=<n>` | Type: 1=client, 2=repeater, 3=room, 4=sensor |
| `h<>n` | Hop count comparison (e.g., `h>2`, `h<0`, `h=1`) |
| `d` | Direct contacts only (equivalent to `h>-1`) |
| `f` | Flood contacts only (equivalent to `h<0`) |
| `u<>time` | Updated before/after time (supports `d`, `h`, `m` suffixes) |

### Examples

```bash
# Remove clients not updated in 2 days
meshcore-cli-rs apply_to "u<2d,t=1" remove_contact

# Login to all direct repeaters updated in last 24h
meshcore-cli-rs apply_to "t=2,u>1d,d" "login password"

# Reset path for all flood repeaters
meshcore-cli-rs apply_to "t=2,f" reset_path
```

## Examples

### Get Device Info

```bash
$ meshcore-cli-rs -s /dev/ttyUSB0 infos
Name: MyNode
Public Key: 993acd42fc779962c68c627829b32b111fa27a67d86b75c17460ff48c3102db4
TX Power: 22 dBm
Location: 47.794000, -3.428000
Radio: 869.525 MHz, BW 250kHz, SF11, CR 4/5
```

### JSON Output

```bash
$ meshcore-cli-rs -s /dev/ttyUSB0 -j infos
{
  "name": "MyNode",
  "public_key": "993acd42...",
  "tx_power": 22,
  ...
}
```

### Send and Wait for ACK

```bash
$ meshcore-cli-rs -s /dev/ttyUSB0 msg Alice "Hello!" wait_ack
Message sent (ack: 4802ed93)
Message acknowledged!
```

### Sync Clock and Verify

```bash
$ meshcore-cli-rs -s /dev/ttyUSB0 sync_time clock
Clock synchronized
Current time: 2026-01-01 12:00:00 (1735732800)
```

### List Contacts

```bash
$ meshcore-cli-rs -s /dev/ttyUSB0 contacts
Contacts (15):
  Alice          a1b2c3d4e5f6  [D:2]  47.80,-3.43
  Bob_Repeater   b2c3d4e5f6a1  [F]    48.12,-2.98
  Room_Server    c3d4e5f6a1b2  [D:1]  -
```

### Interactive Session

```bash
$ meshcore-cli-rs -s /dev/ttyUSB0 chat
MyNode> to Alice
Alice> Hello there!
Alice(D): Hi! How are you?
Alice> I'm good, thanks!
Alice(D): Great to hear!
Alice> to Bob_Repeater
Bob_Repeater> clock
Bob_Repeater(0): 12:00 - 1/1/2026 UTC
Bob_Repeater> quit
$
```

### Script Execution

```bash
$ cat commands.txt
sync_time
advert
contacts

$ meshcore-cli-rs -s /dev/ttyUSB0 script commands.txt
Clock synchronized
Advertisement sent
Contacts (15):
...
```

## Get/Set Parameters

### Get Parameters

```bash
meshcore-cli-rs get help              # List all parameters
meshcore-cli-rs get name              # Device name
meshcore-cli-rs get coords            # GPS coordinates
meshcore-cli-rs get tx_power          # TX power in dBm
meshcore-cli-rs get radio             # Radio configuration
meshcore-cli-rs get telemetry_mode    # Telemetry settings
```

### Set Parameters

```bash
meshcore-cli-rs set name "MyDevice"
meshcore-cli-rs set coords 47.5,8.5
meshcore-cli-rs set tx_power 20
meshcore-cli-rs set radio 869.525,250,11,5
meshcore-cli-rs set manual_add_contacts on
```

## Requirements

- Rust 1.85+ (Edition 2024)
- Serial/USB access to MeshCore companion device
- Linux/macOS/Windows (with appropriate serial port permissions)

### Linux Serial Permissions

On Linux, add your user to the `dialout` group:

```bash
sudo usermod -a -G dialout $USER
# Log out and back in for changes to take effect
```

## Dependencies

This CLI uses [meshcore-rs](https://github.com/chrstnwhlrt/meshcore-rs) as the underlying library.

## License

MIT License - see [LICENSE](LICENSE) file.

## Related Projects

- [meshcore-rs](https://github.com/chrstnwhlrt/meshcore-rs) - Rust library (dependency)
- [meshcore-cli](https://github.com/meshcore-dev/meshcore-cli) - Original Python CLI (reference implementation)
- [meshcore_py](https://github.com/fdlamotte/meshcore_py) - Original Python library

## Contributing

Contributions are welcome! Please note that this is an independent community project.

## Acknowledgments

This project is inspired by and based on [meshcore-cli](https://github.com/meshcore-dev/meshcore-cli) by fdlamotte and the meshcore-dev community.
