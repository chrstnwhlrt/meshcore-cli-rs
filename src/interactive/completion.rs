//! Completion candidates of the chat mode (`make_completion_dict`).

use crate::app::{App, Target, contact_type, vstr};
use crate::editor::CompletionTree;

fn leaves(keys: &[&str]) -> CompletionTree {
    CompletionTree::leaves(keys.iter().copied())
}

fn on_off() -> CompletionTree {
    leaves(&["on", "off"])
}

fn add_leaves(tree: &mut CompletionTree, keys: &[&str]) {
    for key in keys {
        tree.leaf(*key);
    }
}

/// Parameters of `set` at the root.
fn root_set() -> CompletionTree {
    let mut set = CompletionTree::new();
    add_leaves(&mut set, &["name", "pin"]);
    set.node("radio", leaves(&[",,,", "f,bw,sf,cr"]));
    add_leaves(&mut set, &["tx", "freq"]);
    set.node("tuning", leaves(&[",", "af,tx_d"]));
    add_leaves(&mut set, &["lat", "lon", "coords", "private_key"]);
    set.node("print_snr", on_off());
    set.node("print_timestamp", leaves(&["on", "off", "%Y:%M"]));
    set.node("json_msgs", on_off());
    set.node("color", on_off());
    add_leaves(&mut set, &["lc_output_format", "lc_sort_order"]);
    for key in [
        "print_adverts",
        "display_channel_data",
        "json_log_rx",
        "channel_echoes",
        "advert_echoes",
        "echo_unk_chans",
        "print_new_contacts",
        "print_path_updates",
        "classic_prompt",
        "manual_add_contacts",
    ] {
        set.node(key, on_off());
    }
    set.leaf("autoadd_config");
    for key in [
        "telemetry_mode_base",
        "telemetry_mode_loc",
        "telemetry_mode_env",
    ] {
        set.node(key, leaves(&["always", "device", "never"]));
    }
    set.node("advert_loc_policy", leaves(&["none", "share"]));
    set.node("auto_update_contacts", on_off());
    set.node("multi_acks", on_off());
    add_leaves(
        &mut set,
        &["max_attempts", "max_flood_attempts", "flood_after"],
    );
    set.node("path_hash_mode", leaves(&["0", "1", "2"]));
    set.node("path.hash.mode", leaves(&["0", "1", "2"]));
    add_leaves(
        &mut set,
        &[
            "default_scope",
            "tz.offset",
            "radio.rxgain",
            "radio.fem.rxgain",
            "radio.fem.txgain",
            "dutycycle",
            "int.thresh",
            "cad",
            "agc.reset.interval",
            "multi.acks",
            "wifi.status",
            "wifi.ssid",
            "wifi.pwd",
            "ui.show_clock",
            "ui.auto_off_delay",
            "ui.backlight_delay",
            "ui.backlight_on_keys",
            "ui.backlight_on_msgs",
            "ui.auto_off_blank",
            "ui.max_partial",
            "ui.nd_name",
        ],
    );
    set.node("ui.gps_action", leaves(&["toggle", "once"]));
    add_leaves(&mut set, &["arrow_head", "slash_start", "slash_end"]);
    set.node("invert_slash", on_off());
    set
}

