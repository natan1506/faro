//! Vigia: daemon de monitoramento em tempo real para macOS.
//!
//! Fluxo: eslogger -> fila -> motor de regras (thread própria) -> funil de alertas
//! -> SQLite + socket Unix (app no notch) + ntfy (celular).
//! Em paralelo: varredura de conexões (lsof) e de permissões (TCC).

mod actions;
mod config;
mod event;
mod net;
mod notify;
mod procinfo;
mod rules;
mod sensor;
mod server;
mod state;
mod store;
mod tcc;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tracing::{info, warn};
use vigia_proto::{encode, ClientMsg, ServerMsg, PROTOCOL_VERSION};

use crate::config::Config;
use crate::rules::{Engine, Finding};
use crate::sensor::SensorMsg;
use crate::state::Shared;
use crate::store::Store;

const DEFAULT_CONFIG: &str = "/Library/Application Support/Vigia/config.yaml";

#[derive(Parser)]
#[command(
    name = "faro-daemon",
    version,
    about = "Monitoramento em tempo real do Mac (Endpoint Security via eslogger)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Roda o daemon (padrão).
    Run {
        #[arg(long, default_value = DEFAULT_CONFIG)]
        config: PathBuf,
        /// Lê eventos de um arquivo .jsonl em vez do eslogger (para testes).
        #[arg(long)]
        replay: Option<String>,
        /// Sobrescreve o caminho do socket.
        #[arg(long)]
        socket: Option<String>,
        /// Sobrescreve o caminho do banco.
        #[arg(long)]
        db: Option<String>,
        /// Desliga as varreduras de rede e TCC (útil no replay).
        #[arg(long)]
        no_pollers: bool,
        /// Espera antes de começar o replay (dá tempo de o app conectar).
        #[arg(long, default_value_t = 0)]
        replay_delay: u64,
    },
    /// Valida a configuração e mostra o resumo.
    Check {
        #[arg(long, default_value = DEFAULT_CONFIG)]
        config: PathBuf,
    },
    /// Conecta no socket e imprime os alertas (sem precisar do app).
    Tail {
        #[arg(long, default_value = vigia_proto::DEFAULT_SOCKET)]
        socket: String,
        /// Mostra também os últimos N alertas.
        #[arg(long, default_value_t = 20)]
        history: u32,
    },
    /// Fase 0: roda as regras sobre uma gravação do eslogger e mostra volume,
    /// processos mais barulhentos e o que viraria alerta (para ajustar exceções).
    Analisar {
        #[arg(long, default_value = DEFAULT_CONFIG)]
        config: PathBuf,
        /// Arquivo .jsonl gravado com `eslogger exec open btm_launch_item_add`.
        arquivo: PathBuf,
    },
    /// Executa uma ação num alerta pelo terminal.
    Act {
        #[arg(long, default_value = vigia_proto::DEFAULT_SOCKET)]
        socket: String,
        #[arg(long)]
        id: i64,
        /// kill, kill_tree, quarantine, allow, dismiss
        #[arg(long)]
        action: String,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();
    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new()?;
    match cli.cmd.unwrap_or(Cmd::Run {
        config: DEFAULT_CONFIG.into(),
        replay: None,
        socket: None,
        db: None,
        no_pollers: false,
        replay_delay: 0,
    }) {
        Cmd::Run {
            config,
            replay,
            socket,
            db,
            no_pollers,
            replay_delay,
        } => rt.block_on(run(config, replay, socket, db, no_pollers, replay_delay)),
        Cmd::Check { config } => check(&config),
        Cmd::Analisar { config, arquivo } => analyze(&config, &arquivo),
        Cmd::Tail { socket, history } => rt.block_on(tail(&socket, history)),
        Cmd::Act { socket, id, action } => rt.block_on(act(&socket, id, &action)),
    }
}

