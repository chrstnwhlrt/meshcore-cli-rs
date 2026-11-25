//! Contact commands.

use std::cmp::Ordering;
use std::fmt::Write as _;

use meshcore::{Contact, ContactType, PublicKey};
use serde_json::{Value, json};

use super::{CmdError, CmdResult, Ctx};
use crate::app::{to_value, vf64, vint, vstr};
use crate::display::type_name;
use crate::util::ansi;
use crate::util::text::{pad_left, pad_right};
use crate::util::time::{haversine_km, now_secs, time_ago};

/// Formats a path: comma separated hashes of `hash_mode + 1` bytes.
///
/// `path` is the hex string of the path bytes, `hops` the number of hops.
#[must_use]
pub fn format_path(path: &str, hops: i64, hash_mode: i64) -> String {
    let size = usize::try_from(hash_mode + 1).unwrap_or(1) * 2;
    let hops = usize::try_from(hops).unwrap_or(0);
    let chunks: Vec<&str> = (0..hops)
        .filter_map(|i| path.get(i * size..((i + 1) * size).min(path.len())))
        .filter(|s| !s.is_empty())
        .collect();
    chunks.join(",")
}

/// Path of a contact payload as displayed by `lc` and `path`.
#[must_use]
pub fn contact_path_str(contact: &Value) -> String {
    match vint(contact, "out_path_len") {
        -1 => "Flood".to_string(),
        0 => "0 hop".to_string(),
        hops => format_path(
            &vstr(contact, "out_path"),
            hops,
            vint(contact, "out_path_hash_mode"),
        ),
    }
}

/// Sorting criteria of `lc` (`SORTING_CRITERIA`).
fn compare(criterion: char, a: &Value, b: &Value) -> Option<Ordering> {
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let fav = |v: &Value| vint(v, "flags") & 1 == 1;
    let ord = match criterion.to_ascii_lowercase() {
        'a' => num(a, "last_advert").partial_cmp(&num(b, "last_advert"))?,
        'n' => vstr(a, "adv_name").cmp(&vstr(b, "adv_name")),
        't' => num(a, "type").partial_cmp(&num(b, "type"))?,
        'm' => num(a, "lastmod").partial_cmp(&num(b, "lastmod"))?,
        'h' => num(a, "out_path_len").partial_cmp(&num(b, "out_path_len"))?,
        'd' => num(a, "distance").partial_cmp(&num(b, "distance"))?,
        'f' => fav(a).cmp(&fav(b)),
        _ => return None,
    };
    // Upper case sorts descending; for 'f' the lower case puts favourites first.
    let descending = if criterion.eq_ignore_ascii_case(&'f') {
        criterion == 'f'
    } else {
        criterion.is_ascii_uppercase()
    };
    Some(if descending { ord.reverse() } else { ord })
}

/// Applies a sort order string; the leftmost criterion has priority.
pub fn sort_contacts(contacts: &mut [Value], order: &str) {
    for criterion in order.chars().rev() {
        if compare(criterion, &Value::Null, &Value::Null).is_none() {
            continue;
        }
        contacts.sort_by(|a, b| compare(criterion, a, b).unwrap_or(Ordering::Equal));
    }
}

/// Type letters accepted by `lc -t` (`TYPE_MAP`).
fn type_from_letter(letter: char) -> Option<i64> {
    match letter {
        '1' | 'c' => Some(1),
        '2' | 'r' => Some(2),
        '3' | 'o' => Some(3),
        '4' | 's' => Some(4),
        _ => None,
    }
}

/// Distance from the node to a contact.
fn distance(self_info: &Value, contact: &Value) -> f64 {
    haversine_km(
        vf64(self_info, "adv_lat"),
        vf64(self_info, "adv_lon"),
        vf64(contact, "adv_lat"),
        vf64(contact, "adv_lon"),
    )
}