/// Parameters of `get` at the root.
fn root_get() -> CompletionTree {
    leaves(&[
        "name",
        "bat",
        "fstats",
        "radio",
        "tx",
        "coords",
        "lat",
        "lon",
        "private_key",
        "print_snr",
        "print_timestamp",
        "json_msgs",
        "color",
        "display_channel_data",
        "lc_output_format",
        "lc_sort_order",
        "print_adverts",
        "json_log_rx",
        "channel_echoes",
        "advert_echoes",
        "echo_unk_chans",
        "print_path_updates",
        "print_new_contacts",
        "classic_prompt",
        "manual_add_contacts",
        "autoadd_config",
        "telemetry_mode_base",
        "telemetry_mode_loc",
        "telemetry_mode_env",
        "advert_loc_policy",
        "auto_update_contacts",
        "multi_acks",
        "max_attempts",
        "max_flood_attempts",
        "flood_after",
        "custom",
        "stats",
        "status",
        "stats_core",
        "stats_radio",
        "stats_packets",
        "allowed_repeat_freq",
        "path_hash_mode",
        "path.hash.mode",
        "repeat",
        "default_scope",
        "tz.offset",
        "radio.rxgain",
        "radio.fem.rxgain",
        "radio.fem.txgain",
        "dutycycle",
        "int.thresh",
        "cad",
        "agc.reset.interval",
        "multi.acks",
        "pin",
        "wifi.status",
        "wifi.ssid",
        "wifi.pwd",
        "ui.show_clock",
        "ui.auto_off_delay",
        "ui.backlight_delay",
        "ui.backlight_on_keys",
        "ui.backlight_on_msgs",
        "ui.auto_off_blank",
        "ui.max_partial",
    ])
}

/// Commands at the root.
fn root_commands(
    contacts: &CompletionTree,
    pending: &CompletionTree,
    discovered: &CompletionTree,
) -> CompletionTree {
    let mut root = CompletionTree::new();
    add_leaves(
        &mut root,
        &[
            "public",
            "chan",
            "chan_data",
            "ver",
            "infos",
            "advert",
            "floodadv",
            "cli",
        ],
    );
    root.node("msg", contacts.clone());
    add_leaves(&mut root, &["wait_ack", "time"]);
    root.node("clock", leaves(&["sync"]));
    add_leaves(
        &mut root,
        &[
            "reboot",
            "card",
            "upload_card",
            "contacts",
            "lc",
            "pending_contacts",
        ],
    );
    root.node("add_pending", pending.clone());
    root.leaf("flush_pending");
    for key in [
        "contact_info",
        "export_contact",
        "upload_contact",
        "share_contact",
        "path",
        "disc_path",
    ] {
        root.node(key, contacts.clone());
    }
    let mut advert_path = contacts.clone();
    advert_path.update(pending);
    root.node("advert_path", advert_path);
    root.node(
        "node_discover",
        leaves(&[
            "-tall", "-tsens", "-trep", "-tcomp", "-troom", "-tcli", "-f",
        ]),
    );
    root.leaf("list_discovered");
    root.node("add_discovered", discovered.clone());
    root.leaf("trace");
    for key in [
        "reset_path",
        "change_path",
        "change_flags",
        "remove_contact",
    ] {
        root.node(key, contacts.clone());
    }
    root.node("add_contact", leaves(&["key type name"]));
    root.node("reset_advert_time", contacts.clone());
    root.node("import_contact", leaves(&["meshcore://"]));
    add_leaves(
        &mut root,
        &["reload_contacts", "rc", "aliases", "aliases_load", "alias"],
    );
    root.node("handler_attach", leaves(&["rxlog", "msgs", "chan_data"]));
    add_leaves(&mut root, &["handler_detach", "handlers"]);
    for key in [
        "login",
        "cmd",
        "req_status",
        "req_neighbours",
        "logout",
        "req_telemetry",
        "req_binary",
        "req_mma",
        "req_owner",
        "req_regions",
        "req_clock",
        "req_acl",
        "contact_timeout",
    ] {
        root.node(key, contacts.clone());
    }
    add_leaves(
        &mut root,
        &[
            "self_telemetry",
            "get_channel",
            "set_channel",
            "add_channel",
            "get_channels",
            "remove_channel",
            "apply_to",
            "at",
            "scope",
            "send_raw",
            "sync_msgs",
            "recv",
            "wait_msg",
            "trywait_msg",
            "msgs_subscribe",
            "sleep",
            "wait_key",
            "echo",
            "help",
        ],
    );
    root.node(
        "gps",
        leaves(&[
            "on",
            "off",
            "sync",
            "once",
            "diag",
            "fix",
            "fix_stats",
            "period",
        ]),
    );
    root.node("set", root_set());
    root.node("get", root_get());
    for topic in [
        "?get",
        "?set",
        "?scope",
        "?contact_info",
        "?apply_to",
        "?at",
        "?lc",
        "?node_discover",
        "?nd",
        "?pending_contacts",
        "?add_pending",
        "?flush_pending",
        "?get_channels",
        "?set_channel",
        "?get_channel",
        "?add_channel",
        "?remove_channel",
        "?path",
        "?change_path",
        "?trace",
        "?alias",
        "?add_contact",
        "?autoadd",
    ] {
        root.leaf(topic);
    }
    root
}

