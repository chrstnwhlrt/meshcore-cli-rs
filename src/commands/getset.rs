//! `get` and `set`.

use serde_json::{Value, json};

use super::{CmdError, CmdResult, Ctx};
use crate::app::{to_value, vf64, vint, vstr};
use crate::display::py_number;

/// Python's `str(bool)`.
const fn py_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

/// `on`/`off`.
const fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// True for the words the reference accepts as "on".
fn is_on(value: &str) -> bool {
    matches!(value, "on" | "true" | "yes" | "1")
}

/// Parses a telemetry mode: always (2), device/selected (1), never (0).
#[must_use]
pub fn telemetry_mode(value: &str) -> u8 {
    if value == "2" || value.starts_with("al") || value == "yes" || value == "on" {
        2
    } else {
        u8::from(value == "1" || value == "selected" || value.starts_with("dev"))
    }
}

/// Parses the `autoadd_config` argument: a number (`int(x, 0)`) or keywords
/// (`ov`, `cli`/`chat`, `rep`/`rpt`, `room`, `sen`).
///
/// # Errors
///
/// A number that does not fit in a byte fails like Python's `to_bytes`.
pub fn autoadd_flags(value: &str) -> Result<u8, CmdError> {
    let value = value.to_lowercase();
    match crate::util::py::int(&value, 0) {
        Ok(number) => return to_byte_range(number),
        // a number beyond 64 bits is no list of names
        Err(e) if !e.starts_with("invalid literal") => return Err(CmdError::Failed(e)),
        Err(_) => {}
    }
    let mut flags = 0u8;
    if value.contains("ov") {
        flags |= 0x01;
    }
    if value.contains("cha") || value.contains("cli") {
        flags |= 0x02;
    }
    if value.contains("rep") || value.contains("rpt") {
        flags |= 0x04;
    }
    if value.contains("roo") {
        flags |= 0x08;
    }
    if value.contains("sen") {
        flags |= 0x10;
    }
    Ok(flags)
}

/// A Python int converted for a setter; out of range values fail with the
/// messages of Python's `int.to_bytes`.
fn to_byte_range<T: TryFrom<i64>>(number: i64) -> Result<T, CmdError> {
    T::try_from(number).map_err(|_| {
        CmdError::Failed(if number < 0 {
            "can't convert negative int to unsigned".to_string()
        } else {
            "int too big to convert".to_string()
        })
    })
}

/// Python's `int(value)` for a setter argument.
fn py_int<T: TryFrom<i64>>(value: &str) -> Result<T, CmdError> {
    to_byte_range(crate::util::py::int(value, 10).map_err(CmdError::Failed)?)
}

/// Python's `float(value)` for a setter argument.
fn py_float(value: &str) -> Result<f64, CmdError> {
    crate::util::py::float(value).map_err(CmdError::Failed)
}

/// A float Python can turn into an int: not NaN and not infinite.
fn integral(value: f64) -> Result<f64, CmdError> {
    if value.is_nan() {
        Err(CmdError::Failed(
            "cannot convert float NaN to integer".to_string(),
        ))
    } else if value.is_infinite() {
        Err(CmdError::Failed(
            "cannot convert float infinity to integer".to_string(),
        ))
    } else {
        Ok(value)
    }
}

/// A radio frequency (MHz) or bandwidth (kHz), failing like `meshcore_py`'s
/// `int(float(value) * 1000).to_bytes(4, "little")`. meshcore-rs sends it
/// rounded to the nearest thousandth, where `meshcore_py` cuts the rest off.
fn radio_thousandths(value: f64) -> Result<f64, CmdError> {
    let scaled = integral((value * 1000.0).round())?;
    if scaled < 0.0 {
        Err(CmdError::Failed(
            "can't convert negative int to unsigned".to_string(),
        ))
    } else if scaled > f64::from(u32::MAX) {
        Err(CmdError::Failed("int too big to convert".to_string()))
    } else {
        Ok(value)
    }
}

/// A coordinate in whole millionths of a degree, rounded and failing like
/// `meshcore_py`'s `round(value * 1e6).to_bytes(4, "little", signed=True)`:
/// halves go to the even number, as Python's `round` does.
fn micro_degrees(value: f64) -> Result<f64, CmdError> {
    let micro = integral((value * 1e6).round_ties_even())?;
    if (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&micro) {
        // meshcore-rs multiplies again and rounds, which gives `micro` back
        Ok(micro / 1e6)
    } else {
        Err(CmdError::Failed("int too big to convert".to_string()))
    }
}