/// `contacts` / `list` / `lc` and `reload_contacts` / `rc`.
pub async fn list(c: &mut Ctx<'_>, reload: bool) -> CmdResult {
    let app = c.app.clone();
    if reload {
        let _ = app.mc.get_contacts(0).await;
    } else {
        let _ = app.mc.ensure_contacts(true).await;
    }

    let (mut sort_order, mut output_format) = {
        let s = app.settings();
        (s.lc_sort_order.clone(), s.lc_output_format.clone())
    };
    let mut display_types: Vec<i64> = vec![1, 2, 3, 4];
    let mut filter_flag = 0i64;
    let mut quiet = false;
    let mut display_total = true;
    let mut argnum = 0;
    let mut args = &c.cmds[1..];
    while let Some(arg) = args.first() {
        if arg.is_empty() {
            return Err(CmdError::Params);
        }
        let Some(option) = arg.strip_prefix('-') else {
            break;
        };
        argnum += 1;
        args = &args[1..];
        let mut chars = option.chars();
        match chars.next() {
            Some('s') => sort_order = chars.as_str().to_string(),
            Some('f') => output_format = chars.as_str().to_string(),
            Some('t') => display_types = chars.filter_map(type_from_letter).collect(),
            Some('b') => {
                filter_flag = crate::util::py::int(chars.as_str(), 0).map_err(CmdError::Failed)?;
            }
            Some('q') => quiet = true,
            Some('l') => display_total = false,
            _ => {}
        }
    }
    if quiet {
        return Ok(argnum);
    }

    let mut all: Vec<Value> = app
        .mc
        .contacts()
        .iter()
        .map(|contact| app.contact_value(contact))
        .collect();
    // the reference adds the distance to its contact dicts, so it also shows
    // up in the json of all contacts
    if output_format.to_lowercase().contains('d') || sort_order.to_lowercase().contains('d') {
        let self_info = app.self_info_value();
        for contact in &mut all {
            let d = distance(&self_info, contact);
            if let Value::Object(map) = contact {
                map.insert("distance".into(), json!(d));
            }
        }
    }
    let mut shown = all.clone();
    if filter_flag > 0 {
        shown.retain(|x| vint(x, "flags") & filter_flag != 0);
    }
    // always applied: -t with no known letter shows nothing, as in the
    // reference (where a set is never equal to [])
    shown.retain(|x| display_types.contains(&vint(x, "type")));
    sort_contacts(&mut shown, &sort_order);

    if c.json {
        if argnum == 0 {
            let mut map = serde_json::Map::new();
            for contact in &all {
                map.insert(vstr(contact, "public_key"), contact.clone());
            }
            c.json_indent(&Value::Object(map));
        } else {
            c.json_indent(&Value::Array(shown));
        }
        return Ok(argnum);
    }

    let now = now_secs();
    let mut text = String::new();
    for contact in &shown {
        text.push_str(&pad_right(&vstr(contact, "adv_name"), 30));
        text.push(' ');
        text.push_str(&ansi::column(34));
        for field in output_format.to_lowercase().chars() {
            match field {
                't' => {
                    text.push(' ');
                    text.push_str(&pad_right(&type_name(vint(contact, "type")), 4));
                }
                'k' => {
                    text.push(' ');
                    text.push_str(
                        &vstr(contact, "public_key")
                            .chars()
                            .take(12)
                            .collect::<String>(),
                    );
                }
                'h' => {
                    text.push(' ');
                    text.push_str(&pad_right(&contact_path_str(contact), 12));
                }
                'a' => {
                    text.push(' ');
                    text.push_str(&pad_left(&time_ago(vint(contact, "last_advert"), now), 4));
                }
                'm' => {
                    text.push(' ');
                    text.push_str(&pad_left(&time_ago(vint(contact, "lastmod"), now), 4));
                }
                'b' => {
                    let _ = write!(text, " {:02x}", vint(contact, "flags"));
                }
                'd' => {
                    let _ = write!(text, " {:7.2}km", vf64(contact, "distance"));
                }
                _ => {}
            }
        }
        text.push('\n');
    }
    c.emit(&text);
    if display_total {
        c.line(&format!(
            "> {} from {} contacts in device",
            shown.len(),
            all.len()
        ));
    }
    Ok(argnum)
}

/// `pending_contacts`.
pub fn pending_contacts(c: &mut Ctx<'_>) -> usize {
    let pending = c.app.mc.pending_contacts();
    if c.json {
        let mut map = serde_json::Map::new();
        for contact in &pending {
            let value = to_value(contact);
            map.insert(vstr(&value, "public_key"), value);
        }
        c.json_indent(&Value::Object(map));
    } else {
        let lines: Vec<String> = pending
            .iter()
            .map(|p| format!("{}: {}", p.name, p.public_key))
            .collect();
        c.line(&lines.join("\n"));
    }
    0
}