/// Commands available inside any contact.
fn contact_commands() -> CompletionTree {
    leaves(&[
        "contact_info",
        "contact_name",
        "contact_key",
        "contact_type",
        "contact_lastmod",
        "export_contact",
        "share_contact",
        "upload_contact",
        "path",
        "disc_path",
        "advert_path",
        "trace",
        "dtrace",
        "reset_path",
        "change_path",
        "change_flags",
        "reset_advert_time",
        "req_telemetry",
        "req_binary",
        "forget_password",
    ])
}

/// Commands inside a client.
fn client_commands() -> CompletionTree {
    let mut tree = contact_commands();
    tree.node("get", leaves(&["timeout"]));
    tree.node("set", leaves(&["timeout"]));
    tree
}

/// Commands inside a repeater or room (the remote CLI).
fn repeater_commands(contacts: &CompletionTree) -> CompletionTree {
    let mut tree = contact_commands();
    add_leaves(
        &mut tree,
        &[
            "login",
            "logout",
            "req_status",
            "req_neighbours",
            "cmd",
            "ver",
            "advert",
            "time",
        ],
    );
    tree.node("clock", leaves(&["sync"]));
    add_leaves(
        &mut tree,
        &["reboot", "clkreboot", "start ota", "password", "neighbors"],
    );
    tree.node("neighbor.remove", contacts.clone());
    add_leaves(
        &mut tree,
        &[
            "discover.neighbors",
            "req_acl",
            "req_owner",
            "req_regions",
            "req_clock",
        ],
    );
    tree.node("setperm", contacts.clone());
    tree.node(
        "region",
        leaves(&[
            "get", "allowf", "denyf", "put", "remove", "save", "home", "list",
        ]),
    );
    let mut gps = leaves(&[
        "on",
        "off",
        "sync",
        "once",
        "diag",
        "fix",
        "fix_stats",
        "period",
        "setloc",
    ]);
    gps.node("advert", leaves(&["none", "share", "prefs"]));
    tree.node("gps", gps);
    tree.node("powersaving", on_off());
    let mut sensor = leaves(&["list"]);
    sensor.node("set", leaves(&["gps"]));
    sensor.node("get", leaves(&["gps"]));
    tree.node("sensor", sensor);
    tree.node(
        "get",
        leaves(&[
            "name",
            "role",
            "radio",
            "freq",
            "tx",
            "af",
            "repeat",
            "allow.read.only",
            "flood.advert.interval",
            "flood.max",
            "advert.interval",
            "advert.loc",
            "guest.password",
            "owner.info",
            "rxdelay",
            "txdelay",
            "direct.tx_delay",
            "public.key",
            "lat",
            "lon",
            "telemetry",
            "status",
            "timeout",
            "acl",
            "bridge.enabled",
            "bridge.delay",
            "bridge.source",
            "bridge.baud",
            "bridge.secret",
            "bridge.type",
            "path.hash.mode",
        ]),
    );
    let mut set = leaves(&["name"]);
    set.node("radio", leaves(&[",,,", "f,bw,sf,cr"]));
    add_leaves(&mut set, &["freq", "tx", "af"]);
    set.node("repeat", on_off());
    add_leaves(
        &mut set,
        &["flood.advert.interval", "flood.max", "advert.interval"],
    );
    set.node("advert.loc", leaves(&["none", "shared", "prefs"]));
    add_leaves(&mut set, &["guest.password", "owner.info"]);
    set.node("allow.read.only", on_off());
    add_leaves(
        &mut set,
        &[
            "rxdelay",
            "txdelay",
            "direct.txdelay",
            "lat",
            "lon",
            "timeout",
        ],
    );
    set.node("perm", contacts.clone());
    set.node("bridge.enabled", on_off());
    add_leaves(
        &mut set,
        &[
            "bridge.delay",
            "bridge.source",
            "bridge.baud",
            "bridge.secret",
        ],
    );
    set.node("path.hash.mode", leaves(&["0", "1", "2"]));
    tree.node("set", set);
    tree.leaf("erase");
    tree.node("log", leaves(&["start", "stop", "erase"]));
    tree
}