/// A retry count: `int(value)`; the reference also takes negative counts,
/// which mean no attempt at all.
fn retry_count(value: &str) -> Result<u32, CmdError> {
    let number = crate::util::py::int(value, 10).map_err(CmdError::Failed)?;
    Ok(u32::try_from(number.max(0)).unwrap_or(u32::MAX))
}

/// Echo of a display setting in json mode.
fn echo_setting(c: &mut Ctx<'_>, name: &str, value: &str) {
    if c.json {
        c.json_line(&json!({"cmd": name, "param": value}));
    }
}

/// Result of a device setter: `ok`, the payload in json, or the error.
fn device_result(c: &mut Ctx<'_>, result: Result<(), meshcore::Error>) {
    match result {
        Err(e) => c.line(&format!("Error: {e}")),
        Ok(()) => {
            if c.json {
                c.json_indent(&json!({}));
            } else {
                c.line("ok");
            }
        }
    }
}

/// `set <param> <value>`.
pub async fn set(c: &mut Ctx<'_>) -> CmdResult {
    let param = c.arg(1)?.to_string();
    if param == "help" {
        // printed like the reference does, past any redirection
        crate::output::print(&crate::help::help_for("set"));
        return Ok(1);
    }
    let value = c.arg(2)?.to_string();
    let app = c.app.clone();

    match param.as_str() {
        "lc_output_format" => app.settings().lc_output_format = value,
        "lc_sort_order" => app.settings().lc_sort_order = value,
        "max_flood_attempts" => app.settings().max_flood_attempts = retry_count(&value)?,
        "max_attempts" => app.settings().max_attempts = retry_count(&value)?,
        "flood_after" => app.settings().flood_after = retry_count(&value)?,
        "classic_prompt" => {
            app.settings().classic = value == "on";
            echo_setting(c, &param, &value);
        }
        "arrow_head" => app.settings().arrow_head = value,
        "slash_start" => app.settings().slash_start = value,
        "slash_end" => app.settings().slash_end = value,
        "invert_slash" => app.settings().invert_slash = value == "on",
        "display_channel_data" => app.settings().display_channel_data = value == "on",
        "color" | "print_timestamp" | "print_snr" | "json_log_rx" | "channel_echoes"
        | "advert_echoes" | "echo_unk_chans" | "print_adverts" | "print_path_updates"
        | "print_new_contacts" | "json_msgs" => {
            {
                let mut s = app.settings();
                let on = value == "on";
                match param.as_str() {
                    "color" => s.color = on,
                    "print_timestamp" => s.timestamp.clone_from(&value),
                    "print_snr" => s.print_snr = on,
                    "json_log_rx" => s.json_log_rx = on,
                    "channel_echoes" => s.channel_echoes = on,
                    "advert_echoes" => s.advert_echoes = on,
                    "echo_unk_chans" => s.echo_unk_chans = on,
                    "print_adverts" => s.print_adverts = on,
                    "print_path_updates" => s.print_path_updates = on,
                    "print_new_contacts" => s.print_new_contacts = on,
                    _ => s.json_msgs = on,
                }
            }
            echo_setting(c, &param, &value);
        }
        "pin" => {
            let pin: u32 = py_int(&value)?;
            let result = app.mc.set_devicepin(pin).await;
            device_result(c, result);
        }
        "radio" => {
            let params: Vec<&str> = value.split(',').collect();
            if params.len() < 4 {
                return Err(CmdError::Params);
            }
            let repeat = params
                .get(4)
                .map(|r| matches!(*r, "repeat" | "on" | "1" | "yes"));
            // each value is read and packed before the next, as in meshcore_py
            let result = app
                .mc
                .set_radio(
                    radio_thousandths(py_float(params[0])?)?,
                    radio_thousandths(py_float(params[1])?)?,
                    py_int(params[2])?,
                    py_int(params[3])?,
                    repeat,
                )
                .await;
            device_result(c, result);
        }
        "path_hash_mode" | "path.hash.mode" => {
            let mode = crate::util::py::int(&value, 10).map_err(CmdError::Failed)?;
            if mode >= 3 {
                log::error!("Can't set value to {mode}");
            } else {
                let result = app.mc.set_path_hash_mode(to_byte_range(mode)?).await;
                device_result(c, result);
            }
        }
        "name" => {
            let result = app.mc.set_name(&value).await;
            device_result(c, result);
        }
        "tx" => {
            // the reference sends int(value) unsigned, so negative values fail
            let dbm = crate::util::py::int(&value, 10).map_err(CmdError::Failed)?;
            if dbm < 0 {
                return Err(CmdError::Failed(
                    "can't convert negative int to unsigned".to_string(),
                ));
            }
            let result = app.mc.set_tx_power(to_byte_range(dbm)?).await;
            device_result(c, result);
        }
        "lat" | "lon" => {
            let info = app.self_info_value();
            let number = py_float(&value)?;
            let (lat, lon) = if param == "lat" {
                (number, vf64(&info, "adv_lon"))
            } else {
                (vf64(&info, "adv_lat"), number)
            };
            let result = app
                .mc
                .set_coords(micro_degrees(lat)?, micro_degrees(lon)?)
                .await;
            device_result(c, result);
        }
        "coords" => {
            let mut parts = value.split(',');
            let lat = py_float(parts.next().unwrap_or_default())?;
            let lon = py_float(parts.next().ok_or(CmdError::Params)?)?;
            let result = app
                .mc
                .set_coords(micro_degrees(lat)?, micro_degrees(lon)?)
                .await;
            device_result(c, result);
        }
        "private_key" => {
            let bytes = crate::util::py::fromhex(&value).map_err(CmdError::Failed)?;
            // the firmware rejects other lengths, so do not send them
            match <[u8; 64]>::try_from(bytes.as_slice()) {
                Ok(key) => {
                    let result = app.mc.import_private_key(&key).await;
                    device_result(c, result);
                }
                Err(_) => c.line(&format!(
                    "Error: private key must be 64 bytes, got {}",
                    bytes.len()
                )),
            }
        }
        "tuning" => {
            // the reference reads both numbers before meshcore_py packs them
            let mut parts = value.split(',');
            let int = |text: &str| crate::util::py::int(text, 10).map_err(CmdError::Failed);
            let rx_delay = int(parts.next().unwrap_or_default())?;
            let airtime_factor = int(parts.next().ok_or(CmdError::Params)?)?;
            let result = app
                .mc
                .set_tuning(to_byte_range(rx_delay)?, to_byte_range(airtime_factor)?)
                .await;
            device_result(c, result);
        }
        "manual_add_contacts" => {
            let on = is_on(&value);
            match app.mc.set_manual_add_contacts(on).await {
                Err(e) => c.line(&format!("Error : {e}")),
                Ok(()) => c.line(&format!("manual add contact: {}", py_bool(on))),
            }
        }
        "autoadd_config" => {
            let flags = autoadd_flags(&value)?;
            if let Err(e) = app.mc.set_autoadd_config(flags, None).await {
                c.line(&format!("Error : {e}"));
            }
        }
        "multi_acks" => {
            let on = is_on(&value);
            match app.mc.set_multi_acks(u8::from(on)).await {
                Err(e) => c.line(&format!("Error : {e}")),
                Ok(()) => c.line(&format!("multi_acks: {}", py_bool(on))),
            }
        }
        "auto_update_contacts" => app.mc.set_auto_update_contacts(is_on(&value)),
        "telemetry_mode_base" | "telemetry_mode_loc" | "telemetry_mode_env" => {
            let mode = telemetry_mode(&value);
            let (result, label) = match param.as_str() {
                "telemetry_mode_base" => {
                    (app.mc.set_telemetry_mode_base(mode).await, "telemetry mode")
                }
                "telemetry_mode_loc" => (
                    app.mc.set_telemetry_mode_loc(mode).await,
                    "telemetry mode for location",
                ),
                _ => (
                    app.mc.set_telemetry_mode_env(mode).await,
                    "telemetry mode for env",
                ),
            };
            match result {
                Err(e) => c.line(&format!("Error : {e}")),
                Ok(()) => c.line(&format!("{label}: {mode}")),
            }
        }
        "advert_loc_policy" => {
            let policy = u8::from(value == "1" || value == "share");
            match app.mc.set_advert_loc_policy(policy).await {
                Err(e) => c.line(&format!("Error : {e}")),
                Ok(()) => c.line(&format!("Policy for adv_loc: {policy}")),
            }
        }
        "default_scope" => match app.mc.set_default_flood_scope(Some(value.as_str())).await {
            Err(e) => c.line(&format!("Error : {e}")),
            Ok(()) => c.line("Default scope set"),
        },
        _ => {
            let name = param.strip_prefix('_').unwrap_or(&param).to_string();
            match app.mc.set_custom_var(&name, &value).await {
                Err(e) => {
                    log::debug!("No config found by that {e}, trying to exec line as cli");
                    return Err(CmdError::Unknown);
                }
                Ok(()) => {
                    if c.json {
                        c.json_line(&json!({"result": "set", "var": name, "value": value}));
                    } else {
                        c.line(&format!("Var {name} set to {value}"));
                    }
                }
            }
        }
    }
    Ok(2)
}

