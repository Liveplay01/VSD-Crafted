//! Minimaler obs-websocket-v5-Client: pro Aktion eine kurze Verbindung.
use crate::config::ObsConfig;
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

fn auth_string(password: &str, salt: &str, challenge: &str) -> String {
    let secret = B64.encode(Sha256::digest(format!("{password}{salt}")));
    B64.encode(Sha256::digest(format!("{secret}{challenge}")))
}

async fn next_json<S>(ws: &mut S) -> Result<Value>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match ws.next().await.context("OBS hat die Verbindung geschlossen")?? {
            Message::Text(t) => return Ok(serde_json::from_str(&t)?),
            Message::Close(_) => bail!("OBS hat die Verbindung geschlossen"),
            _ => {}
        }
    }
}

pub async fn request(cfg: &ObsConfig, request_type: &str, data: Option<&str>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), request_inner(cfg, request_type, data))
        .await
        .context("OBS antwortet nicht (Timeout)")?
}

async fn request_inner(cfg: &ObsConfig, request_type: &str, data: Option<&str>) -> Result<()> {
    let url = format!("ws://{}:{}", cfg.host, cfg.port);
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .context("OBS nicht erreichbar – läuft OBS mit aktiviertem WebSocket-Server?")?;

    let hello = next_json(&mut ws).await?;
    let mut identify = json!({"op": 1, "d": {"rpcVersion": 1}});
    if let Some(auth) = hello["d"].get("authentication") {
        let salt = auth["salt"].as_str().unwrap_or_default();
        let challenge = auth["challenge"].as_str().unwrap_or_default();
        identify["d"]["authentication"] = json!(auth_string(&cfg.password, salt, challenge));
    }
    ws.send(Message::Text(identify.to_string())).await?;
    let identified = next_json(&mut ws).await?;
    if identified["op"] != 2 {
        bail!("OBS-Anmeldung fehlgeschlagen (Passwort?)");
    }

    let mut d = json!({"requestType": request_type, "requestId": "n1deck"});
    if let Some(data) = data {
        d["requestData"] = serde_json::from_str(data).context("OBS-Daten sind kein gültiges JSON")?;
    }
    ws.send(Message::Text(json!({"op": 6, "d": d}).to_string())).await?;

    loop {
        let msg = next_json(&mut ws).await?;
        if msg["op"] == 7 {
            let status = &msg["d"]["requestStatus"];
            if status["result"] != true {
                bail!("OBS: {}", status["comment"].as_str().unwrap_or("Request abgelehnt"));
            }
            break;
        }
    }
    let _ = ws.close(None).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_matches_spec_shape() {
        // Nur Formtest: Base64 von 32 Byte SHA-256 = 44 Zeichen.
        assert_eq!(auth_string("pw", "salt", "challenge").len(), 44);
    }
}