/// `add_pending <key|name>`: adds a pending contact to the device.
pub async fn add_pending(c: &mut Ctx<'_>) -> CmdResult {
    let arg = c.arg(1)?.to_string();
    let mc = &c.app.mc;
    // meshcore_py looks the key up as it is written: lower case hex only
    let mut contact = PublicKey::from_hex(&arg)
        .ok()
        .filter(|key| key.to_string() == arg)
        .and_then(|key| mc.pop_pending_contact(&key));
    if contact.is_none()
        && let Some(found) = mc.pending_contacts().into_iter().find(|p| p.name == arg)
    {
        contact = mc.pop_pending_contact(&found.public_key);
    }
    match contact {
        None => {
            if c.json {
                c.json_line(&json!({"error": "Contact does not exist"}));
            } else {
                log::error!("Contact {arg} does not exist");
            }
        }
        Some(contact) => match mc.add_contact(&contact).await {
            Err(e) => c.line(&format!("Error adding contact: {e}")),
            Ok(()) => {
                if c.json {
                    c.json_indent(&json!({}));
                }
            }
        },
    }
    Ok(1)
}

/// `path <ct>`: the stored path of a contact.
pub async fn path(c: &mut Ctx<'_>) -> CmdResult {
    let name = c.arg(1)?.to_string();
    let Some(contact) = c.app.contact_from_arg(&name).await else {
        c.unknown_contact(&name);
        return Ok(1);
    };
    let v = to_value(&contact);
    if c.json {
        // the reference names the hash size out_path_hash_len (bytes per
        // hop, as in path discovery results) but reads a key contacts do
        // not have; this is the value it means
        c.json_line(&json!({
            "adv_name": v["adv_name"],
            "out_path_hash_len": vint(&v, "out_path_hash_mode") + 1,
            "out_path_len": v["out_path_len"],
            "out_path": v["out_path"],
        }));
    } else {
        c.line(&contact_path_str(&v));
    }
    Ok(1)
}

/// `contact_info <ct>`: all fields of a contact as json.
pub async fn contact_info(c: &mut Ctx<'_>) -> CmdResult {
    let name = c.arg(1)?.to_string();
    let _ = c.app.mc.ensure_contacts(true).await;
    match c.app.contact_from_arg(&name).await {
        None => c.unknown_contact(&name),
        Some(contact) => {
            let value = c.app.contact_value(&contact);
            c.json_indent(&value);
        }
    }
    Ok(1)
}

/// `add_contact <key> <type> <name>`.
pub async fn add_contact(c: &mut Ctx<'_>) -> CmdResult {
    let key = c.arg(1)?.to_string();
    // the reference builds its dict in this order: int(type) fails before a
    // missing name is noticed; meshcore_py then reads the key (a bad one is
    // the format error) before it packs the type into a byte
    let kind = crate::util::py::int(c.arg(2)?, 10).map_err(CmdError::Failed)?;
    let name = c.arg(3)?.to_string();
    // bytes.fromhex in meshcore_py, which also takes spaces between pairs
    let Some(public_key) = crate::util::py::fromhex(&key)
        .ok()
        .and_then(|bytes| PublicKey::from_slice(&bytes))
    else {
        c.line("Error ! Command format add_contact key type name");
        return Ok(3);
    };
    let kind = u8::try_from(kind).map_err(|_| {
        CmdError::Failed(if kind < 0 {
            "can't convert negative int to unsigned".to_string()
        } else {
            "int too big to convert".to_string()
        })
    })?;
    let contact = new_contact(public_key, kind, &name);
    match c.app.mc.add_contact(&contact).await {
        Err(e) => c.line(&format!("Error adding contact: {e}")),
        Ok(()) => {
            if c.json {
                c.json_indent(&json!({}));
            }
        }
    }
    let _ = c.app.mc.get_contacts(0).await;
    Ok(3)
}