/// Prints the payload `meshcore_py` gives a failed request, or the error.
fn failed_answer(c: &mut Ctx<'_>, error: &meshcore::Error) {
    match super::py_error_payload(error) {
        Some(payload) => c.json_line(&payload),
        None => c.line(&format!("Error: {error}")),
    }
}

/// Prints a local setting.
fn show(c: &mut Ctx<'_>, key: &str, json_value: Value, text: &str) {
    if c.json {
        let mut map = serde_json::Map::new();
        map.insert(key.to_string(), json_value);
        c.json_line(&Value::Object(map));
    } else {
        c.line(text);
    }
}

/// `get <param>`.
pub async fn get(c: &mut Ctx<'_>) -> CmdResult {
    let param = c.arg(1)?.to_string();
    let app = c.app.clone();
    let s = app.settings().clone();

    match param.as_str() {
        "help" => crate::output::print(&crate::help::help_for("get")),
        "lc_output_format" => show(
            c,
            "lc_output_format",
            json!(s.lc_output_format),
            &format!("lc_output_format: {}", s.lc_output_format),
        ),
        "lc_sort_order" => show(
            c,
            "lc_sort_order",
            json!(s.lc_sort_order),
            &format!("lc_sort_order: \"{}\"", s.lc_sort_order),
        ),
        "max_attempts" => show(
            c,
            "max_attempts",
            json!(s.max_attempts),
            &format!("max_attempts: {}", s.max_attempts),
        ),
        "max_flood_attempts" => show(
            c,
            "max_flood_attempts",
            json!(s.max_flood_attempts),
            &format!("max_flood_attempts: {}", s.max_flood_attempts),
        ),
        "flood_after" => show(
            c,
            "flood_after",
            json!(s.flood_after),
            &format!("flood_after: {}", s.flood_after),
        ),
        "classic_prompt" => show(c, "classic_prompt", json!(s.classic), on_off(s.classic)),
        "json_msgs" => show(c, "json_msgs", json!(s.json_msgs), on_off(s.json_msgs)),
        "display_channel_data" => show(
            c,
            "display_channel_data",
            json!(s.display_channel_data),
            on_off(s.display_channel_data),
        ),
        "color" => show(c, "color", json!(s.color), on_off(s.color)),
        "print_timestamp" => show(c, "timestamp", json!(s.timestamp), &s.timestamp),
        "json_log_rx" => show(
            c,
            "json_log_rx",
            json!(s.json_log_rx),
            on_off(s.json_log_rx),
        ),
        "channel_echoes" => show(
            c,
            "channel_echoes",
            json!(s.channel_echoes),
            on_off(s.channel_echoes),
        ),
        "advert_echoes" => show(
            c,
            "advert_echoes",
            json!(s.advert_echoes),
            on_off(s.advert_echoes),
        ),
        "echo_unk_chans" => show(
            c,
            "echo_unk_chans",
            json!(s.echo_unk_chans),
            on_off(s.echo_unk_chans),
        ),
        "print_adverts" => show(
            c,
            "print_adverts",
            json!(s.print_adverts),
            on_off(s.print_adverts),
        ),
        "print_path_updates" => show(
            c,
            "print_path_updates",
            json!(s.print_path_updates),
            on_off(s.print_path_updates),
        ),
        "print_new_contacts" => show(
            c,
            "print_new_contacts",
            json!(s.print_new_contacts),
            on_off(s.print_new_contacts),
        ),
        "print_snr" => show(c, "print_snr", json!(s.print_snr), on_off(s.print_snr)),
        "name"
        | "tx"
        | "coords"
        | "lat"
        | "lon"
        | "multi_acks"
        | "manual_add_contacts"
        | "telemetry_mode_base"
        | "telemetry_mode_loc"
        | "telemetry_mode_env"
        | "advert_loc_policy" => {
            let _ = app.mc.send_appstart().await;
            let info = app.self_info_value();
            get_self_info_field(c, &param, &info);
        }
        "radio" => {
            let _ = app.mc.send_appstart().await;
            let info = app.self_info_value();
            let mut radio = serde_json::Map::new();
            for key in ["radio_freq", "radio_bw", "radio_sf", "radio_cr"] {
                radio.insert(key.into(), info.get(key).cloned().unwrap_or(Value::Null));
            }
            if let Ok(device) = app.mc.send_device_query().await {
                let device = to_value(&device);
                if let Some(repeat) = device.get("repeat") {
                    radio.insert("repeat".into(), repeat.clone());
                }
            }
            let radio = Value::Object(radio);
            if c.json {
                c.json_line(&radio);
            } else {
                let mut text = format!(
                    "{},{},{},{}",
                    py_number(&radio["radio_freq"]),
                    py_number(&radio["radio_bw"]),
                    py_number(&radio["radio_sf"]),
                    py_number(&radio["radio_cr"])
                );
                if let Some(repeat) = radio.get("repeat") {
                    text.push(',');
                    text.push_str(on_off(repeat.as_bool().unwrap_or(false)));
                }
                c.line(&text);
            }
        }
        "repeat" => match app.mc.send_device_query().await {
            Err(e) => c.line(&format!("ERROR: {e}")),
            Ok(device) => {
                let device = to_value(&device);
                if c.json {
                    c.json_indent(&device);
                } else if let Some(repeat) = device.get("repeat") {
                    c.line(&format!(
                        "Repeat: {}",
                        on_off(repeat.as_bool().unwrap_or(false))
                    ));
                } else {
                    c.line("Can't repeat");
                }
            }
        },
        "path_hash_mode" | "path.hash.mode" => match app.mc.send_device_query().await {
            Err(e) => c.line(&format!("ERROR: {e}")),
            Ok(device) => {
                let device = to_value(&device);
                if c.json {
                    c.json_indent(&device);
                } else if let Some(mode) = device.get("path_hash_mode") {
                    c.line(&py_number(mode));
                } else {
                    c.line("Not available");
                }
            }
        },
        "bat" => match app.mc.get_bat().await {
            Err(e) => c.line(&format!("Error getting bat {e}")),
            Ok(bat) => {
                let bat = to_value(&bat);
                if c.json {
                    c.json_indent(&bat);
                } else {
                    c.line(&format!("Battery level : {}", vint(&bat, "level")));
                }
            }
        },
        "fstats" => match app.mc.get_bat().await {
            Ok(bat) if to_value(&bat).get("used_kb").is_some() => {
                let bat = to_value(&bat);
                if c.json {
                    c.json_indent(&bat);
                } else {
                    c.line(&format!(
                        "Using {}kB of {}kB",
                        vint(&bat, "used_kb"),
                        vint(&bat, "total_kb")
                    ));
                }
            }
            // firmware without storage info: the reference prints the
            // event meshcore_py gave it
            Ok(bat) => c.line(&format!(
                "Error getting fs stats Event(type=<EventType.BATTERY: 'battery_info'>, payload={}, attributes={{}})",
                crate::util::json::py_repr(&to_value(&bat))
            )),
            Err(e) => c.line(&format!("Error getting fs stats {e}")),
        },
        "private_key" => match app.mc.export_private_key().await {
            Err(e) => c.line(&format!("Error exporting private key {e}")),
            Ok(key) => {
                // the library serializes the key as a hex string
                let value = to_value(&key);
                let hex = value
                    .as_str()
                    .map_or_else(|| vstr(&value, "private_key"), str::to_string);
                if c.json {
                    c.json_line(&json!({ "private_key": hex }));
                } else {
                    c.line(&format!("Private key: {hex}"));
                }
            }
        },
        "autoadd_config" => match app.mc.get_autoadd_config().await {
            Err(_) => log::error!("Can't get autoadd_config"),
            Ok(config) => {
                let config = to_value(&config);
                c.line(&format!("0x{:02x}", vint(&config, "config")));
            }
        },
        "auto_update_contacts" => {
            let on = app.mc.auto_update_contacts();
            show(
                c,
                "auto_update_contacts",
                json!(on),
                &format!("auto_update_contacts: {}", on_off(on)),
            );
        }
        "custom" => match app.mc.get_custom_vars().await {
            Err(e) => {
                if c.json {
                    c.json_line(&json!({"error": e.to_string()}));
                } else {
                    log::error!("Couldn't get custom variables");
                }
            }
            Ok(vars) => c.json_indent(&to_value(&vars)),
        },
        "stats_core" => stats(c, app.mc.get_stats_core().await.map(|s| to_value(&s))),
        "stats_radio" => stats(c, app.mc.get_stats_radio().await.map(|s| to_value(&s))),
        "stats_packets" => stats(c, app.mc.get_stats_packets().await.map(|s| to_value(&s))),
        "stats" | "status" => {
            let mut merged = serde_json::Map::new();
            for (label, result) in [
                ("core", app.mc.get_stats_core().await.map(|s| to_value(&s))),
                (
                    "radio",
                    app.mc.get_stats_radio().await.map(|s| to_value(&s)),
                ),
                (
                    "packets",
                    app.mc.get_stats_packets().await.map(|s| to_value(&s)),
                ),
            ] {
                match result {
                    Ok(Value::Object(map)) => merged.extend(map),
                    Ok(_) => {}
                    Err(e) => {
                        // the reference merges the core answer before it
                        // checks it, so its failure shows up in the output
                        if label == "core"
                            && let Some(Value::Object(map)) = super::py_error_payload(&e)
                        {
                            merged.extend(map);
                        }
                        log::error!("Couldn't get {label} stats");
                    }
                }
            }
            c.json_indent(&Value::Object(merged));
        }
        // the reference prints the answer, a failed one included
        "allowed_repeat_freq" => match app.mc.get_allowed_repeat_freq().await {
            Ok(freqs) => c.json_line(&json!({ "freqs": to_value(&freqs) })),
            Err(e) => failed_answer(c, &e),
        },
        "default_scope" => match app.mc.get_default_flood_scope().await {
            Ok(Some(scope)) => c.json_line(&to_value(&scope)),
            Ok(None) => c.json_line(&json!({})),
            Err(e) => failed_answer(c, &e),
        },
        _ => {
            let vars = match app.mc.get_custom_vars().await {
                Ok(vars) => to_value(&vars),
                Err(e) => {
                    if c.json {
                        c.json_line(&json!({"error": e.to_string()}));
                    } else {
                        log::error!("Couldn't get custom variables");
                    }
                    return Ok(1);
                }
            };
            let name = param.strip_prefix('_').unwrap_or(&param);
            let Some(value) = vars.get(name) else {
                log::debug!("No config found by that name, trying to exec line as cli");
                return Err(CmdError::Unknown);
            };
            if c.json {
                c.json_line(&json!({"var": name, "value": value}));
            } else {
                let text = value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_string);
                c.line(&text);
            }
        }
    }
    Ok(1)
}

