//! Channel commands.

use serde_json::Value;

use super::{CmdResult, Ctx};
use crate::app::{App, ChannelEntry, to_value};
use crate::util::json::{dumps, py_repr};

/// True when a channel argument is a number rather than a name: decimal
/// digits only, of any script, as `int()` reads them. The reference takes
/// every `isnumeric()` text for a number, so a name like "三" or "½" fails
/// there; here it is a name.
pub fn is_channel_number(chan: &str) -> bool {
    !chan.is_empty() && chan.chars().all(|c| crate::util::py::decimal(c).is_some())
}

/// The slot of a channel number; one beyond a byte fails like
/// `meshcore_py`'s `to_bytes(1)` does in the reference.
///
/// # Errors
///
/// "int too big to convert" for a number beyond a byte.
pub fn slot_number(chan: &str) -> Result<u8, super::CmdError> {
    crate::util::py::int(chan, 10)
        .ok()
        .and_then(|n| u8::try_from(n).ok())
        .ok_or_else(|| super::CmdError::Failed("int too big to convert".to_string()))
}

/// Fails for a channel number beyond one byte, before anything is sent.
pub fn check_channel_number(chan: &str) -> Result<(), super::CmdError> {
    if is_channel_number(chan) {
        slot_number(chan)?;
    }
    Ok(())
}

/// Resolves a channel argument (number or name) to a slot number.
async fn channel_index(app: &App, chan: &str) -> Option<u8> {
    if is_channel_number(chan) {
        return slot_number(chan).ok();
    }
    app.channel_by_name(chan).await.map(|c| c.index)
}

/// Sets a channel and refreshes the cache (`set_channel` helper).
///
/// `chan` is a slot number or the name of an existing channel (`""` finds
/// the first free slot). Returns the new channel info.
pub async fn set_channel_slot(
    app: &App,
    chan: &str,
    name: &str,
    key: Option<[u8; 16]>,
) -> Option<ChannelEntry> {
    let index = channel_index(app, chan).await?;
    app.mc.set_channel(index, name, key).await.ok()?;
    let info = app.mc.get_channel(index).await.ok()?;
    let entry = ChannelEntry::from_value(to_value(&info));
    app.update_channel(entry.clone());
    Some(entry)
}

/// Parses a 32 character hex channel key; other lengths give `None`.
///
/// # Errors
///
/// Fails like the reference for keys that are not hex.
fn parse_key(key: &str) -> Result<Option<[u8; 16]>, super::CmdError> {
    if key.chars().count() != 32 {
        return Ok(None);
    }
    let bytes = crate::util::py::fromhex(key).map_err(super::CmdError::Failed)?;
    <[u8; 16]>::try_from(bytes.as_slice())
        .map(Some)
        .map_err(|_| super::CmdError::Failed("Channel secret must be exactly 16 bytes".to_string()))
}

/// `get_channel <n|name>`.
pub async fn get_channel(c: &mut Ctx<'_>) -> CmdResult {
    let chan = c.arg(1)?.to_string();
    let entry = if is_channel_number(&chan) {
        let index = slot_number(&chan)?;
        let cached = c
            .app
            .state()
            .channels
            .as_ref()
            .and_then(|chans| chans.iter().find(|e| e.index == index).cloned());
        match cached {
            Some(entry) => Some(entry),
            None => c
                .app
                .mc
                .get_channel(index)
                .await
                .ok()
                .map(|info| ChannelEntry::from_value(to_value(&info))),
        }
    } else {
        c.app.channel_by_name(&chan).await
    };
    match entry {
        Some(entry) => c.line(&py_repr(&entry.value)),
        None => c.line("Error while requesting channel info"),
    }
    Ok(1)
}

/// `get_channels` / `gc`: all used slots.
pub async fn get_channels(c: &mut Ctx<'_>) -> CmdResult {
    let channels = c.app.channels(false).await;
    let used: Vec<&ChannelEntry> = channels.iter().filter(|e| !e.name.is_empty()).collect();
    if c.json {
        let parts: Vec<String> = used.iter().map(|e| dumps(&e.value)).collect();
        let text = format!("[{}]", parts.join(",\n"));
        c.line(&text);
    } else {
        for entry in used {
            let secret = entry
                .value
                .get("channel_secret")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            c.emit(&format!("{}: {} [{secret}]\n", entry.index, entry.name));
        }
    }
    Ok(0)
}

/// `set_channel <n> <name> [key]`.
pub async fn set_channel(c: &mut Ctx<'_>) -> CmdResult {
    let chan = c.arg(1)?.to_string();
    let name = c.arg(2)?.to_string();
    // as in the reference, the slot number is packed only when the channel
    // is set: after the key is read, and not at all for a key of the wrong
    // length
    let (argnum, result) = if name.starts_with('#') || !c.has(3) {
        check_channel_number(&chan)?;
        (2, set_channel_slot(c.app, &chan, &name, None).await)
    } else {
        let key = parse_key(c.arg(3)?)?;
        let result = match key {
            Some(key) => {
                check_channel_number(&chan)?;
                set_channel_slot(c.app, &chan, &name, Some(key)).await
            }
            None => None,
        };
        (3, result)
    };
    if result.is_none() {
        c.line("Error setting channel");
    }
    Ok(argnum)
}

/// `add_channel <name> [key]`: uses the first free slot.
pub async fn add_channel(c: &mut Ctx<'_>) -> CmdResult {
    let name = c.arg(1)?.to_string();
    let (argnum, result) = if name.starts_with('#') || !c.has(2) {
        (1, set_channel_slot(c.app, "", &name, None).await)
    } else {
        let key = parse_key(c.arg(2)?)?;
        let result = match key {
            Some(key) => set_channel_slot(c.app, "", &name, Some(key)).await,
            None => None,
        };
        (2, result)
    };
    if result.is_none() {
        c.line("Error adding channel");
    }
    Ok(argnum)
}

/// `remove_channel <n|name>`: clears name and key of the slot.
pub async fn remove_channel(c: &mut Ctx<'_>) -> CmdResult {
    let chan = c.arg(1)?.to_string();
    check_channel_number(&chan)?;
    if set_channel_slot(c.app, &chan, "", Some([0u8; 16]))
        .await
        .is_none()
    {
        c.line("Error deleting channel");
    }
    Ok(1)
}

/// `scope <scope>`: sets the flood scope.
pub async fn scope(c: &mut Ctx<'_>) -> CmdResult {
    let scope = c.arg(1)?.to_string();
    if c.app.set_scope(&scope).await.is_none() {
        c.line("Error while setting scope");
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_numbers() {
        assert!(is_channel_number("3"));
        assert!(is_channel_number("\u{661}"));
        assert!(!is_channel_number(""));
        assert!(!is_channel_number("#test"));
        assert!(!is_channel_number("\u{4e09}"));
        assert!(!is_channel_number("+1"));
        assert_eq!(slot_number("\u{661}\u{662}").unwrap(), 12);
        assert!(
            matches!(slot_number("256"), Err(super::super::CmdError::Failed(m)) if m == "int too big to convert")
        );
        assert!(check_channel_number("public").is_ok());
    }

    #[test]
    fn keys() {
        assert_eq!(
            parse_key("00112233445566778899aabbccddeeff")
                .unwrap()
                .unwrap()[15],
            0xff
        );
        assert!(parse_key("0011").unwrap().is_none());
        assert!(parse_key("zz112233445566778899aabbccddeeff").is_err());
        assert!(parse_key("00112233445566778899aabbccdd eef").is_err());
    }
}