/// `reset_advert_time <ct>`: sets the last advert time of a contact to 0.
pub async fn reset_advert_time(c: &mut Ctx<'_>) -> CmdResult {
    let name = c.arg(1)?.to_string();
    let Some(mut contact) = c.app.contact_from_arg(&name).await else {
        c.unknown_contact(&name);
        return Ok(1);
    };
    contact.last_advert = 0;
    match c.app.mc.update_contact(&contact, None).await {
        Err(_) => c.line(&format!("Error while resetting last advert for: {name}")),
        Ok(()) => {
            if c.json {
                c.json_indent(&json!({}));
            }
        }
    }
    Ok(1)
}

/// Parses a path argument of `change_path` into bytes and hash mode.
///
/// `0` is the empty path, `aabb:1` gives the mode explicitly (`int()` of
/// the text after the first colon, as `meshcore_py` reads it), with commas
/// (`aa,bb`) the mode follows from the size of the first hash.
///
/// # Errors
///
/// Returns an error for malformed hex or modes; a mode below 0 (a first
/// hash of one digit gives -1) fails in the reference with a Python error.
pub fn parse_path_arg(arg: &str) -> Result<(Vec<u8>, Option<u8>), String> {
    if arg == "0" {
        return Ok((Vec::new(), None));
    }
    let mut fields = arg.split(':');
    let hops = fields.next().unwrap_or_default();
    let mode = match fields.next() {
        Some(mode) => Some(crate::util::py::int(mode, 10)?),
        None if arg.contains(',') => {
            let first = hops.split(',').next().unwrap_or_default();
            Some(i64::try_from(first.chars().count() / 2).unwrap_or(i64::MAX) - 1)
        }
        None => None,
    };
    let mode = mode
        .map(|mode| u8::try_from(mode).map_err(|_| format!("hash mode {mode}")))
        .transpose()?;
    let bytes = crate::util::py::fromhex(&hops.replace(',', ""))?;
    Ok((bytes, mode))
}

/// `change_path <ct> <path>`.
pub async fn change_path(c: &mut Ctx<'_>) -> CmdResult {
    let name = c.arg(1)?.to_string();
    let Some(contact) = c.app.contact_from_arg(&name).await else {
        c.unknown_contact(&name);
        return Ok(2);
    };
    // read once the contact is known, as in the reference
    let path_arg = c.arg(2)?.to_string();
    match parse_path_arg(&path_arg) {
        Err(_) => c.line(&format!("Bad path format {path_arg}")),
        Ok((path, mode)) => match c.app.mc.change_contact_path(&contact, &path, mode).await {
            Err(e) => c.line(&format!("Error setting path: {e}")),
            Ok(_) => {
                if c.json {
                    c.json_indent(&json!({}));
                }
            }
        },
    }
    Ok(2)
}

/// `change_flags <ct> <flags>`.
pub async fn change_flags(c: &mut Ctx<'_>) -> CmdResult {
    let name = c.arg(1)?.to_string();
    let Some(contact) = c.app.contact_from_arg(&name).await else {
        c.unknown_contact(&name);
        return Ok(2);
    };
    // read once the contact is known, as in the reference
    let flags = crate::util::py::int(c.arg(2)?, 0).map_err(CmdError::Failed)?;
    // int.to_bytes(1) in meshcore_py
    let flags = u8::try_from(flags).map_err(|_| {
        CmdError::Failed(if flags < 0 {
            "can't convert negative int to unsigned".to_string()
        } else {
            "int too big to convert".to_string()
        })
    })?;
    match c.app.mc.change_contact_flags(&contact, flags).await {
        Err(e) => c.line(&format!("Error setting flags: {e}")),
        Ok(_) => {
            if c.json {
                c.json_indent(&json!({}));
            }
        }
    }
    Ok(2)
}

/// `reset_path <ct>`: back to flood.
pub async fn reset_path(c: &mut Ctx<'_>) -> CmdResult {
    let name = c.arg(1)?.to_string();
    let Some(contact) = c.app.contact_from_arg(&name).await else {
        c.unknown_contact(&name);
        return Ok(1);
    };
    match c.app.mc.reset_path(&contact.public_key).await {
        Err(e) => c.line(&format!("Error resetting path: {e}")),
        Ok(()) => {
            if c.json {
                c.json_indent(&json!({}));
            }
        }
    }
    Ok(1)
}