/// Prints a field of the node's own info (`get name`, `get tx` ...).
fn get_self_info_field(c: &mut Ctx<'_>, param: &str, info: &Value) {
    let field = |key: &str| info.get(key).cloned().unwrap_or(Value::Null);
    match param {
        "name" => {
            if c.json {
                c.json_line(&field("name"));
            } else {
                c.line(&vstr(info, "name"));
            }
        }
        "tx" => {
            if c.json {
                c.json_line(&field("tx_power"));
            } else {
                c.line(&py_number(&field("tx_power")));
            }
        }
        "coords" => {
            if c.json {
                c.json_line(&json!({"lat": field("adv_lat"), "lon": field("adv_lon")}));
            } else {
                c.line(&format!(
                    "{},{}",
                    py_number(&field("adv_lat")),
                    py_number(&field("adv_lon"))
                ));
            }
        }
        "lat" => {
            if c.json {
                c.json_line(&json!({"lat": field("adv_lat")}));
            } else {
                c.line(&py_number(&field("adv_lat")));
            }
        }
        "lon" => {
            if c.json {
                c.json_line(&json!({"lon": field("adv_lon")}));
            } else {
                c.line(&py_number(&field("adv_lon")));
            }
        }
        "manual_add_contacts" => {
            let on = field("manual_add_contacts").as_bool().unwrap_or(false);
            show(
                c,
                "manual_add_contacts",
                json!(on),
                &format!("manual_add_contacts: {}", py_bool(on)),
            );
        }
        "advert_loc_policy" => {
            let v = field("adv_loc_policy");
            let text = format!("advert_loc_policy: {}", py_number(&v));
            show(c, "advert_loc_policy", v, &text);
        }
        other => {
            let v = field(other);
            let text = format!("{other}: {}", py_number(&v));
            show(c, other, v, &text);
        }
    }
}

