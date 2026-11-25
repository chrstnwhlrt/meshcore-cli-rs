//! Help texts, taken from the reference implementation.

use crate::util::ansi;

/// Program name used in messages.
pub const PROGRAM: &str = "meshcore-cli-rs";

/// The `-v` version line.
#[must_use]
pub fn version() -> String {
    format!(
        "{PROGRAM}: command line interface to MeshCore companion radios v{}",
        env!("CARGO_PKG_VERSION")
    )
}

/// The argument section of the usage text, with the empty line after it
/// that the reference's `print()` adds.
#[must_use]
pub fn command_usage() -> String {
    let no_ble = if crate::connect::BLE {
        ""
    } else {
        "\n BLE needs Linux and the ble feature; -a, -d, -P, -f and -T are ignored.\n"
    };
    format!(
        r"
   Usage : {PROGRAM} <args> <commands>

 Arguments :
    -h : prints help for arguments and commands
    -v : prints version
    -i : force interactive loop after executing commands
    -j : json output (disables init file)
    -D : debug (sets logging to DEBUG)
    -q : quiet (sets logging to ERROR)
    -S : scan for devices and show a selector
    -l : list available ble/serial devices and exit
    -T <timeout>    : timeout for the ble scan (-S and -l) default 2s
    -a <address>    : specifies device address (can be a name)
    -d <name>       : filter meshcore devices with name or address
    -P              : pairs with the device (asks for its PIN)
    -f              : connects to the first device found on retry
    -t <hostname>   : connects via tcp/ip
    -p <port>       : specifies tcp port (default 5000)
    -s <port>[:opts]: use serial port <port>
                        opts are comma separated options
                            rts=, dtr=, baud=
    -b <baudrate>   : specify baudrate
    -C              : toggles classic mode for prompt
    -c <on/off>     : disables most of color output if off
    -r              : repeater mode (raw text CLI, use with -s)
{no_ble}
"
    )
}

/// Overview of all commands.
#[must_use]
pub fn command_help() -> &'static str {
    r#"    ?<cmd> may give you some more help about cmd
  General commands
    script <filename>      : execute commands in filename
    infos                  : print information about the node       i
    self_telemetry         : print own telemetry                    t
    card                   : export this node URI
    upload_card            : upload this node URI to map.meshcore.dev
    ver                    : firmware version                       v
    reboot                 : reboots node
    sleep <secs>           : sleeps for a given amount of secs      s
    wait_key               : wait until user presses <Enter>        wk
    apply_to <f> <cmds>    : sends cmds to contacts matching f      at
    alias <key> <cmd>      : create an alias for key, see ?alias
    aliases                : list all aliases
    aliases_load           : load aliases from a file
    handler_attach <d> <c> : attach c as a handler for d (rxlog|msgs|chan_data)
    handler_detach <n>     : detach an handler (from its handle n)
    handlers               : list attached handlers
    echo <text>            : prints text
  Messaging
    msg <name> <msg>       : send message to node by name           m  {
    wait_ack               : wait an ack                            wa }
    chan <nb> <msg>        : send message to channel number <nb>    ch
    chan_data [-p p] c t d : send data d of type t to channel c
    public <msg>           : send message to public channel (0)     dch
    recv                   : reads next msg                         r
    wait_msg               : wait for a message and read it         wm
    trywait_msg <secs>     : wait at most secs for a message        wmt
    sync_msgs              : gets all unread msgs from the node     sm
    msgs_subscribe         : display msgs as they arrive            ms
    get_channels           : prints all channel info                gc
    get_channel <n>        : get info for channel (by number or name)
    set_channel n nm [k]   : set channel info (nb, name, key)
    add_channel name [key] : add new channel with optional key
    remove_channel <n>     : remove channel (by number or name)
    scope <s>              : sets scope for flood messages
  Management
    advert                 : sends advert                           a
    floodadv               : flood advert
    get <param>            : gets a param, "get help" for more
    set <param> <value>    : sets a param, "set help" for more
    time <epoch>           : sets time to given epoch
    clock                  : get current time
    clock sync             : sync device clock                      st
    node_discover <filter> : discovers nodes based on their type    nd
    cli <cmd>              : executes cli command cmd
    send_raw <pkt>         : sends a raw packet (hex string)
  Contacts
    contacts / list        : gets contact list                      lc
    reload_contacts        : force reloading all contacts           rc
    contact_info <ct>      : prints information for contact ct      ci
    contact_timeout <ct> v : sets temp default timeout for contact
    share_contact <ct>     : share a contact with others            sc
    export_contact <ct>    : get a contact's URI                    ec
    upload_contact <ct>    : upload a contact's URI to the map      uc
    import_contact <URI>   : import a contact from its URI          ic
    remove_contact <ct>    : removes a contact from this node
    add_contact <k> <t> <n>: adds contact: key type name
    reset_advert_time <ct> : resets the last advert time of a contact
    path <ct>              : displays path for a contact
    disc_path <ct>         : discover new path and display          dp
    reset_path <ct>        : resets path to a contact to flood      rp
    change_path <ct> <pth> : change the path to a contact           cp
    advert_path <key>      : get path from advert                   ap
    change_flags <ct> <f>  : change contact flags (tel_l|tel_a|star) cf
    req_acl <ct>           : requests access control list for node  ra
    req_telemetry <ct>     : prints telemetry data as json          rt
    req_regions <ct>       : prints regions from repeater           rr
    req_owner <ct>         : prints owner for a repeater            ro
    req_clock <ct>         : prints repeater timestamp (for sync)
    req_mma <ct> <f> <t>   : requests min/max/avg for a sensor      rm
    req_binary <ct> <hex>  : sends a binary request
    pending_contacts       : show pending contacts
    add_pending <pending>  : manually add pending contact
    flush_pending          : flush pending contact list
  Repeaters
    login <name> <pwd>     : log into a node (rep) with given pwd   l
    logout <name>          : log out of a repeater
    cmd <name> <cmd>       : sends a command to a repeater (no ack) c  [
    wmt8                   : wait for a msg (reply) with a timeout     ]
    req_status <name>      : requests status from a node            rs
    req_neighbours <name>  : requests for neighbours in binary form rn
    node_discover          : launch a local node discovery
    list_discovered        : list nodes found by the last discovery
    add_discovered <key>   : add a discovered node (repeater) to the list
    trace <path>           : run a trace, path is comma separated   tr"#
}

