//! Device commands: version, clock and time, reboot, adverts, the node's
//! own info, telemetry and contact card, its CLI and raw packets.

use serde_json::{Value, json};

use super::{CmdError, CmdResult, Ctx};
use crate::app::{to_value, vint, vstr};
use crate::util::time::{format_local, now_u32};

/// `on`/`off` for a json bool.
fn on_off(value: Option<bool>) -> &'static str {
    if value.unwrap_or(false) { "on" } else { "off" }
}

/// `ver`: firmware and model information.
pub async fn ver(c: &mut Ctx<'_>) -> CmdResult {
    match c.app.mc.send_device_query().await {
        Err(e) => c.line(&format!("ERROR: {e}")),
        Ok(info) => {
            let v = to_value(&info);
            if c.json {
                c.json_indent(&v);
            } else {
                let fw = vint(&v, "fw ver");
                c.emit("Device info :\n");
                if fw >= 3 {
                    c.emit(&format!(
                        " Model: {}\n Version: {}\n Comp. radio protocol: {fw}\n Build date: {}\n",
                        vstr(&v, "model"),
                        vstr(&v, "ver"),
                        vstr(&v, "fw_build")
                    ));
                    if let Some(repeat) = v.get("repeat") {
                        c.line(&format!(" Repeat: {}", on_off(repeat.as_bool())));
                    }
                } else {
                    c.line(&format!(" Firmware version : {fw}"));
                }
            }
        }
    }
    Ok(0)
}

/// `clock` (show the time) and `clock sync`.
pub async fn clock(c: &mut Ctx<'_>) -> CmdResult {
    if c.cmds.get(1).is_some_and(|a| a == "sync") {
        sync_time(c).await?;
        return Ok(1);
    }
    match c.app.mc.get_time().await {
        Err(e) => c.line(&format!("Error getting time: {e}")),
        Ok(ts) => {
            if c.json {
                c.json_indent(&json!({"time": ts}));
            } else {
                c.line(&format!(
                    "Current time : {} ({ts})",
                    format_local(i64::from(ts), "%Y-%m-%d %H:%M:%S")
                ));
            }
        }
    }
    Ok(0)
}

/// `sync_time` / `st` / `clock sync`: set the device clock to the host time.
pub async fn sync_time(c: &mut Ctx<'_>) -> Result<(), CmdError> {
    match c.app.mc.set_time(now_u32()).await {
        Err(meshcore::Error::Device(meshcore::ErrorCode::IllegalArg)) => {
            c.error(&json!({"ok": "No sync needed"}), "No time sync needed");
        }
        Err(e) => {
            let text = format!("Error syncing time: {e}");
            c.error(&json!({"error": "Error syncing time"}), &text);
        }
        Ok(()) => {
            if c.json {
                c.json_indent(&json!({"ok": "time synced"}));
            } else {
                c.line("Time synced");
            }
        }
    }
    Ok(())
}

/// `time <epoch>`: set the device clock.
pub async fn time(c: &mut Ctx<'_>) -> CmdResult {
    // int(value).to_bytes(4) in meshcore_py
    let epoch = crate::util::py::int(c.arg(1)?, 10).map_err(CmdError::Failed)?;
    let epoch = u32::try_from(epoch).map_err(|_| {
        CmdError::Failed(if epoch < 0 {
            "can't convert negative int to unsigned".to_string()
        } else {
            "int too big to convert".to_string()
        })
    })?;
    match c.app.mc.set_time(epoch).await {
        Err(e) => {
            let text = format!("Error setting time: {e}");
            c.error(&json!({"error": "Error setting time"}), &text);
        }
        Ok(()) => {
            if c.json {
                c.json_indent(&json!({}));
            } else {
                c.line("Time set");
            }
        }
    }
    Ok(1)
}

/// `reboot`.
pub async fn reboot(c: &mut Ctx<'_>) -> CmdResult {
    match c.app.mc.reboot().await {
        Ok(()) => {
            if c.json {
                c.json_indent(&json!({}));
            }
        }
        Err(e) => c.line(&format!("Error: {e}")),
    }
    Ok(0)
}

/// `self_telemetry` / `t`.
pub async fn self_telemetry(c: &mut Ctx<'_>) -> CmdResult {
    match c.app.mc.get_self_telemetry().await {
        // a timeout too: meshcore_py answers it with an error event, so the
        // reference never shows its own timeout message
        Err(_) => c.line("Error while requesting telemetry"),
        Ok(telemetry) => c.json_indent(&to_value(&telemetry)),
    }
    Ok(0)
}

/// `infos` / `i`: the node's own info as json.
pub async fn infos(c: &mut Ctx<'_>) -> CmdResult {
    let _ = c.app.mc.send_appstart().await;
    let info = c.app.self_info_value();
    c.json_indent(&info);
    Ok(0)
}

/// `advert` / `floodadv`.
pub async fn advert(c: &mut Ctx<'_>, flood: bool) -> CmdResult {
    match c.app.mc.send_advert(flood).await {
        Err(e) => c.line(&format!("Error sending advert: {e}")),
        Ok(()) => {
            if c.json {
                c.json_indent(&json!({}));
            } else {
                c.line("Advert sent");
            }
        }
    }
    Ok(0)
}

/// `cli <cmd>`: runs a command of the companion's own CLI.
pub async fn cli(c: &mut Ctx<'_>) -> CmdResult {
    let command = c.arg(1)?.to_string();
    match c.app.mc.run_cli_command(&command).await {
        Err(e) => c.line(&format!("Error sending cli command: {e}")),
        Ok(text) => {
            if c.json {
                c.json_line(&json!({"text": text}));
            } else {
                c.line(&text);
            }
        }
    }
    Ok(1)
}