fn stats(c: &mut Ctx<'_>, result: Result<Value, meshcore::Error>) {
    match result {
        Ok(v) => c.json_indent(&v),
        Err(_) => log::error!("Couldn't get stats"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_modes() {
        assert_eq!(telemetry_mode("always"), 2);
        assert_eq!(telemetry_mode("all"), 2);
        assert_eq!(telemetry_mode("on"), 2);
        assert_eq!(telemetry_mode("device"), 1);
        assert_eq!(telemetry_mode("selected"), 1);
        assert_eq!(telemetry_mode("1"), 1);
        assert_eq!(telemetry_mode("never"), 0);
        assert_eq!(telemetry_mode("off"), 0);
    }

    #[test]
    fn autoadd() {
        assert_eq!(autoadd_flags("0x1e").ok(), Some(0x1e));
        assert_eq!(autoadd_flags("12").ok(), Some(12));
        assert_eq!(autoadd_flags("ov,rep,room").ok(), Some(0x01 | 0x04 | 0x08));
        assert_eq!(autoadd_flags("cli sensor").ok(), Some(0x02 | 0x10));
        assert!(
            matches!(autoadd_flags("300"), Err(CmdError::Failed(m)) if m == "int too big to convert")
        );
        assert_eq!(autoadd_flags("nothing").ok(), Some(0));
        assert!(autoadd_flags("99999999999999999999").is_err());
    }

    fn failure<T: std::fmt::Debug>(result: Result<T, CmdError>) -> String {
        match result {
            Err(CmdError::Failed(message)) => message,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn radio_values_fail_like_meshcore_py() {
        assert_eq!(radio_thousandths(869.618).ok(), Some(869.618));
        assert_eq!(radio_thousandths(-0.000_4).ok(), Some(-0.000_4));
        assert_eq!(
            failure(radio_thousandths(-1.0)),
            "can't convert negative int to unsigned"
        );
        assert_eq!(
            failure(radio_thousandths(4_294_968.0)),
            "int too big to convert"
        );
        assert_eq!(
            failure(radio_thousandths(f64::NAN)),
            "cannot convert float NaN to integer"
        );
        assert_eq!(
            failure(radio_thousandths(f64::NEG_INFINITY)),
            "cannot convert float infinity to integer"
        );
    }

    #[test]
    fn coordinates_round_like_python() {
        // round(x * 1e6) in Python: halves to even
        assert_eq!(micro_degrees(0.000_002_5).ok(), Some(0.000_002));
        assert_eq!(micro_degrees(0.000_003_5).ok(), Some(0.000_004));
        assert_eq!(micro_degrees(-0.000_002_5).ok(), Some(-0.000_002));
        assert_eq!(micro_degrees(52.520_006_6).ok(), Some(52.520_007));
        assert_eq!(micro_degrees(90.000_000_4).ok(), Some(90.0));
        assert_eq!(micro_degrees(-2_147.483_648).ok(), Some(-2_147.483_648));
        assert_eq!(
            failure(micro_degrees(2_147.483_648)),
            "int too big to convert"
        );
        assert_eq!(
            failure(micro_degrees(f64::NAN)),
            "cannot convert float NaN to integer"
        );
        assert_eq!(
            failure(micro_degrees(f64::INFINITY)),
            "cannot convert float infinity to integer"
        );
    }
}