fn check(path: &std::path::Path) -> Result<()> {
    let c = Config::load(path)?;
    let r = &c.rules;
    println!("config OK: {}", path.display());
    println!("usuário: {} (uid {})", c.user.home, c.user.uid);
    println!("repos de terceiros: {:?}", c.third_party_dirs);
    println!("regras ativas:");
    for (id, on, sev) in [
        (
            rules::SECRET_READ,
            r.secret_read.enabled,
            r.secret_read.severity,
        ),
        (
            rules::PERSISTENCE,
            r.persistence.enabled,
            r.persistence.severity,
        ),
        (
            rules::THIRD_PARTY_SHELL,
            r.third_party_shell.enabled,
            r.third_party_shell.severity,
        ),
        (
            rules::UNSIGNED_EXEC,
            r.unsigned_exec.enabled,
            r.unsigned_exec.severity,
        ),
        (
            rules::TCC_CHANGE,
            r.tcc_change.enabled,
            r.tcc_change.severity,
        ),
        (
            rules::NEW_CONNECTION,
            r.new_connection.enabled,
            r.new_connection.severity,
        ),
    ] {
        let _ = rules::KEYBOARD_TAP;
        println!(
            "  {:<18} {:<5} {}",
            id,
            if on { "on" } else { "off" },
            sev.label_pt()
        );
    }
    println!("exceções: {}", c.allow.len());
    println!(
        "ntfy: {}",
        if c.notify.ntfy.enabled {
            "ligado"
        } else {
            "desligado"
        }
    );
    Ok(())
}

fn analyze(config: &std::path::Path, file: &std::path::Path) -> Result<()> {
    use std::collections::HashMap;
    use std::io::BufRead;

    let cfg = Config::load(config)?;
    let mut engine = Engine::new(
        Arc::new(std::sync::RwLock::new(cfg)),
        Box::new(procinfo::SystemLookup),
    )?;
    let reader = std::io::BufReader::new(
        std::fs::File::open(file).with_context(|| format!("abrindo {}", file.display()))?,
    );
    let (mut total, mut passed) = (0u64, 0u64);
    let mut by_type: HashMap<String, u64> = HashMap::new();
    let mut opens_by_proc: HashMap<String, u64> = HashMap::new();
    let mut findings: HashMap<(String, String), (u64, String)> = HashMap::new();
    let mut unparsed: HashMap<String, (u64, String)> = HashMap::new();
    let (mut first, mut last): (Option<String>, Option<String>) = (None, None);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        total += 1;
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(t) = v.get("time").and_then(|t| t.as_str()) {
                first.get_or_insert_with(|| t.to_string());
                last = Some(t.to_string());
            }
            if let Some(kind) = v
                .get("event")
                .and_then(|e| e.as_object())
                .and_then(|e| e.keys().next())
            {
                *by_type.entry(kind.clone()).or_default() += 1;
                // Evento que o Vigia assina mas o parser não entendeu: sinal de que
                // a Apple mudou o formato do eslogger nesta versão do macOS.
                if matches!(kind.as_str(), "exec" | "open" | "btm_launch_item_add")
                    && matches!(event::parse_line(&line), None | Some(event::EsEvent::Other))
                {
                    let e = unparsed.entry(kind.clone()).or_insert((0, line.clone()));
                    e.0 += 1;
                }
                if kind == "open" {
                    if let Some(p) = v
                        .pointer("/process/executable/path")
                        .and_then(|p| p.as_str())
                    {
                        *opens_by_proc.entry(p.to_string()).or_default() += 1;
                    }
                }
            }
        }
        if engine.prefilter(&line) {
            passed += 1;
        }
        if let Some(f) = engine.on_line(&line) {
            let e = findings
                .entry((f.rule.to_string(), f.process.path.clone()))
                .or_insert((0, f.title.clone()));
            e.0 += 1;
        }
    }
    let secs = match (&first, &last) {
        (Some(a), Some(b)) => chrono::DateTime::parse_from_rfc3339(b)
            .ok()
            .zip(chrono::DateTime::parse_from_rfc3339(a).ok())
            .map(|(b, a)| (b - a).num_milliseconds() as f64 / 1000.0)
            .unwrap_or(0.0),
        _ => 0.0,
    };
    println!(
        "{total} eventos em {secs:.0} s ({:.0} por segundo)",
        if secs > 0.0 { total as f64 / secs } else { 0.0 }
    );
    let mut types: Vec<_> = by_type.into_iter().collect();
    types.sort_by_key(|t| std::cmp::Reverse(t.1));
    for (t, n) in types {
        println!("  {t:<22} {n}");
    }
    println!(
        "passaram pelo filtro barato: {passed} ({:.1}%); o resto foi descartado sem interpretar o JSON",
        if total > 0 { passed as f64 * 100.0 / total as f64 } else { 0.0 }
    );
    if unparsed.is_empty() {
        println!("formato do eslogger: todos os eventos exec/open/btm foram entendidos");
    } else {
        println!("\nATENÇÃO: eventos que o parser (event.rs) não entendeu; o formato do eslogger mudou:");
        for (kind, (n, example)) in &unparsed {
            let cut: String = example.chars().take(600).collect();
            println!("  {kind:<22} {n}x\n  ex.: {cut}");
        }
    }
    println!("\nprocessos que mais abrem arquivos:");
    let mut opens: Vec<_> = opens_by_proc.into_iter().collect();
    opens.sort_by_key(|o| std::cmp::Reverse(o.1));
    for (p, n) in opens.into_iter().take(10) {
        println!("  {n:>8}  {p}");
    }
    if findings.is_empty() {
        println!("\nnenhum alerta nesta gravação");
    } else {
        println!("\no que viraria alerta (regra, processo, vezes):");
        let mut f: Vec<_> = findings.into_iter().collect();
        f.sort_by_key(|x| std::cmp::Reverse(x.1 .0));
        for ((rule, proc), (n, title)) in f {
            println!(
                "  {rule:<18} {n:>5}x  {proc}\n  {:<18}        ex.: {title}",
                ""
            );
        }
        println!("\nSe algum desses for normal no seu dia a dia, crie uma exceção em allow.yaml (veja o README).");
    }
    Ok(())
}

