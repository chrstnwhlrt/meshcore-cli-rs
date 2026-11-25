//! Scripts, aliases, handlers and small helpers.

use serde_json::{Value, json};

use super::{CmdError, CmdResult, Ctx};

/// Seconds given to `sleep` and `trywait_msg`: the reference takes
/// `int(value)`; fractions are accepted here too, anything else (also
/// `nan` and `inf`, which `float()` reads) fails with the message of
/// `int()`.
///
/// # Errors
///
/// Returns Python's `int()` error for text that is no number.
pub fn seconds(text: &str) -> Result<f64, CmdError> {
    match crate::util::py::int(text, 10) {
        #[allow(clippy::cast_precision_loss)]
        Ok(secs) => Ok(secs as f64),
        Err(int_error) => crate::util::py::float(text)
            .ok()
            .filter(|secs| secs.is_finite())
            .ok_or(CmdError::Failed(int_error)),
    }
}

/// `sleep <secs>`.
pub async fn sleep(c: &mut Ctx<'_>) -> CmdResult {
    let secs = seconds(c.arg(1)?)?;
    if secs.is_finite() && secs > 0.0 {
        tokio::time::sleep(crate::util::time::duration_from_secs(secs)).await;
    }
    Ok(1)
}

/// `wait_key`: waits for Enter.
pub async fn wait_key(c: &mut Ctx<'_>) -> CmdResult {
    let prompt = if c.json {
        String::new()
    } else {
        format!("Press Enter to continue ...{}", c.end)
    };
    c.flush_out();
    // Ctrl-C and Ctrl-D end the wait as Enter does, as in the reference
    let _ = tokio::task::spawn_blocking(move || crate::editor::read_line(&prompt)).await;
    Ok(0)
}

/// Asks for a file name in a blocking task.
async fn ask_file() -> Option<String> {
    tokio::task::spawn_blocking(crate::repeater::prompt_for_file)
        .await
        .ok()
        .flatten()
}

/// `script [file]`.
pub async fn script(c: &mut Ctx<'_>) -> CmdResult {
    let (argnum, file) = if c.has(1) {
        (1, Some(c.arg(1)?.to_string()))
    } else {
        (0, ask_file().await)
    };
    let Some(file) = file else {
        return Err(CmdError::Cancelled);
    };
    // expanded as typed; process_script strips it afterwards
    let file = crate::paths::expand_user(&file).display().to_string();
    c.flush_out();
    let (app, json) = (c.app.clone(), c.json);
    let output = Box::pin(super::process_script(&app, &file, json, c.sink))
        .await
        .map_err(CmdError::Failed)?;
    c.emit(&output);
    Ok(argnum)
}

/// `alias [key [value]]`.
pub fn alias(c: &mut Ctx<'_>) -> CmdResult {
    if c.has(2) {
        let key = c.arg(1)?.to_string();
        let value = c.arg(2)?.to_string();
        let mut state = c.app.state();
        if value.is_empty() {
            state.remove_alias(&key);
        } else {
            state.set_alias(&key, &value);
        }
        Ok(2)
    } else if c.has(1) {
        let key = c.arg(1)?.to_string();
        let value = c.app.state().alias(&key);
        match value {
            Some(value) => c.line(&value),
            None => log::error!("Alias not set"),
        }
        Ok(1)
    } else {
        let value = c.app.state().aliases_value();
        c.json_indent(&value);
        Ok(0)
    }
}

/// `aliases_load [file]`: loads aliases from a json object.
pub async fn aliases_load(c: &mut Ctx<'_>) -> CmdResult {
    let (argnum, file) = if c.has(1) {
        (1, Some(c.arg(1)?.to_string()))
    } else {
        (0, ask_file().await)
    };
    let Some(file) = file else {
        return Err(CmdError::Cancelled);
    };
    let Some(path) = crate::paths::resolve_existing(&file) else {
        log::error!("File not found {}", crate::util::py::strip(&file));
        return Ok(argnum);
    };
    // json.load(f): f.read() of the text, then json.loads
    let content = crate::util::py::read_text(&path, crate::util::py::TextRead::All)
        .map_err(|e| CmdError::Failed(e.message(&path.display().to_string())))?;
    let parsed = crate::util::json::py_loads(&crate::util::py::universal_newlines(&content))
        .map_err(CmdError::Failed)?;
    match parsed {
        Value::Object(map) if map.values().all(Value::is_string) => {
            let mut state = c.app.state();
            for (key, value) in map {
                state.set_alias(&key, value.as_str().unwrap_or_default());
            }
        }
        _ => log::error!("Malformed aliases file"),
    }
    Ok(argnum)
}

/// `handler_attach <rxlog|msgs|chan_data> <cmd>`.
pub fn handler_attach(c: &mut Ctx<'_>) -> CmdResult {
    let kind = c.arg(1)?.to_string();
    let cmd = c.arg(2)?.to_string();
    match c.app.handlers.attach(&kind, &cmd) {
        Ok(description) => {
            if c.json {
                c.json_line(&description);
            } else {
                let id = description.get("id").cloned().unwrap_or(Value::Null);
                c.line(&format!("New handler {id}"));
            }
        }
        Err(crate::handlers::AttachError::Logged(e)) => log::error!("{e}"),
        Err(crate::handlers::AttachError::Raised(e)) => return Err(CmdError::Failed(e)),
    }
    Ok(2)
}

/// `handler_detach <id>`.
pub async fn handler_detach(c: &mut Ctx<'_>) -> CmdResult {
    // int(cmds[1]) and the KeyError of an unknown number, as in the reference
    let number = crate::util::py::int(c.arg(1)?, 10).map_err(CmdError::Failed)?;
    let id = u32::try_from(number).map_err(|_| CmdError::Failed(number.to_string()))?;
    match c.app.handlers.detach(id).await {
        Ok((description, cmd)) => {
            if c.json {
                c.json_line(&json!({"handler": description, "status": "detatched"}));
            } else {
                c.line(&format!("Successfully detached {id} of cmd {cmd}"));
            }
        }
        Err(e) => return Err(CmdError::Failed(e)),
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_like_int_with_fractions() {
        assert_eq!(seconds("3").ok(), Some(3.0));
        assert_eq!(seconds(" 0.5 ").ok(), Some(0.5));
        for text in ["nan", "inf", "-Infinity", "x"] {
            assert!(
                matches!(seconds(text), Err(CmdError::Failed(m)) if m.starts_with("invalid literal for int()")),
                "{text}"
            );
        }
    }
}