/// The full `-h` output.
#[must_use]
pub fn usage() -> String {
    format!(
        "{}\n{} Available Commands and shortcuts (can be chained) :\n{}",
        version(),
        command_usage(),
        command_help()
    )
}

/// Detailed help for `?<cmd>`.
#[must_use]
pub fn help_for(cmdname: &str) -> String {
    let text = match cmdname {
        "apply_to" | "at" => APPLY_TO_HELP,
        "node_discover" | "nd" | "list_discovered" | "add_discovered" => ND_HELP,
        "get" => GET_HELP,
        "set" => SET_HELP,
        "scope" => SCOPE_HELP,
        "contact_info" => CONTACT_INFO_HELP,
        "pending_contacts" | "flush_pending" | "add_pending" | "autoadd" | "add_contact" => {
            CONTACTS_HELP
        }
        c if c.contains("channel") => CHANNEL_HELP,
        "trace" | "tr" => TRACE_HELP,
        c if c.contains("path") => PATH_HELP,
        c if c.contains("alias") || c == "@" => ALIAS_HELP,
        "lc" | "contacts" | "list" => LC_HELP,
        _ => return format!("Sorry, no help yet for {cmdname}\n"),
    };
    // the reference prints each text with an empty line after it
    format!("{text}\n")
}

const APPLY_TO_HELP: &str = r"apply_to <f> <cmd> : applies cmd to contacts matching filter <f>
    Filter is constructed with comma separated fields :
     - u, matches modification time < or > than a timestamp
            (can also be days hours or minutes ago if followed by d,h or m)
     - a, same as u but for advert time
     - t, matches the type (1: client, 2: repeater, 3: room, 4: sensor)
     - h, matches number of hops
     - d, direct, similar to h>-1
     - f, flood, similar to h<0 or h=-1
     - b, show nodes that have all specified flag bits on (bit 0 is favourite)

    Note: Some commands like contact_name (aka cn), contact_key (aka ck), contact_type (aka ct), reset_path (aka rp), forget_password (aka fp) can be chained. There is also a sleep command taking an optional event. The sleep will be issued after the command, it helps limiting rate through repeaters ...

    Examples:
        # removes all clients that have not been updated in last 2 days
        at u<2d,t=1 remove_contact
        # gives traces to repeaters that have been updated in the last 24h and are direct
        at t=2,u>1d,d cn trace
        # tries to do flood login to all repeaters
        at t=2 rp login