async fn run(
    config: PathBuf,
    replay: Option<String>,
    socket: Option<String>,
    db: Option<String>,
    no_pollers: bool,
    replay_delay: u64,
) -> Result<()> {
    let mut cfg = Config::load(&config)?;
    if let Some(s) = socket {
        cfg.paths.socket = s;
    }
    if let Some(d) = db {
        cfg.paths.db = d;
    }
    let mut store =
        Store::open(&cfg.paths.db).with_context(|| format!("abrindo {}", cfg.paths.db))?;
    let learning_until =
        store.learning_started()? + chrono::Duration::hours(cfg.learning_hours as i64);
    let socket_path = cfg.paths.socket.clone();
    let uid = cfg.user.uid;
    let shared = Shared::new(cfg, store);
    *shared.learning_until.lock().unwrap() = Some(learning_until.to_rfc3339());
    info!(
        "Faro {} iniciando; aprendizado de rede até {learning_until}",
        env!("CARGO_PKG_VERSION")
    );

    let listener = server::bind(&socket_path, uid)?;
    tokio::spawn(server::serve(shared.clone(), listener));

    // Sensor -> motor (thread bloqueante própria) -> funil assíncrono.
    let queue = shared.cfg.read().unwrap().sensor.queue;
    let (line_tx, mut line_rx) = mpsc::channel::<SensorMsg>(queue);
    let (find_tx, mut find_rx) = mpsc::unbounded_channel::<Finding>();
    let mut engine = Engine::new(shared.cfg.clone(), Box::new(procinfo::SystemLookup))?
        .with_timeline(shared.timeline.clone());
    std::thread::Builder::new()
        .name("motor".into())
        .spawn(move || {
            while let Some(msg) = line_rx.blocking_recv() {
                match msg {
                    SensorMsg::SelfPid(p) => engine.ignore_pid(p),
                    SensorMsg::Line(l) => {
                        if let Some(f) = engine.on_line(&l) {
                            if find_tx.send(f).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        })?;
    {
        let shared = shared.clone();
        tokio::spawn(async move {
            while let Some(f) = find_rx.recv().await {
                shared.ingest(f).await;
            }
        });
    }

    match replay {
        Some(path) => {
            shared
                .simulated
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let shared = shared.clone();
            tokio::spawn(async move {
                if replay_delay > 0 {
                    info!("replay começa em {replay_delay} s");
                    tokio::time::sleep(Duration::from_secs(replay_delay)).await;
                }
                sensor::run_replay(shared, path, line_tx).await;
            });
        }
        None => {
            tokio::spawn(sensor::run_eslogger(shared.clone(), line_tx));
        }
    }

    if !no_pollers {
        tokio::spawn(net_loop(shared.clone()));
        tokio::spawn(tcc_loop(shared.clone()));
    }
    tokio::spawn(stats_loop(shared.clone()));
    tokio::spawn(housekeeping_loop(shared.clone()));

    shutdown_signal().await;
    info!("encerrando");
    let _ = std::fs::remove_file(&socket_path);
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM");
    let mut int = signal(SignalKind::interrupt()).expect("SIGINT");
    tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
}

async fn stats_loop(shared: Arc<Shared>) {
    let mut last = 0u64;
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tick.tick().await;
        let now = shared.events_total.load(Ordering::Relaxed);
        *shared.eps.lock().unwrap() = (now - last) as f64 / 5.0;
        last = now;
    }
}

async fn housekeeping_loop(shared: Arc<Shared>) {
    let mut tick = tokio::time::interval(Duration::from_secs(3600));
    loop {
        tick.tick().await;
        let days = shared.cfg.read().unwrap().retention_days;
        match shared.store.lock().unwrap().purge_older_than(days) {
            Ok(n) if n > 0 => info!("{n} alertas antigos apagados (retenção de {days} dias)"),
            Err(e) => warn!("limpeza falhou: {e}"),
            _ => {}
        }
    }
}

async fn net_loop(shared: Arc<Shared>) {
    let every = shared
        .cfg
        .read()
        .unwrap()
        .rules
        .new_connection
        .interval_secs
        .max(1);
    let mut tick = tokio::time::interval(Duration::from_secs(every));
    loop {
        tick.tick().await;
        // Lido a cada volta: a regra pode ser ligada ou desligada pelos Ajustes.
        if !shared.cfg.read().unwrap().rules.new_connection.enabled {
            continue;
        }
        let me = shared.clone();
        let findings = tokio::task::spawn_blocking(move || {
            let cfg = me.cfg.read().unwrap().clone();
            let learning = me
                .learning_until
                .lock()
                .unwrap()
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .is_some_and(|t| chrono::Utc::now() < t);
            let conns = net::parse_lsof(&net::run_lsof(&cfg.rules.new_connection.lsof));
            let mut store = me.store.lock().unwrap();
            net::scan(
                &cfg,
                &conns,
                learning,
                |k| store.net_mark_seen(k).unwrap_or(false),
                procinfo::proc_by_pid,
            )
        })
        .await
        .unwrap_or_default();
        for f in findings {
            shared.ingest(f).await;
        }
    }
}

async fn tcc_loop(shared: Arc<Shared>) {
    let every = shared
        .cfg
        .read()
        .unwrap()
        .rules
        .tcc_change
        .interval_secs
        .max(1);
    // Primeira passada só registra o que já existe, a não ser que o banco já tenha histórico.
    let mut baseline = shared
        .store
        .lock()
        .unwrap()
        .meta_get("tcc_baseline")
        .ok()
        .flatten()
        .is_none();
    let mut warned = false;
    let mut tick = tokio::time::interval(Duration::from_secs(every));
    loop {
        tick.tick().await;
        if !shared.cfg.read().unwrap().rules.tcc_change.enabled {
            continue;
        }
        let me = shared.clone();
        let base = baseline;
        let res = tokio::task::spawn_blocking(move || {
            let cfg = me.cfg.read().unwrap().clone();
            let grants =
                tcc::read_grants(&cfg.rules.tcc_change.db, &cfg.rules.tcc_change.services)?;
            let mut store = me.store.lock().unwrap();
            let f = tcc::diff(&cfg, &grants, base, |g| {
                store.tcc_mark_seen(&g.service, &g.client).unwrap_or(false)
            });
            if base {
                store.meta_set("tcc_baseline", &store::now())?;
            }
            Ok::<_, anyhow::Error>(f)
        })
        .await;
        match res {
            Ok(Ok(findings)) => {
                baseline = false;
                for f in findings {
                    shared.ingest(f).await;
                }
            }
            Ok(Err(e)) if !warned => {
                warned = true;
                warn!("não consegui ler o TCC.db (precisa de Acesso Total ao Disco): {e:#}");
            }
            _ => {}
        }
    }
}

async fn connect(socket: &str) -> Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(socket)
        .await
        .with_context(|| format!("conectando em {socket} (o daemon está rodando?)"))
}

async fn tail(socket: &str, history: u32) -> Result<()> {
    let s = connect(socket).await?;
    let (rd, mut wr) = s.into_split();
    wr.write_all(
        encode(&ClientMsg::Hello {
            version: PROTOCOL_VERSION,
        })
        .as_bytes(),
    )
    .await?;
    if history > 0 {
        wr.write_all(encode(&ClientMsg::History { limit: history }).as_bytes())
            .await?;
    }
    let mut lines = BufReader::new(rd).lines();
    while let Some(l) = lines.next_line().await? {
        match serde_json::from_str::<ServerMsg>(&l) {
            Ok(ServerMsg::Welcome { status, .. }) => println!(
                "conectado: sensor {:?}, {} eventos, {} alertas abertos",
                status.sensor, status.events_total, status.alerts_open
            ),
            Ok(ServerMsg::History { alerts }) => alerts.iter().rev().for_each(print_alert),
            Ok(ServerMsg::Alert { alert }) => print_alert(&alert),
            Ok(ServerMsg::Status { status }) => println!("sensor: {:?}", status.sensor),
            Ok(other) => println!("{other:?}"),
            Err(_) => println!("{l}"),
        }
    }
    Ok(())
}

fn print_alert(a: &vigia_proto::Alert) {
    let x = if a.count > 1 {
        format!(" x{}", a.count)
    } else {
        String::new()
    };
    println!(
        "#{} [{}] {:?} {}{} :: {}",
        a.id,
        a.severity.label_pt(),
        a.status,
        a.title,
        x,
        a.detail
    );
}

async fn act(socket: &str, id: i64, action: &str) -> Result<()> {
    let action: vigia_proto::ActionKind =
        serde_json::from_value(serde_json::Value::String(action.into()))
            .context("ação inválida")?;
    let s = connect(socket).await?;
    let (rd, mut wr) = s.into_split();
    wr.write_all(
        encode(&ClientMsg::Act {
            alert_id: id,
            action,
        })
        .as_bytes(),
    )
    .await?;
    let mut lines = BufReader::new(rd).lines();
    while let Some(l) = lines.next_line().await? {
        if let Ok(ServerMsg::ActionResult { ok, message, .. }) =
            serde_json::from_str::<ServerMsg>(&l)
        {
            println!("{} {message}", if ok { "ok:" } else { "falhou:" });
            return Ok(());
        }
    }
    Ok(())
}
