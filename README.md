# meshcore-cli-rs

Command line interface to MeshCore companion radios and repeaters, written in Rust.

It is a port of [meshcore-cli](https://github.com/meshcore-dev/meshcore-cli), the Python reference, and runs on top of [meshcore-rs](https://github.com/chrstnwhlrt/meshcore-rs). Commands, chaining, chat mode and the output in text and JSON follow the reference, so scripts written for one work with the other.

## Disclaimer

**This project is an independent, community-driven port of [meshcore-cli](https://github.com/meshcore-dev/meshcore-cli) to Rust.**

- This project is **not affiliated with, endorsed by, or officially associated with MeshCore** or its developers in any way.
- This is provided **as-is, without any warranty** of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and noninfringement.
- **No guarantee is made regarding functionality, correctness, reliability, or compatibility** with any particular MeshCore device or firmware version.
- Use at your own risk. The authors are not responsible for any damage to hardware, data loss, or other issues that may arise from using this software.
- Protocol implementation is based on reverse engineering and may be incomplete or incorrect.

## Status

The port follows meshcore-cli 1.6.10 (commit `34355c9` from 2026-11-11), with meshcore_py 2.3.19 as the reference for the protocol.

Devices are reached over a serial port (USB), TCP or BLE. BLE goes through BlueZ and is available on Linux only (feature `ble`, on by default); on other systems the BLE options are accepted and ignored.

Both CLIs were run with the same command lines against a simulated companion (over TCP and over a serial pseudo terminal), a simulated repeater console and a real companion radio with MeshCore firmware 1.18.0, over USB and over BLE. The outputs match, apart from the differences listed at the end of this file and values that change between two runs, like the battery voltage.

## Installation

Rust 1.95 or newer is needed. On Linux the build needs `pkg-config` and the headers of libdbus for BLE through BlueZ, on Debian and Ubuntu `libdbus-1-dev`. `--no-default-features` builds without BLE and needs neither.

```bash
cargo install --git https://github.com/chrstnwhlrt/meshcore-cli-rs
```

Or from a checkout:

```bash
git clone https://github.com/chrstnwhlrt/meshcore-cli-rs
cd meshcore-cli-rs
cargo build --release
# binary: target/release/meshcore-cli-rs
```

With Nix:

```bash
nix run github:chrstnwhlrt/meshcore-cli-rs -- -s /dev/ttyUSB0 infos
nix build github:chrstnwhlrt/meshcore-cli-rs
```

On Linux the user needs access to the serial port, usually through the `dialout` group:

```bash
sudo usermod -a -G dialout $USER
# log out and back in
```

## Usage

```
meshcore-cli-rs <args> <commands>
```

| Option | Meaning |
|--------|---------|
| `-a <address>`, `-d <name>` | BLE device: its address or a part of its name; without `-s` and `-t` the first MeshCore device found (or the last one used) is taken |
| `-P` | pair with the BLE device, asks for its PIN |
| `-f` | take the first BLE device found when connecting fails |
| `-T <seconds>` | BLE scan time (default 2) |
| `-s <port>[:opts]` | serial port, e.g. `/dev/ttyUSB0`; options are comma separated: `baud=`, `dtr=`, `rts=` |
| `-b <baudrate>` | baud rate (default 115200) |
| `-t <hostname>` | connect over TCP |
| `-p <port>` | TCP port (default 5000) |
| `-l` | list BLE devices and serial ports and exit |
| `-S` | choose a BLE device or serial port from a list |
| `-r` | repeater mode: text console of a repeater, use with `-s` |
| `-j` | JSON output (the init script is not run) |
| `-i` | go to chat mode after the commands |
| `-c <on/off>` | colors |
| `-C` | toggle the classic prompt |
| `-q` | quiet, only errors are logged |
| `-D` | debug logging |
| `-v` | version |
| `-h` | help for arguments and commands |

Commands can be chained, each one takes the arguments it needs:

```bash
meshcore-cli-rs -s /dev/ttyUSB0 sync_time msg Alice "hello" wait_ack
meshcore-cli-rs -t 192.168.1.20 -j get bat get radio
```

`-h` lists all commands with their short forms. `?<cmd>` explains a command in more detail, `get help` and `set help` list the parameters.

Some examples:

```bash
meshcore-cli-rs -d MyNode infos                    # BLE, device name contains MyNode
meshcore-cli-rs -s /dev/ttyUSB0 infos              # node information (json)
meshcore-cli-rs -s /dev/ttyUSB0 lc                 # contact list
meshcore-cli-rs -s /dev/ttyUSB0 sync_msgs          # read waiting messages
meshcore-cli-rs -s /dev/ttyUSB0 login Rep1 '$' rs Rep1   # '$' asks for the password
meshcore-cli-rs -s /dev/ttyUSB0 trace 5f,7c        # trace along two hops
meshcore-cli-rs -s /dev/ttyUSB0 apply_to "t=2,u>1d,d" "cn trace"
```

`apply_to` takes the filter and the commands as two arguments, so both need quotes in a shell when they contain spaces or `>`.

### JSON

With `-j` every command prints JSON formatted like Python's `json.dumps` in the reference: same key order, `", "` and `": "` as separators, non-ASCII characters escaped. A command prefixed with a dot (`.get bat`) prints JSON once, without `-j`.

### Chat mode

Chat mode starts when no command is given, with `-i`, or when the last argument is `chat`, `im` or `interactive`. Lines are parsed like the reference does it:

| Input | Meaning |
|-------|---------|
| `to <name>` | go to a contact or channel (`to public`, `to ch1`, `to #test`) |
| `to <name>%<scope>` | the same, with a flood scope |
| `to ..`, `to ~`, `to !` | previous recipient, back to the node, sender of the last message |
| text | message to the current contact or channel; at a repeater, room or sensor it is sent as a command, `send <text>` or a leading `"` sends a message instead |
| `/<cmd>`, `/<name>/<cmd>`, `/<name> <text>` | command at node level, command or message for another contact |
| `!<text>` | reply to the last received message |
| `$$<cmds>` | run the line like command line arguments |
| `:<cmd>` | at node level: run a CLI command on the device |
| `@<alias> <args>` | run an alias, see `?alias` |
| `> file <line>`, `>> file <line>` | write or append the output to a file |
| `\|<command> <line>` | send the output to a shell command |
| `<key\| <command> <line>` | run the command and put its output in place of `{key}` in the line (`<\|` for `{}`) |
| `a \| b` | run `a`, then `b` |

Tab completes commands, contacts and parameters. The history is kept in `~/.config/meshcore/history`, in the same format as the Python CLI, so both share it. `quit`, `q` or Ctrl+D leave chat mode.

### Handlers

`handler_attach <type> <command>` starts a command and writes every event of the type (`rxlog`, `msgs` or `chan_data`) as one JSON line to its stdin. What the command prints is shown. `handlers` lists the handlers, `handler_detach <n>` stops one.

```bash
meshcore-cli-rs -s /dev/ttyUSB0 handler_attach rxlog "jq -c .payload_typename" chat
```

### Repeater mode

`-r` talks to the text console on a repeater's serial port:

```bash
meshcore-cli-rs -r -s /dev/ttyUSB0 get radio     # one command
meshcore-cli-rs -r -s /dev/ttyUSB0               # interactive console
```

Besides the commands of the repeater firmware, the console knows `clock sync` (also `st` and `sync_time`, sets the repeater clock to the local time), `region download [file]`, `region upload [file]`, `script <file>` and `help`. The history is kept in `~/.config/meshcore/repeater_history`.

## Configuration

Everything lives in `~/.config/meshcore/` on all platforms. Files are only written when that directory exists (`mkdir -p ~/.config/meshcore`).

| File | Content |
|------|---------|
| `init` | commands run after connecting (not with `-j`) |
| `<node name>.init` | commands run after connecting to this node |
| `history`, `repeater_history`, `region_files_history` | input histories |
| `<node name>.msgs` | received and sent messages, one JSON object per line |
| `<public key>.pass` | password saved by `login` in chat mode, removed by `forget_password` (`fp`) |
| `scopes` | flood scope per channel, one `<channel name> <scope>` per line |
| `default_address` | address of the last BLE device, used when no device is given |

`aliases_load <file>` reads aliases from a JSON object of names and values.

## Differences to the Python reference

BLE works on Linux only, through BlueZ. `-P` asks for the PIN in the terminal; in the reference it starts the pairing and the system's Bluetooth agent asks for the PIN.

Fixed in this port, where the reference fails or prints wrong data:

- `path <ct>` with `-j` fails in the reference (it reads `out_path_hash_len`, which contacts do not have); here it prints the hash size in bytes under that name. Without `-j` the reference prints no newline after the path, so chained output runs together.
- `req_binary` calls a function meshcore_py does not have; here it sends the request and prints the answer.
- A trace to a contact (`trace` and `dtrace` in chat mode) uses the path hash size of the contact instead of the node setting, and 3-byte hashes are no longer reversed twice.
- `sync_msgs` does not stop at hidden channel data, `apply_to ... send` no longer changes the text for the following contacts, and the completion of sensor commands no longer changes the one of repeaters.
- `node_discover` collects every answer. The reference waits anew after each answer and can miss one that arrives right after it. It also checks for known contacts before loading them, so on the command line `add_discovered -a` adds known repeaters again under their key prefix; here the contacts are loaded first.
- Repeater mode keeps the case of file names and takes the rest of the line as the name, so it may contain spaces; the reference lowercases the name and takes only one word. It also does not crash when the port cannot be opened.
- A repeater mode script that runs another script goes on after it. The reference always stops it there with "Error during script execution, exiting", even when the inner script ran without error.
- `-c off` also removes the colors of channel data messages, channel and advert echoes and the repeater console.
- `to <name>` also finds channels whose name starts with "ch"; the reference reads every such name as a channel number and fails.
- A channel argument made of numeric characters that are not digits (like `三` or `½`) is a channel name. The reference takes it for a number and fails.
- `set telemetry_mode_base` accepts every word starting with "dev", like the location and environment modes; the reference only takes "dev" there.
- Contacts and adverts of a type above 4 show as `t:<type>`, as `node_discover` shows them. The reference fails there with a Python error: in `lc`, in `contact_type` in chat mode and in advert echoes, which then also skip `json_log_rx` and the handlers.
- `change_flags` reports "Error setting flags" instead of "Error setting path".
- `change_path` reports a hash mode below 0 (a first hash of one digit gives -1) or above 3 as an error; the reference fails there with a Python error.
- `reset_advert_time` with an unknown contact says so; the reference fails with a Python error.
- `get custom` and `get <variable>` with `-j` print a failed request as `{"error": ...}`; the reference fails with a Python error.
- `get private_key` on a device that does not export its key says so; the reference fails with a Python error.
- `sync_time` (also `st` and `clock sync`) reports a timeout or an error reply without a code like any other error and goes on with the next command. The reference fails there with a Python error and skips the commands after it.
- The node's own name and position follow `set name`, `set coords`, `set lat` and `set lon` at once: in the chat prompt, for `to` and `to <own name>`, in the message log and in the distances of `lc`, and `set lat` and `set lon` keep the other coordinate as last set. The reference keeps the values it read at the last `APP_START` (`infos` and the `get` commands read them again), so until then it shows the old name and measures from the old position, and `set lon 8 set lat 50` puts the old longitude back.
- `set tx` takes values up to 127. The reference sends larger ones too, and the device reads only their lowest byte, so 276 sets 20 dBm.
- `set radio` rounds the frequency to whole kHz and the bandwidth to whole Hz. The reference cuts off the rest after a float multiplication, so it sets 869.462 MHz for 869.4625 and 512.001 MHz for 512.002.
- `add_contact` with a key that is not 32 bytes long reports the command format. The reference sends it, and the device reads the fields after the key into it.
- After `reset_path` a contact reads `"out_path_hash_mode": -1` like one the device reports without a path; the reference keeps the old mode in its copy of the contact until the contacts are loaded again.
- `lc` adds the distances (`d` in the format or the sort order) to its own output only. The reference also stores them in its contacts, so they show up in later JSON of the same run (`contact_info`, `lc -j`).
- An `@alias` line with an unclosed quote is reported; the reference ends chat mode with a traceback.
- A line of the `scopes` file that is not `<channel> <scope>` is reported as an error. The reference means to do that too, but its message is lost because logging is set up only after the file is read.
- A `scopes` file, repeater mode script or region file that cannot be read or is not UTF-8 is reported as an error. The reference fails with a Python error, for `scopes` at every start and for a region file that is not UTF-8 after it sent part of it.
- A history file or `default_address` that cannot be read or written is left aside. With such a `default_address` the reference fails with a Python error at every start or after a BLE connection, and with such a history file prompt_toolkit shows errors instead of reading lines.
- Typos and invisible non-breaking spaces in help texts and output are fixed, `?trace` gives 2 as the flag value of 4 byte hashes (the reference says 3), and the help lists all commands and parameters. It no longer offers `e` as short form of `card`, which the reference lists but does not know.

Behaves differently on purpose:

- A wrong password is reported at once ("Login failed"). meshcore_py waits for the timeout, because it only listens for a successful login.
- `msgs_subscribe` and chat mode fetch all waiting messages and show them before the next command runs. meshcore_py fetches one and leaves the rest until the device announces new messages.
- `sleep` and `trywait_msg` accept fractions of seconds, and `get max_attempts` exists next to `set max_attempts`.
- Whole numbers beyond 64 bits are refused with "int too big to convert". Python reads them and fails only when it packs them for the device, partly with another message, or takes them as they are, for example as a retry count.
- `-D` keeps debug logging with `-j` and `-q`; the reference switches back to errors only once it is connected.
- Wrong options, failed connections and init scripts that cannot be read end with exit code 1 and the message of the reference; the reference ends with 0, after a failed connection or script also with a Python traceback.
- `-l` prints "No ble device found" when no MeshCore device is in range. The reference prints it only when it sees no Bluetooth device at all.
- A BLE device that another program on this computer is using is reported as in use after about 3 s. The reference tries to connect for 30 s, then scans for the device and reports that it cannot find it, since a connected device does not advertise.
- Device errors and timeouts read like `device error 6 (ERR_CODE_ILLEGAL_ARG)` or `timed out` instead of a Python object, and `handlers` leaves out internal Python objects.
- When `upload_card` or `upload_contact` cannot reach the map, the error message is the one of the HTTP library ureq instead of the one of Python's requests.
- Input the device cannot take whole is refused with the reason before anything is sent, also in chat mode: a text over 160 bytes (on a channel including the node name and ": "), a command over 176 bytes, a login password over 15 bytes, an empty message, node name or CLI command, a NUL character in a text or name, a `:` in the name of a custom variable, a private key that is not 64 bytes long, a contact card under 98 bytes, a raw packet under 2 bytes, a trace without hops, a path that is not a whole number of hops, a path over 63 hops or 64 bytes, a path hash mode above 2 (mode 3 is reserved) and coordinates beyond 90 degrees latitude or 180 degrees longitude. The reference sends it, and the device rejects it (a too long direct message with ERR_CODE_TABLE_FULL, coordinates with ERR_CODE_ILLEGAL_ARG, the short or empty ones as an unknown command), cuts it short or reads it differently. Apart from the private key, meshcore-rs makes these checks, and its README lists them. They assume companion firmware 1.16 or newer; older firmware takes frames of 172 instead of 176 bytes, so there a direct message or a command to a node holds at most 159 bytes.
- Further details follow meshcore-rs, whose README lists where it differs from meshcore_py: for example `reload_contacts` forgets contacts removed on the device, and names longer than the 31 bytes the device keeps are cut at a character boundary.
- The JSON of `req_neighbours` has no empty line after it.
- The line editor is rustyline instead of prompt_toolkit: Tab prints the candidates instead of opening a menu and completes the whole word before the cursor, where prompt_toolkit only looks at the part after a character like `.` or `-`, and `-S` shows a numbered list instead of a dialog. Tab also offers the commands and settings the reference leaves out of its completion, like `sync_msgs`, `recv` or `set arrow_head`. With input from a pipe or a file no prompt is printed; prompt_toolkit prints its prompts with the lines it reads and warns that the input is not a terminal.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo fmt --check
nix flake check        # package, clippy, rustfmt and tests
```

To build against a local checkout of meshcore-rs:

```bash
cargo build --config 'patch."https://github.com/chrstnwhlrt/meshcore-rs".meshcore.path="../meshcore-rs"'
```

## License

MIT License, see [LICENSE](LICENSE).

## Related Projects

- [meshcore-rs](https://github.com/chrstnwhlrt/meshcore-rs): the Rust library this CLI uses
- [meshcore-cli](https://github.com/meshcore-dev/meshcore-cli): the Python CLI this port follows
- [meshcore_py](https://github.com/meshcore-dev/meshcore_py): the Python library behind it

## Acknowledgments

This project is based on [meshcore-cli](https://github.com/meshcore-dev/meshcore-cli) by fdlamotte and the meshcore-dev community.
