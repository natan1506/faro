//! Estado compartilhado do daemon e o funil por onde passam todos os alertas.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::broadcast;
use tracing::{info, warn};
use vigia_proto::{
    ActionKind, Alert, AlertStatus, ExecEntry, Report, SensorState, ServerMsg, Severity, Status,
};

use crate::config::Config;
use crate::rules::{Finding, Timeline, KEYBOARD_TAP};
use crate::store::{now, Store};
use crate::{actions, notify, procinfo};

pub struct Shared {
    pub cfg: Arc<RwLock<Config>>,
    pub store: Mutex<Store>,
    pub tx: broadcast::Sender<ServerMsg>,
    pub events_total: AtomicU64,
    pub events_dropped: AtomicU64,
    pub eps: Mutex<f64>,
    pub sensor: Mutex<SensorState>,
    pub started_at: String,
    pub learning_until: Mutex<Option<String>>,
    pub timeline: Timeline,
    /// Modo replay: alertas marcados como simulação e botões sem efeito no sistema.
    pub simulated: AtomicBool,
}

impl Shared {
    pub fn new(cfg: Config, store: Store) -> Arc<Self> {
        let (tx, _) = broadcast::channel(1024);
        Arc::new(Self {
            cfg: Arc::new(RwLock::new(cfg)),
            store: Mutex::new(store),
            tx,
            events_total: AtomicU64::new(0),
            events_dropped: AtomicU64::new(0),
            eps: Mutex::new(0.0),
            sensor: Mutex::new(SensorState::Starting),
            started_at: now(),
            learning_until: Mutex::new(None),
            timeline: Arc::new(Mutex::new(VecDeque::new())),
            simulated: AtomicBool::new(false),
        })
    }

    pub fn status(&self) -> Status {
        Status {
            version: env!("CARGO_PKG_VERSION").into(),
            started_at: self.started_at.clone(),
            sensor: self.sensor.lock().unwrap().clone(),
            events_total: self.events_total.load(Ordering::Relaxed),
            events_dropped: self.events_dropped.load(Ordering::Relaxed),
            events_per_sec: *self.eps.lock().unwrap(),
            alerts_open: self.store.lock().unwrap().open_count().unwrap_or(0),
            learning_until: self.learning_until.lock().unwrap().clone(),
        }
    }

    pub fn set_sensor(&self, s: SensorState) {
        *self.sensor.lock().unwrap() = s;
        let _ = self.tx.send(ServerMsg::Status {
            status: self.status(),
        });
    }

    /// Transforma um achado em alerta: dedup, cadeia de pais, gravação,
    /// envio para os apps conectados e, se for grave, para o celular.
    pub async fn ingest(self: &Arc<Self>, f: Finding) {
        let simulated = self.simulated.load(Ordering::Relaxed);
        self.ingest_as(f, simulated).await
    }