/// `advert_path <key|name>`: path of the last advert from a node.
pub async fn advert_path(c: &mut Ctx<'_>) -> CmdResult {
    let arg = c.arg(1)?.to_string();
    let mut key = c.app.contact_from_arg(&arg).await.map(|ct| ct.public_key);
    if key.is_none() {
        // the reference keeps the last pending contact that matches
        key = c
            .app
            .mc
            .pending_contacts()
            .into_iter()
            .rfind(|p| p.name == arg || p.public_key.to_string().starts_with(&arg))
            .map(|p| p.public_key);
    }
    let key = match key {
        Some(key) => key,
        // the text as key: meshcore_py takes the first 32 bytes of a hex
        // string of at least 64 characters and refuses anything else
        None => (arg.chars().count() >= 64)
            .then(|| crate::util::py::fromhex(&arg).ok())
            .flatten()
            .and_then(|bytes| bytes.get(..32).and_then(PublicKey::from_slice))
            .ok_or_else(|| CmdError::Failed(format!("Invalid public key hex string: {arg}")))?,
    };
    match c.app.mc.get_advert_path(&key).await {
        Err(e) => c.line(&format!("Error getting advert path : {e}")),
        Ok(path) => {
            let v = to_value(&path);
            if c.json {
                c.json_line(&v);
            } else {
                let text = match vint(&v, "path_len") {
                    0 => "0 hop".to_string(),
                    -1 => "Flood".to_string(),
                    hops => format_path(&vstr(&v, "path"), hops, vint(&v, "path_hash_mode")),
                };
                c.line(&text);
            }
        }
    }
    Ok(1)
}

/// Looks up the contact of argument 1 and reports unknown ones.
async fn contact_arg(c: &mut Ctx<'_>) -> Result<Option<Contact>, CmdError> {
    let name = c.arg(1)?.to_string();
    let contact = c.app.contact_from_arg(&name).await;
    if contact.is_none() {
        c.unknown_contact(&name);
    }
    Ok(contact)
}

/// `share_contact <ct>`.
pub async fn share_contact(c: &mut Ctx<'_>) -> CmdResult {
    if let Some(contact) = contact_arg(c).await? {
        match c.app.mc.share_contact(&contact.public_key).await {
            Err(e) => c.line(&format!("Error while sharing contact: {e}")),
            Ok(()) => {
                if c.json {
                    c.json_indent(&json!({}));
                }
            }
        }
    }
    Ok(1)
}

/// `export_contact <ct>`: the contact's URI.
pub async fn export_contact(c: &mut Ctx<'_>) -> CmdResult {
    if let Some(contact) = contact_arg(c).await? {
        match c.app.mc.export_contact(Some(&contact.public_key)).await {
            Err(e) => c.line(&format!("Error exporting contact: {e}")),
            Ok(uri) => {
                if c.json {
                    c.json_line(&json!({"uri": uri}));
                } else {
                    c.line(&uri);
                }
            }
        }
    }
    Ok(1)
}

/// `upload_contact <ct>`: uploads the contact's URI to the map.
pub async fn upload_contact(c: &mut Ctx<'_>) -> CmdResult {
    if let Some(contact) = contact_arg(c).await? {
        match c.app.mc.export_contact(Some(&contact.public_key)).await {
            Err(e) => c.line(&format!("Error exporting contact: {e}")),
            Ok(uri) => {
                let response = super::device::upload_uri(uri).await?;
                if c.json {
                    c.json_line(&json!({"response": response}));
                } else {
                    c.line(&response);
                }
            }
        }
    }
    Ok(1)
}

/// `import_contact <meshcore://...>`.
pub async fn import_contact(c: &mut Ctx<'_>) -> CmdResult {
    let uri = c.arg(1)?.to_string();
    if let Some(card) = uri.strip_prefix("meshcore://") {
        let data = crate::util::py::fromhex(card).map_err(CmdError::Failed)?;
        if let Err(e) = c.app.mc.import_contact(&data).await {
            c.line(&format!("Error while importing contact: {e}"));
        } else {
            log::info!("Contact successfully added");
            let _ = c.app.mc.get_contacts(0).await;
        }
    }
    Ok(1)
}

/// `remove_contact <ct>`.
pub async fn remove_contact(c: &mut Ctx<'_>) -> CmdResult {
    if let Some(contact) = contact_arg(c).await? {
        if let Err(e) = c.app.mc.remove_contact(&contact.public_key).await {
            c.line(&format!("Error removing contact: {e}"));
        } else {
            if c.json {
                c.json_indent(&json!({}));
            }
            // the reference drops the contact's dict with its timeout
            c.app
                .state()
                .timeouts
                .remove(&contact.public_key.to_string());
        }
    }
    Ok(1)
}

