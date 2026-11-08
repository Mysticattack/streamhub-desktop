//! Minimaler OBS-WebSocket-Client (v5) fuer Start/Stop direkt aus der App.

use std::time::Duration;
use tungstenite::{connect, Message};

use base64::Engine;
use sha2::{Digest, Sha256};

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn sha256_b64(input: &str) -> String {
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    b64(&h.finalize())
}

/// Schickt eine Anfrage an OBS (ws://127.0.0.1:4455) und gibt die Antwort zurueck.
pub fn obs_request(password: &str, request_type: &str) -> Result<String, String> {
    obs_request_json(password, request_type, serde_json::json!({}))
}

pub fn obs_request_json(
    password: &str,
    request_type: &str,
    request_data: serde_json::Value,
) -> Result<String, String> {
    let url = "ws://127.0.0.1:4455";
    let (mut socket, _) = connect(url).map_err(|e| format!("OBS nicht erreichbar: {}", e))?;
    let _ = Duration::from_secs(6);

    // 1) Hello
    let hello_txt = match socket.read() {
        Ok(Message::Text(t)) => t.to_string(),
        Ok(_) => return Err("unerwartete Nachricht von OBS".into()),
        Err(e) => return Err(format!("Handshake: {}", e)),
    };
    let hello: serde_json::Value =
        serde_json::from_str(&hello_txt).map_err(|e| format!("Hello: {}", e))?;

    // 2) Auth, falls verlangt
    let mut ident = serde_json::json!({"op": 1, "d": {"rpcVersion": 1}});
    if let Some(auth) = hello.get("d").and_then(|d| d.get("authentication")) {
        let salt = auth.get("salt").and_then(|v| v.as_str()).unwrap_or("");
        let challenge = auth.get("challenge").and_then(|v| v.as_str()).unwrap_or("");
        let secret = sha256_b64(&format!("{}{}", password, salt));
        let auth_string = sha256_b64(&format!("{}{}", secret, challenge));
        ident["d"]["authentication"] = serde_json::json!(auth_string);
    }
    socket
        .send(Message::Text(ident.to_string().into()))
        .map_err(|e| format!("Auth senden: {}", e))?;

    // Identified
    match socket.read() {
        Ok(Message::Text(t)) => {
            let v: serde_json::Value = serde_json::from_str(&t).unwrap_or_default();
            if v.get("op").and_then(|o| o.as_u64()) != Some(2) {
                return Err(format!("Anmeldung abgelehnt: {}", t));
            }
        }
        _ => return Err("keine Antwort auf die Anmeldung".into()),
    }

    // 3) Anfrage
    let req = serde_json::json!({
        "op": 6, "d": {"requestType": request_type, "requestId": "1", "requestData": request_data}
    });
    socket
        .send(Message::Text(req.to_string().into()))
        .map_err(|e| format!("Anfrage senden: {}", e))?;

    let resp = match socket.read() {
        Ok(Message::Text(t)) => t.to_string(),
        _ => return Err("keine Antwort".into()),
    };
    let _ = socket.close(None);
    let v: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
    let status = v
        .get("d")
        .and_then(|d| d.get("requestStatus"))
        .cloned()
        .unwrap_or_default();
    if status.get("result").and_then(|r| r.as_bool()).unwrap_or(false) {
        Ok(resp)
    } else {
        let comment = status
            .get("comment")
            .and_then(|c| c.as_str())
            .unwrap_or("unbekannter Fehler");
        Err(comment.to_string())
    }
}

/// Ist OBS gerade am Senden?
pub fn obs_is_streaming(password: &str) -> Option<bool> {
    let r = match obs_request(password, "GetStreamStatus") {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[obs] {}", e);
            return None;
        }
    };
    let v: serde_json::Value = serde_json::from_str(&r).ok()?;
    v.get("d")?.get("responseData")?.get("outputActive")?.as_bool()
}