";

const ND_HELP: &str = r#"node_discover [-t<filter>] [-f] : discovers 0-hop nodes and displays signal info

    `-t` asks for nodes regarding filter.
    filter can be an integer value or "all" for all types or nodes or a comma separated list consisting of :
     - cli or comp for companions
     - rep for repeaters
     - sens for sensors
     - room for chat rooms

    `-f` or `-full`: will ask for the full key of the node

    BEWARE: filter option have changed since v1.6.4, using -t is now mandatory !!!

    Once node_discover has been performed with -f (to get full key) you can add the node using `add_discovered` or list them with `list_discovered`. The discovered list is emptied at each node discovery.

add_discovered [-n<name>] [-a] <pubkey_prefix>

    `-n`: to give a name to the node
    `-a`: will add all found nodes (won't use the provided name though)
    <pubkey_prefix>: to identify the node to add if `-a` has not been used

"#;

const GET_HELP: &str = r"Gets parameters from node
  Please see also help for set command, which is more up to date ...
    name               : node name
    bat                : battery level in mV
    fstats             : fs statistics
    coords             : adv coordinates
    lat                : latitude
    lon                : longitude
    radio              : radio parameters
    tx                 : tx power
    private_key        : private key of the node
    print_snr          : snr display in messages
    print_adverts      : display adverts as they come
    print_new_contacts : display new pending contacts when available
    print_path_updates : display path updates as they come
    custom             : all custom variables in json format
                each custom var can also be get/set directly
    stats/status       : print status of the node
    stats_core         : core stats (bat/error/uptime/queue)
    stats_radio        : radio stats (noise/rssi/snr/tx_air/rx_air)
    stats_packets      : packets stats (recv/sent/flood/direct)
    allowed_repeat_freq: possible frequency ranges for repeater mode
    path_hash_mode     : size of path hashes (0: 1 byte, 1: 2 bytes, 2: 3 bytes)
    default_scope      : default flood scope of the node
";

const SET_HELP: &str = r#"Available parameters :
  device:
    pin <pin>                   : ble pin
    radio <freq,bw,sf,cr>       : radio params (add ",repeat" to enable repeat mode)
    tuning <rx_dly,af>          : tuning params
    tx <dbm>                    : tx power
    name <name>                 : node name
    lat <lat>                   : latitude
    lon <lon>                   : longitude
    private_key                 : private key
    coords <lat,lon>            : coordinates
    multi_acks <on/off>         : multi-acks feature
    telemetry_mode_base <mode>  : set basic telemetry mode all/selected/off
    telemetry_mode_loc <mode>   : set location telemetry mode all/selected/off
    telemetry_mode_env <mode>   : set env telemetry mode all/selected/off
    advert_loc_policy <policy>  : "share" means loc will be shared in adv
    manual_add_contacts <on/off>: let user manually add contacts to device
      - when off device automatically adds contacts from adverts
      - when on contacts must be added manually using add_pending
        (pending contacts list is built by meshcli from adverts while connected)
    autoadd_config              : set autoadd_config flags (see ?autoadd)
    path_hash_mode <value>      : size of path hashes (0: 1 byte, 1: 2 bytes, 2: 3 bytes)
    default_scope <scope>       : default flood scope of the node
  display:
    print_timestamp <on/off/fmt>: toggle printing of timestamp, can be strftime format
    print_snr <on/off>          : toggle snr display in messages
    print_adverts <on/off>      : display adverts as they come
    print_new_contacts <on/off> : display new pending contacts when available
    print_path_updates <on/off> : display path updates as they come
    json_log_rx <on/off>        : logs packets incoming to device as json
    json_msgs <on/off>          : display incoming messages as json
    display_channel_data <on/of>: display incoming channel data
    channel_echoes <on/off>     : print repeats for channel data
    advert_echoes <on/off>      : print repeats for adverts
    echo_unk_chans <on/off>     : also dump unk channels (encrypted)
    color <on/off>              : color off should remove ANSI codes from output
    lc_output_format <fmt>      : default output format of lc (see ?lc)
    lc_sort_order <order>       : default sort order of lc (see ?lc)
  meshcore-cli behaviour:
    classic_prompt <on/off>     : activates less fancier prompt
    arrow_head <string>         : change arrow head in prompt
    slash_start <string>        : idem for slash start
    slash_end <string>          : slash end
    invert_slash <on/off>       : apply color inversion to slash
    auto_update_contacts <on/of>: auto sync contact list with device
    max_attempts <n>            : attempts when sending a message in chat mode
    flood_after <n>             : attempt after which messages are sent flood
    max_flood_attempts <n>      : attempts in flood mode
"#;

const SCOPE_HELP: &str = r#"scope <scope> : changes flood scope of the node

    The scope command can be used from command line or interactive mode to set the region in which flood packets will be transmitted.

Managing Flood Scope in interactive mode
    Flood scope has recently been introduced in meshcore (from v1.10.0). It limits the scope of packets to regions, using transport codes in the frame.
    When entering chat mode, scope will be at default_scope.
    You can switch scope using the scope command, or postfixing the to command with %<scope>.
    Scope can also be applied to a command using % before the scope name. For instance login%#Morbihan will limit diffusion of the login command (which is usually sent flood to get the path to a repeater) to the #Morbihan region.

    Default_scope for the device can be set/get by using set default_scope and get default_scope, if set, the scope will revert to this default.

    When default_scope is active, there is no scope showed on the prompt/

    Setting scope to "0" goes back to default scope. Setting scope to "*" forces messages to be sent unscoped.
"#;

const CONTACT_INFO_HELP: &str = r"contact_info <ct> : displays contact info

    in interactive mode, there are some lighter commands that can be chained to give more compact information
        - contact_name (cn)
        - contact_key (ck)
        - contact_type (ct)
";

const CONTACTS_HELP: &str = r"Contact management

To receive a message from another user, it is necessary to have its public key. This key is stored on a contact list in the device, and this list has a finite size (50 when meshcore started, now over 350 for most devices).

By default contacts are automatically added to the device contact list when an advertisement is received, so as soon as you receive an advert, you can talk with your buddy.

With growing number of users, it becomes necessary to manage contact list and one of the ways is to add contacts manually to the device. This is done by turning on manual_add_contacts. Once this option has been turned on, a pending list is built by meshcore-cli from the received adverts. You can view the list issuing a pending_contacts command, flush the list using flush_pending or add a contact from the list with add_pending followed by the key of the contact or its name (both will be auto-completed with tab).

This feature only really works in interactive mode.

You can also set autoadd_config flag to filter contacts that are automatically added to your contact list (this will work when manual_add_contact is set to True).

// Auto-add config bitmask
// Bit 0: If set, overwrite oldest non-favourite contact when contacts file is full
// Bits 1-4: these indicate which contact types to auto-add when manual_contact_mode = 0x01
#define AUTO_ADD_OVERWRITE_OLDEST (1 << 0)  // 0x01 - overwrite oldest non-favourite when full
#define AUTO_ADD_CHAT             (1 << 1)  // 0x02 - auto-add Chat (Companion) (ADV_TYPE_CHAT)
#define AUTO_ADD_REPEATER         (1 << 2)  // 0x04 - auto-add Repeater (ADV_TYPE_REPEATER)
#define AUTO_ADD_ROOM_SERVER      (1 << 3)  // 0x08 - auto-add Room Server (ADV_TYPE_ROOM)
#define AUTO_ADD_SENSOR           (1 << 4)  // 0x10 - auto-add Sensor (ADV_TYPE_SENSOR)

Instead of an int you can use can make a string containing keys which can be (ov, cli, rep, room or sen), parser will look for keys in the string.

Contacts can also be added manually by using `add_contact` command, which takes key, type and name as parameters.

Note: There is also an auto_update_contacts setting that has nothing to do with adding contacts, it permits to automatically sync contact lists between device and meshcore-cli (when there is an update in name, location or path).
";

const CHANNEL_HELP: &str = r"Channel management

Channels are used to send messages to a group of people. This group of people share a common key, used to encrypt, identify and decrypt the messages that are sent flood over the network (possibly with a scope).

    Channel commands are the following:
        - get_channels
        - get_channel chan
        - add_channel name [key]
        - set_channel chan name [key]
        - remove_channel chan

There is a fixed number of slots on companions to store channel messages, each channel has a number, a name and a key, the get_channels command lists theses slots.

You can also call get_channel (with number or name) to get information about one channel.

Adding a channel can be done using the set_channel command, taking as parameters the channel number, the name and the key. Key is optional, if not provided, it will be computed from the name.
 The add_channel command won't take a number as it will use first available slot.

There is a special case for auto channels, which starts with a #, these have always their key computed from the name (note that mccli does not lowercase and strip characters so you should be careful when sharing when users of the android app or ripple).

To remove a channel, use remove_channel, either with channel name or number.
";

const TRACE_HELP: &str = r"Trace

Trace is a command used to get signal information (SNR) along a path.

Basic call to trace takes the path to follow as an argument, specifying each repeater along the path with its hash (separated or not with a comma).

Example:

Track-R|*> trace 6a61
 →13.25→[6a]→12.50→[61]→13.50→

At the beginning hashes were only 1 byte long. But with firmware after 1.12 you can use multi byte paths (2 bytes long and 4 bytes long hashes). The flag specifying the size of the hashes will either be guessed from the size of the tokens when used with commas, or specified using a colon (0: 1 byte, 1: 2 bytes, 3: 4 bytes), so AAAA,BBBB or AAAABBBB:1 are equivalent. When there is only one repeater on the path, you can put a comma at the end of the path to get the hash size right.

Here are some examples :

Track-R|*> trace 6a,61
 →13.25→[6a]→12.50→[61]→13.50→
Track-R|*> trace 6a61:0
 →13.25→[6a]→12.50→[61]→13.50→
Track-R|*> trace 6a83,6144
 →11.75→[6a83]→12.25→[6144]→13.00→
Track-R|*> trace 6a836144:1
 →12.00→[6a83]→12.00→[6144]→13.75→
Track-R|*> trace 6a83,
 →13.25→[6a83]→13.50→
Track-R|*> trace 6a83:1
 →12.75→[6a83]→12.50→
Track-R|*>

You can also send a trace with a node as parameter, it will (if path to that node is set) use the outgoing path for outgoing and incoming path. If destination is a repeater the trace will be done to the destination, or else to the last repeater of the path.

Track-R/SDQ_FdL_Rep|6a83> trace
 →12.25→[6a83]→12.00→[6144]→12.00→[6a83]→12.00→

In this case, the repeater had a path configured with 2 bytes hash, so it did a two bytes trace, going to the repeater and then coming back.

See also ?path

";

const PATH_HELP: &str = r"path management (reset_path, change_path)

In Meshcore, there are two ways for a packet to reach a destination, flood or path. Flood messages are send through the mesh and will be repeated once by each repeater along the way (building a path in the packet, so the destination knows where the packet came from). Path message have a path encoded in them, each repeater along the way will repeat the packet and remove its own hash from the path (once at the destination, path is empty).

The path for each contact is stored in the contact information, along with the path len and the path_hash_mode (specifying if its hash is 1, 2 or 3 bytes long. 0 for 1, 1 for 2 and 2 for 3).  A path len of 255 (or -1 if signed) means path is not set (flood).

meshcore-cli provides some functions to manage path :
 * path         : print path to a node
 * reset_path   : set path back to flood
 * change_path  : specify path to destination
 * contact_info : get all information for a contact
 * advert_path  : path taken by an advert
 * disc_path    : discover in and out path for a contact

When using change_path, you specify manually the path to the contact. Path is given as an hex string containing hashes for all repeaters in the way (you can use commas to separate hashes). By default hash_size will be the one of the node. If using commas, it will be guessed from first hash. You can also use a colon to specify path_hash_mode.

If you want to set the path for a node through 112233 445566 778899, you can use
 - 114477:0 or 11,44,77 for one byte hash
 - 112244557788:1 or 1122,4455,7788 for two byte hash
 - 112233445566778899:2 or 112233,445566,778899 for three byte hash

To set an empty path use 0.

To get the path for a contact, you can use three commands:
 - path will gives you the path stored in the node.
 - You can also get a path from a key using advert_path which will give you the path taken for last advert from that node to come.
 disc_path will send a path request and give you input and output path for a node.

Note that the path shown on the prompt only uses 1 byte notation without commas to keep it slim.

";

const ALIAS_HELP: &str = r#"alias mechanism (alias, aliases, aliases_load, @)

Meshcore-cli lets you create aliases to store a complex line for later use.

Aliases are created using the `alias` command and retrieved starting a line with `@` (@ stands for alias recall). `alias <key> ""` removes key from aliases, if only one parameter is given to the command, the content of the alias is displayed.

The alias command takes two parameters, alias name and value. Value can contain placeholders, {c} is replaced by contact name if currently in a contact on by first parameter, other parameters are stored into {1} {2} ...

You can list all aliases using `aliases` command and so using `.> <filename> aliases` will save the alias dict in a file (`alias` with no parameter will act as `aliases`).

To recall an alias dict from a file use `aliases_load`, with a name it will try to load aliases from a file by the given name (searching also in mccli config dir), with no name it will prompt you for a file (with a file completer).
"#;

const LC_HELP: &str = r"lc: displays contact list from the the node

lc supports these parameters:
  sort order    : -s
  output format : -f
  filter by type: -t
  filter by flag: -b
  quiet         : -q (only refreshes the contact list)
  list only     : -l (no total line)

Sort order is given as a string with different criteria (lowercase for ascending, uppercase for descending order):
    n: sorts by name
    t: sorts by type
    a: sorts by last advert
    m: sorts by modification time
    h: sorts by path len
    d: sorts by distance
    f: starts with favorites
If several criteria are used, they are evaluated from right to left, so the leftmost one has more priority `-stfn` first sorts by name, then puts the favorites first and finally orders by type.
Use get/set lc_sort_order to get or set default value

Output format is given as a string with one character by field. Name is always displayed first but you can tailor the rest of the output:
    t: type
    k: key prefix
    a: last advert
    m: last modification
    h: hop count
    d: distance (in km)
    b: flag bits (as hex)
Use get/set lc_output_format to get or set default value

Type filter is given as a string of types (1 or c: client, 2 or r: repeater, 3 or o: room, 4 or s: sensor).
";

/// Help of the repeater mode (`-r`).
#[must_use]
pub fn repeater_help() -> String {
    let (c, g, y, e) = (ansi::BCYAN, ansi::BGREEN, ansi::BYELLOW, ansi::END);
    format!(
        r"
{c}Repeater CLI Commands:{e}

{g}Info:{e}
  ver                 - Firmware version
  board               - Board name
  clock               - Show current time
  clock sync          - Sync the clock with this computer (also st or sync_time)

{g}Stats:{e}
  stats-core          - Core stats (uptime, battery, queue)
  stats-radio         - Radio stats (RSSI, SNR, noise floor)
  stats-packets       - Packet statistics (sent/recv counts)
  clear stats         - Reset all statistics

{g}Network:{e}
  neighbors           - Show neighboring repeaters (zero-hop)
  advert              - Send advertisement now

{g}Logging:{e}
  log start           - Enable packet logging
  log stop            - Disable packet logging
  log                 - Dump log file to console
  log erase           - Erase log file

{g}Configuration (get/set):{e}
  get name            - Node name
  get radio           - Radio params (freq,bw,sf,cr)
  get tx              - TX power (dBm)
  get repeat          - Repeat mode on/off
  get public.key      - Node public key
  get advert.interval - Advertisement interval (minutes)
  get owner.info      - Owner information

  set name <name>     - Set node name
  set tx <power>      - Set TX power (dBm)
  set repeat on|off   - Enable/disable repeating
  set radio f,bw,sf,cr - Set radio params (reboot to apply)
  set advert.interval <min> - Set advert interval (60-240 min)
  set owner.info <i>  - Set owner information

{g}Region management:{e}
  region             - display currently configured regions
  region save        - save current region config to flash
  region download    - download regions config from node to file
  region (up)load    - upload regions config to node from file
  region home        - get/set home region
  region get         - get info (and parent) for a region
  region put         - adds or update a region
  region remove      - remove a region definition
  region allowf      - gives flood permission to a region
  region denyf       - remove flood permission to a region
  region list        - list allowed/denied regions

{g}System:{e}
  reboot              - Reboot device
  erase               - Erase filesystem (serial only)
  script <file>       - Execute script

{y}Type 'quit' or 'q' to exit, Ctrl+C to abort{e}
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_dispatch() {
        assert!(help_for("at").starts_with("apply_to"));
        assert!(help_for("add_channel").starts_with("Channel management"));
        assert!(help_for("change_path").starts_with("path management"));
        assert!(help_for("aliases_load").starts_with("alias mechanism"));
        assert!(help_for("tr").starts_with("Trace"));
        assert_eq!(help_for("nothing"), "Sorry, no help yet for nothing\n");
    }

    #[test]
    fn usage_contains_program_and_commands() {
        let u = usage();
        assert!(u.contains("Usage : meshcore-cli-rs <args> <commands>"));
        assert!(u.contains("send_raw <pkt>"));
    }
}