/// `contact_timeout <ct> <secs>`: default timeout for requests to a contact.
pub async fn contact_timeout(c: &mut Ctx<'_>) -> CmdResult {
    let name = c.arg(1)?.to_string();
    // like the reference, the value is only read for a known contact
    match c.app.contact_from_arg(&name).await {
        None => {
            let text = format!("Unknown contact {name}");
            c.error(&json!({"error": "unknown contact"}), &text);
        }
        Some(contact) => {
            let secs = crate::util::py::float(c.arg(2)?).map_err(CmdError::Failed)?;
            c.app
                .state()
                .timeouts
                .insert(contact.public_key.to_string(), secs);
        }
    }
    Ok(2)
}

/// Builds the contact used by `add_discovered` and `add_contact`.
#[must_use]
pub fn new_contact(key: PublicKey, kind: u8, name: &str) -> Contact {
    Contact::new(key, ContactType::from_code(kind), name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(format_path("aabbcc", 3, 0), "aa,bb,cc");
        assert_eq!(format_path("aabbccdd", 2, 1), "aabb,ccdd");
        assert_eq!(format_path("aabbccddeeff", 2, 2), "aabbcc,ddeeff");
        assert_eq!(contact_path_str(&json!({"out_path_len": -1})), "Flood");
        assert_eq!(contact_path_str(&json!({"out_path_len": 0})), "0 hop");
        assert_eq!(
            contact_path_str(
                &json!({"out_path_len": 2, "out_path": "1122", "out_path_hash_mode": 0})
            ),
            "11,22"
        );
    }

    #[test]
    fn path_arguments() {
        assert_eq!(parse_path_arg("0").unwrap(), (vec![], None));
        assert_eq!(
            parse_path_arg("11,44,77").unwrap(),
            (vec![0x11, 0x44, 0x77], Some(0))
        );
        assert_eq!(
            parse_path_arg("1122,4455").unwrap(),
            (vec![0x11, 0x22, 0x44, 0x55], Some(1))
        );
        assert_eq!(parse_path_arg("112244557788:1").unwrap().1, Some(1));
        assert_eq!(
            parse_path_arg("114477").unwrap(),
            (vec![0x11, 0x44, 0x77], None)
        );
        assert!(parse_path_arg("zz").is_err());
        assert!(parse_path_arg("11:x").is_err());
        // int() of the field after the first colon only
        assert_eq!(
            parse_path_arg("1122: 1:x").unwrap(),
            (vec![0x11, 0x22], Some(1))
        );
        assert_eq!(parse_path_arg("11,22:\u{661}").unwrap().1, Some(1));
        // a one digit first hash gives mode -1
        assert!(parse_path_arg("1,22").is_err());
        assert!(parse_path_arg("11:-1").is_err());
    }

    #[test]
    fn sorting() {
        let mut v = vec![
            json!({"adv_name": "b", "type": 2, "flags": 0}),
            json!({"adv_name": "a", "type": 1, "flags": 1}),
            json!({"adv_name": "c", "type": 2, "flags": 1}),
        ];
        sort_contacts(&mut v, "n");
        assert_eq!(
            v.iter().map(|x| vstr(x, "adv_name")).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        sort_contacts(&mut v, "N");
        assert_eq!(
            v.iter().map(|x| vstr(x, "adv_name")).collect::<Vec<_>>(),
            ["c", "b", "a"]
        );
        sort_contacts(&mut v, "fn");
        assert_eq!(
            v.iter().map(|x| vstr(x, "adv_name")).collect::<Vec<_>>(),
            ["a", "c", "b"]
        );
        sort_contacts(&mut v, "Tn");
        assert_eq!(
            v.iter().map(|x| vstr(x, "adv_name")).collect::<Vec<_>>(),
            ["b", "c", "a"]
        );
    }

    #[test]
    fn type_letters() {
        assert_eq!(type_from_letter('r'), Some(2));
        assert_eq!(type_from_letter('4'), Some(4));
        assert_eq!(type_from_letter('x'), None);
    }
}