    async fn ingest_as(self: &Arc<Self>, f: Finding, simulated: bool) {
        let me = self.clone();
        let res = tokio::task::spawn_blocking(move || {
            let dedup = me.cfg.read().unwrap().dedup_secs;
            // Na simulação os pids são da gravação: perguntar ao `ps` traria processos reais
            // que só calham de ter o mesmo número.
            let pid = f.process.pid;
            let chain = if pid > 1 && !simulated {
                procinfo::chain(pid, 6)
            } else {
                vec![]
            };
            me.store
                .lock()
                .unwrap()
                .upsert_finding(&f, chain, dedup, simulated)
        })
        .await;
        let (alert, is_new) = match res {
            Ok(Ok(x)) => x,
            Ok(Err(e)) => return warn!("falha ao gravar alerta: {e:#}"),
            Err(e) => return warn!("tarefa de alerta falhou: {e}"),
        };
        if is_new {
            info!(id = alert.id, rule = %alert.rule, severity = ?alert.severity, "{}", alert.title);
        }
        let _ = self.tx.send(ServerMsg::Alert {
            alert: alert.clone(),
        });
        if is_new {
            let ntfy = self.cfg.read().unwrap().notify.ntfy.clone();
            if !alert.simulated && notify::should_send(&ntfy, &alert) {
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = notify::send(&ntfy, &alert) {
                        warn!("ntfy falhou: {e:#}");
                    }
                });
            }
        }
    }

    /// Executa a ação de um botão e devolve a mensagem para o app.
    pub async fn act(self: &Arc<Self>, alert_id: i64, action: ActionKind) -> (bool, String) {
        let me = self.clone();
        let res = tokio::task::spawn_blocking(move || {
            let alert = me
                .store
                .lock()
                .unwrap()
                .get(alert_id)
                .ok()
                .flatten()
                .ok_or_else(|| "alerta não encontrado".to_string())?;
            if alert.simulated {
                actions::check(&alert, action)?;
                // Nunca mexer no sistema por causa de um evento gravado: os pids e caminhos
                // são da gravação e podem coincidir com coisas reais suas.
                let (status, msg) = simulated_outcome(action);
                let updated = me
                    .store
                    .lock()
                    .unwrap()
                    .set_status(alert_id, status, Some(msg.clone()))
                    .ok()
                    .flatten();
                return Ok((updated, msg, true));
            }
            let outcome = {
                let mut cfg = me.cfg.write().unwrap();
                actions::perform(&mut cfg, &alert, action)
            };
            let (status, msg, ok) = match outcome {
                Ok((st, m)) => (st, m, true),
                Err(e)
                    if action == ActionKind::Kill
                        || action == ActionKind::KillTree
                        || action == ActionKind::Quarantine =>
                {
                    (AlertStatus::Failed, e, false)
                }
                Err(e) => return Err(e),
            };
            let updated = me
                .store
                .lock()
                .unwrap()
                .set_status(alert_id, status, Some(msg.clone()))
                .ok()
                .flatten();
            Ok::<_, String>((updated, msg, ok))
        })
        .await;
        match res {
            Ok(Ok((updated, msg, ok))) => {
                if let Some(a) = updated {
                    info!(id = a.id, ?action, ok, "{msg}");
                    let _ = self.tx.send(ServerMsg::Alert { alert: a });
                }
                (ok, msg)
            }
            Ok(Err(e)) => (false, e.to_string()),
            Err(e) => (false, format!("falha interna: {e}")),
        }
    }

    pub async fn sensor_alert(self: &Arc<Self>, title: String, detail: String) {
        self.ingest(Finding {
            rule: "sensor",
            severity: Severity::High,
            title,
            detail,
            target: None,
            process: Default::default(),
            actions: vec![ActionKind::Dismiss],
            dedup_key: "sensor|down".into(),
        })
        .await;
    }

    pub fn timeline(&self, limit: u32) -> Vec<ExecEntry> {
        let t = self.timeline.lock().unwrap();
        t.iter().rev().take(limit as usize).cloned().collect()
    }

    /// Achado vindo do app (hoje só leitores de teclado).
    pub async fn report(self: &Arc<Self>, r: Report) -> (bool, String) {
        if r.rule != KEYBOARD_TAP {
            return (
                false,
                format!("regra {} não pode ser reportada pelo app", r.rule),
            );
        }
        if self
            .cfg
            .read()
            .unwrap()
            .is_allowed(KEYBOARD_TAP, &r.process, r.target.as_deref())
        {
            return (true, "permitido por exceção".into());
        }
        let key = format!("{KEYBOARD_TAP}|{}", r.process.path);
        let mut actions = vec![];
        if r.process.pid > 1 {
            actions.extend([ActionKind::KillTree, ActionKind::Kill]);
        }
        actions.extend([
            ActionKind::OpenSettings,
            ActionKind::Allow,
            ActionKind::Dismiss,
        ]);
        // O leitor de teclado vem do app, que olha o Mac de verdade mesmo no replay.
        self.ingest_as(
            Finding {
                rule: KEYBOARD_TAP,
                severity: r.severity,
                title: r.title,
                detail: r.detail,
                target: r.target,
                process: r.process,
                actions,
                dedup_key: key,
            },
            false,
        )
        .await;
        (true, "registrado".into())
    }

    pub async fn save_settings(self: &Arc<Self>, s: vigia_proto::Settings) -> ServerMsg {
        let me = self.clone();
        let res = tokio::task::spawn_blocking(move || {
            let mut cfg = me.cfg.write().unwrap();
            let r = cfg.save_settings(&s, |p| std::path::Path::new(p).is_dir());
            (r, cfg.settings())
        })
        .await;
        match res {
            Ok((Ok(()), settings)) => {
                info!("ajustes salvos pelo app");
                ServerMsg::SettingsSaved {
                    ok: true,
                    message: "ajustes salvos".into(),
                    settings,
                }
            }
            Ok((Err(message), settings)) => ServerMsg::SettingsSaved {
                ok: false,
                message,
                settings,
            },
            Err(e) => ServerMsg::Error {
                message: format!("falha interna: {e}"),
            },
        }
    }

    pub fn history(&self, limit: u32) -> Vec<Alert> {
        self.store
            .lock()
            .unwrap()
            .history(limit.min(1000))
            .unwrap_or_default()
    }
}

/// O que o botão faria, sem fazer: o alerta só muda de estado no painel.
fn simulated_outcome(action: ActionKind) -> (AlertStatus, String) {
    let status = match action {
        ActionKind::Kill | ActionKind::KillTree => AlertStatus::Killed,
        ActionKind::Quarantine => AlertStatus::Quarantined,
        ActionKind::Allow => AlertStatus::Allowed,
        _ => AlertStatus::Dismissed,
    };
    (status, "simulação: nada foi executado no seu Mac".into())
}
