//! Socket Unix para o app. Só aceita o usuário configurado (e o root).

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};
use vigia_proto::{encode, AllowView, ClientMsg, ServerMsg, PROTOCOL_VERSION};

use crate::state::Shared;

pub fn bind(path: &str, owner_uid: u32) -> Result<UnixListener> {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = UnixListener::bind(path).with_context(|| format!("bind em {path}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let c = std::ffi::CString::new(path)?;
        // SAFETY: caminho válido terminado em NUL; gid -1 mantém o grupo.
        let rc = unsafe { libc::chown(c.as_ptr(), owner_uid, u32::MAX) };
        if rc != 0 {
            warn!(
                "não consegui passar o socket para o uid {owner_uid} (rodando sem root?): {}",
                std::io::Error::last_os_error()
            );
        }
    }
    Ok(listener)
}

pub async fn serve(shared: Arc<Shared>, listener: UnixListener) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                warn!("accept: {e}");
                continue;
            }
        };
        let allowed_uid = shared.cfg.read().unwrap().user.uid;
        let peer = stream.peer_cred().map(|c| c.uid()).ok();
        let me = unsafe { libc::geteuid() };
        if !matches!(peer, Some(u) if u == allowed_uid || u == 0 || u == me) {
            warn!("conexão recusada de uid {peer:?}");
            continue;
        }
        let shared = shared.clone();
        tokio::spawn(async move {
            if let Err(e) = client(shared, stream).await {
                info!("cliente desconectou: {e}");
            }
        });
    }
}

async fn client(shared: Arc<Shared>, stream: UnixStream) -> Result<()> {
    let (rd, mut wr) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::channel::<ServerMsg>(256);
    let mut events = shared.tx.subscribe();

    let writer = tokio::spawn(async move {
        loop {
            let msg = tokio::select! {
                m = out_rx.recv() => match m { Some(m) => m, None => break },
                e = events.recv() => match e {
                    Ok(m) => m,
                    Err(broadcast::error::RecvError::Lagged(n)) => ServerMsg::Error { message: format!("{n} atualizações perdidas; peça o histórico de novo") },
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            };
            if wr.write_all(encode(&msg).as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::new(rd).lines();
    while let Some(line) = lines.next_line().await? {
        let reply = match serde_json::from_str::<ClientMsg>(&line) {
            Ok(ClientMsg::Hello { .. }) => ServerMsg::Welcome {
                version: PROTOCOL_VERSION,
                status: shared.status(),
            },
            Ok(ClientMsg::History { limit }) => ServerMsg::History {
                alerts: shared.history(limit),
            },
            Ok(ClientMsg::Timeline { limit }) => ServerMsg::Timeline {
                entries: shared.timeline(limit.min(1000)),
            },
            Ok(ClientMsg::Report { report }) => {
                let (ok, message) = shared.report(report).await;
                ServerMsg::Reported { ok, message }
            }
            Ok(ClientMsg::Status) => ServerMsg::Status {
                status: shared.status(),
            },
            Ok(ClientMsg::Ping) => ServerMsg::Pong,
            Ok(ClientMsg::GetSettings) => ServerMsg::Settings {
                settings: shared.cfg.read().unwrap().settings(),
            },
            Ok(ClientMsg::SaveSettings { settings }) => shared.save_settings(settings).await,
            Ok(ClientMsg::Allowlist) => ServerMsg::Allowlist {
                entries: shared
                    .cfg
                    .read()
                    .unwrap()
                    .allow
                    .iter()
                    .map(|a| AllowView {
                        rule: a.rule.clone(),
                        signing_id: a.signing_id.clone(),
                        team_id: a.team_id.clone(),
                        path_prefix: a.path_prefix.clone(),
                        target: a.target.clone(),
                        comment: a.comment.clone(),
                    })
                    .collect(),
            },
            Ok(ClientMsg::Act { alert_id, action }) => {
                let (ok, message) = shared.act(alert_id, action).await;
                ServerMsg::ActionResult {
                    alert_id,
                    action,
                    ok,
                    message,
                }
            }
            Err(e) => ServerMsg::Error {
                message: format!("mensagem inválida: {e}"),
            },
        };
        if out_tx.send(reply).await.is_err() {
            break;
        }
    }
    writer.abort();
    Ok(())
}
