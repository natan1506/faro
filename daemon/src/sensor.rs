//! Fonte de eventos: o `eslogger` (ao vivo) ou um arquivo `.jsonl` (replay, para testes).
//! Linhas vão por uma fila limitada até a thread do motor de regras. Se o motor
//! não der conta, linhas são descartadas e contadas, em vez de travar o sensor.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use vigia_proto::SensorState;

use crate::state::Shared;

pub enum SensorMsg {
    Line(String),
    /// pid do eslogger, para o motor ignorar os eventos dele.
    SelfPid(i32),
}

pub async fn run_eslogger(shared: Arc<Shared>, tx: mpsc::Sender<SensorMsg>) {
    let mut backoff = 1u64;
    loop {
        let (bin, events) = {
            let c = shared.cfg.read().unwrap();
            (c.sensor.eslogger.clone(), c.sensor.events.clone())
        };
        info!("iniciando {bin} {}", events.join(" "));
        let child = Command::new(&bin)
            .args(&events)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let reason = format!("não consegui iniciar {bin}: {e}");
                error!("{reason}");
                shared.set_sensor(SensorState::Down {
                    reason: reason.clone(),
                });
                shared.sensor_alert("Sensor parado".into(), reason).await;
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
        };
        if let Some(pid) = child.id() {
            let _ = tx.send(SensorMsg::SelfPid(pid as i32)).await;
        }
        let stdout = child.stdout.take().expect("stdout com pipe");
        let stderr = child.stderr.take().expect("stderr com pipe");
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                warn!("eslogger: {l}");
            }
        });
        shared.set_sensor(SensorState::Running);
        let started = std::time::Instant::now();
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    shared.events_total.fetch_add(1, Ordering::Relaxed);
                    if tx.try_send(SensorMsg::Line(line)).is_err() {
                        shared.events_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    warn!("erro lendo eslogger: {e}");
                    break;
                }
            }
        }
        let status = child
            .wait()
            .await
            .map(|s| s.to_string())
            .unwrap_or_else(|e| e.to_string());
        let reason = format!("eslogger terminou ({status})");
        warn!("{reason}");
        shared.set_sensor(SensorState::Restarting {
            reason: reason.clone(),
        });
        // Caiu logo depois de subir: provavelmente falta Acesso Total ao Disco ou não é root.
        if started.elapsed() < Duration::from_secs(5) {
            shared
                .sensor_alert(
                    "Sensor não consegue rodar".into(),
                    format!("{reason}. Confira se o daemon roda como root e tem Acesso Total ao Disco (Ajustes > Privacidade e Segurança)."),
                )
                .await;
            backoff = (backoff * 2).min(60);
        } else {
            backoff = 1;
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
    }
}

/// Lê um arquivo de eventos gravados (um JSON por linha), sem descartar nada.
pub async fn run_replay(shared: Arc<Shared>, path: String, tx: mpsc::Sender<SensorMsg>) {
    shared.set_sensor(SensorState::Replay);
    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            shared.set_sensor(SensorState::Down {
                reason: format!("{path}: {e}"),
            });
            return;
        }
    };
    let mut lines = BufReader::new(file).lines();
    let mut n = 0u64;
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        shared.events_total.fetch_add(1, Ordering::Relaxed);
        if tx.send(SensorMsg::Line(line)).await.is_err() {
            break;
        }
        n += 1;
    }
    info!("replay de {path}: {n} eventos");
}
