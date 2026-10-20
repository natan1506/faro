//! Conversa com o daemon pelo socket Unix.
//!
//! Duas formas: pedidos avulsos (uma conexão por pedido) e uma inscrição
//! permanente que recebe cada alerta novo e reconecta sozinha se o daemon cair.

use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use vigia_proto::{encode, ClientMsg, ServerMsg, PROTOCOL_VERSION};

use crate::{on_alert, set_connected};

pub fn socket_path() -> String {
    std::env::var("VIGIA_SOCKET").unwrap_or_else(|_| vigia_proto::DEFAULT_SOCKET.to_string())
}

async fn connect() -> Result<UnixStream, String> {
    let path = socket_path();
    match tokio::time::timeout(Duration::from_secs(3), UnixStream::connect(&path)).await {
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => Err(format!(
            "o daemon do Faro não está respondendo em {path} ({e})"
        )),
        Err(_) => Err(format!(
            "o daemon do Faro demorou para responder em {path}"
        )),
    }
}

/// Manda um pedido e espera a resposta que `pick` aceitar. O daemon empurra
/// alertas em toda conexão, então mensagens que não são a resposta são puladas.
pub async fn request<T>(
    msg: &ClientMsg,
    mut pick: impl FnMut(ServerMsg) -> Option<T>,
) -> Result<T, String> {
    let stream = connect().await?;
    let (r, mut w) = stream.into_split();
    w.write_all(encode(msg).as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(r).lines();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let line = tokio::time::timeout_at(deadline, lines.next_line())
            .await
            .map_err(|_| "o daemon não respondeu a tempo".to_string())?
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "o daemon fechou a conexão".to_string())?;
        let reply: ServerMsg =
            serde_json::from_str(&line).map_err(|e| format!("resposta inválida do daemon: {e}"))?;
        if let ServerMsg::Error { message } = &reply {
            if !message.contains("atualizações perdidas") {
                return Err(message.clone());
            }
        }
        if let Some(x) = pick(reply) {
            return Ok(x);
        }
    }
}

/// Inscreve-se nos alertas e repassa cada um para as janelas.
pub async fn subscribe_loop(app: AppHandle) {
    loop {
        if let Err(e) = subscribe_once(&app).await {
            let _ = app.emit("vigia:log", e);
        }
        set_connected(&app, false);
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

async fn subscribe_once(app: &AppHandle) -> Result<(), String> {
    let stream = connect().await?;
    let (r, mut w) = stream.into_split();
    w.write_all(
        encode(&ClientMsg::Hello {
            version: PROTOCOL_VERSION,
        })
        .as_bytes(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(r).lines();
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        match serde_json::from_str::<ServerMsg>(&line) {
            Ok(ServerMsg::Welcome { .. }) => set_connected(app, true),
            Ok(ServerMsg::Alert { alert }) => on_alert(app, alert),
            Ok(ServerMsg::Status { status }) => {
                let _ = app.emit("vigia:status", status);
            }
            Ok(_) => {}
            Err(e) => return Err(format!("mensagem inválida do daemon: {e}")),
        }
    }
    Err("o daemon encerrou a conexão".into())
}