/// Commands inside a sensor.
fn sensor_commands(contacts: &CompletionTree) -> CompletionTree {
    let mut tree = repeater_commands(contacts);
    tree.node("req_mma", leaves(&["begin end"]));
    if let Some(get) = tree.child_mut("get") {
        get.leaf("mma");
    }
    tree
}

/// Builds all candidates for the current destination.
pub fn make_completion_dict(app: &App, to: Option<&Target>) -> CompletionTree {
    let contacts_vec = app.mc.contacts();
    let contacts = CompletionTree::leaves(contacts_vec.iter().map(|c| c.name.clone()));
    let pending_vec = app.mc.pending_contacts();
    let mut pending = CompletionTree::leaves(pending_vec.iter().map(|c| c.name.clone()));
    for p in &pending_vec {
        pending.leaf(p.public_key.to_string());
    }
    let (discovered_nodes, aliases, custom_vars, last_node, channels) = {
        let state = app.state();
        (
            state.discovered.clone(),
            state.aliases.clone(),
            state.custom_vars.clone(),
            state.last_node.is_some(),
            state.channels.clone().unwrap_or_default(),
        )
    };
    let mut discovered = leaves(&["-a"]);
    for node in &discovered_nodes {
        discovered.leaf(vstr(node, "pubkey").chars().take(16).collect::<String>());
    }

    let mut to_list = leaves(&["~", "/"]);
    if last_node {
        to_list.leaf("!");
    }
    to_list.leaf("..");
    to_list.update(&contacts);
    to_list.leaf("ch");
    to_list.leaf("ch0");
    to_list.leaf("public");
    for channel in channels.iter().filter(|c| !c.name.is_empty()) {
        to_list.leaf(channel.name.clone());
    }

    let mut completion = CompletionTree::new();
    completion.node("to", to_list.clone());
    completion.node("/to", to_list);

    let root = root_commands(&contacts, &pending, &discovered);
    match to {
        None => {
            completion.update(&root);
            for name in &custom_vars {
                if let Some(set) = completion.child_mut("set") {
                    set.leaf(name.clone());
                }
                if let Some(get) = completion.child_mut("get") {
                    get.leaf(name.clone());
                }
            }
            completion.node(
                "alias",
                CompletionTree::leaves(aliases.iter().map(|(k, _)| k.clone())),
            );
        }
        Some(target) => {
            completion.leaf("send");
            match target.kind() {
                1 => completion.update(&client_commands()),
                2 | 3 => completion.update(&repeater_commands(&contacts)),
                4 => completion.update(&sensor_commands(&contacts)),
                _ => {}
            }
        }
    }

    for (key, _) in &aliases {
        completion.leaf(format!("@{key}"));
    }
    for key in root.keys() {
        let child = root.child(key).cloned();
        completion.insert(format!("/{key}"), child);
    }
    for contact in &contacts_vec {
        let commands = match contact_type(contact) {
            1 => client_commands(),
            2 | 3 => repeater_commands(&contacts),
            4 => sensor_commands(&contacts),
            _ => continue,
        };
        for key in commands.keys() {
            completion.insert(
                format!("/{}/{key}", contact.name),
                commands.child(key).cloned(),
            );
        }
    }
    for channel in channels.iter().filter(|c| !c.name.is_empty()) {
        completion.leaf(format!("/{}", channel.name));
    }
    completion.leaf("script");
    completion.leaf("quit");
    completion
}