/// `send_raw <hex>`: sends a raw packet.
pub async fn send_raw(c: &mut Ctx<'_>) -> CmdResult {
    let packet_hex = c.arg(1)?.to_string();
    let packet = crate::util::py::fromhex(&packet_hex).map_err(CmdError::Failed)?;
    if let Err(e) = c.app.mc.send_raw_packet(&packet).await {
        c.line(&format!("Error sending raw packet [{packet_hex}]: {e}"));
    }
    Ok(1)
}

/// `card`: this node's contact URI.
pub async fn card(c: &mut Ctx<'_>) -> CmdResult {
    match c.app.mc.export_contact(None).await {
        Err(e) => c.line(&format!("Error exporting contact: {e}")),
        Ok(uri) => {
            if c.json {
                c.json_line(&json!({"uri": uri}));
            } else {
                c.line(&uri);
            }
        }
    }
    Ok(0)
}

/// Resolves a `Location` header against the URL that returned it.
fn join_url(base: &str, location: &str) -> String {
    if location.contains("://") {
        return location.to_string();
    }
    let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
    if let Some(network_path) = location.strip_prefix("//") {
        return format!("{scheme}://{network_path}");
    }
    let authority = rest.split('/').next().unwrap_or_default();
    if location.starts_with('/') {
        return format!("{scheme}://{authority}{location}");
    }
    let path = rest[authority.len()..]
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let dir = path.rfind('/').map_or("/", |i| &path[..=i]);
    format!("{scheme}://{authority}{dir}{location}")
}

/// Posts JSON like `requests.post` does, redirects included: 307 and 308
/// repeat the POST, 301, 302 and 303 continue with GET (at most 30).
fn post_json(url: &str, body: &Value) -> Result<u16, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();
    // the body requests sends for json=...
    let body = crate::util::json::dumps(body);
    let mut url = url.to_string();
    let mut post = true;
    for _ in 0..=30 {
        let response = if post {
            agent
                .post(&url)
                .header("content-type", "application/json")
                .send(body.as_str())
        } else {
            agent.get(&url).call()
        }
        .map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|l| l.to_str().ok())
            .map(str::to_string);
        match (status, location) {
            (301 | 302 | 303 | 307 | 308, Some(location)) => {
                url = join_url(&url, &location);
                post = post && matches!(status, 307 | 308);
            }
            _ => return Ok(status),
        }
    }
    Err("Exceeded 30 redirects.".to_string())
}

/// Uploads a contact URI to the public map (`map.meshcore.dev`) and
/// returns the response like Python prints it (`<Response [200]>`).
///
/// # Errors
///
/// Fails when the request cannot be made at all.
pub async fn upload_uri(uri: String) -> Result<String, CmdError> {
    let body = json!({"links": [uri]});
    let result = tokio::task::spawn_blocking(move || {
        post_json("https://map.meshcore.dev/api/v1/nodes", &body)
    })
    .await
    .map_err(|e| CmdError::Failed(e.to_string()))?;
    let status = result.map_err(CmdError::Failed)?;
    Ok(format!("<Response [{status}]>"))
}

/// `upload_card`: uploads this node's URI to the map.
pub async fn upload_card(c: &mut Ctx<'_>) -> CmdResult {
    match c.app.mc.export_contact(None).await {
        Err(e) => c.line(&format!("Error exporting contact: {e}")),
        Ok(uri) => {
            let response = upload_uri(uri).await?;
            if c.json {
                c.json_line(&json!({"response": response}));
            } else {
                c.line(&response);
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_targets() {
        let base = "https://map.meshcore.dev/api/v1/nodes";
        assert_eq!(
            join_url(base, "https://map.meshcore.io/api/v1/nodes"),
            "https://map.meshcore.io/api/v1/nodes"
        );
        assert_eq!(
            join_url(base, "/v2/nodes"),
            "https://map.meshcore.dev/v2/nodes"
        );
        assert_eq!(join_url(base, "//other.host/x"), "https://other.host/x");
        assert_eq!(
            join_url(base, "items"),
            "https://map.meshcore.dev/api/v1/items"
        );
        assert_eq!(join_url("http://h", "x"), "http://h/x");
    }

    /// Serves one canned response per connection and records the requests.
    fn serve(responses: Vec<&'static str>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 4096];
                // read the head, then the body announced by Content-Length
                loop {
                    let n = stream.read(&mut buf).unwrap();
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request).to_string();
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let length = text[..head_end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= head_end + 4 + length || n == 0 {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8_lossy(&request).to_string());
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    #[test]
    fn post_repeats_body_on_307_like_requests() {
        let (url, server) = serve(vec![
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: /moved\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ]);
        let status = post_json(&format!("{url}/api/v1/nodes"), &json!({"links": ["x"]})).unwrap();
        assert_eq!(status, 404);
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /api/v1/nodes "));
        assert!(requests[1].starts_with("POST /moved "));
        assert!(requests[1].ends_with(r#"{"links": ["x"]}"#), "{requests:?}");
        assert!(requests[1].contains("content-type: application/json\r\n"));
    }

    #[test]
    fn post_turns_into_get_on_303() {
        let (url, server) = serve(vec![
            "HTTP/1.1 303 See Other\r\nLocation: /done\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ]);
        assert_eq!(post_json(&url, &json!({})).unwrap(), 200);
        let requests = server.join().unwrap();
        assert!(requests[1].starts_with("GET /done "));
    }
}
