//! Ações disparadas pelos botões do app: matar, quarentena, permitir, ignorar.

use std::path::{Path, PathBuf};
use std::process::Command;

use vigia_proto::{ActionKind, Alert, AlertStatus};

use crate::config::{AllowEntry, Config};
use crate::procinfo;
use crate::rules::{NEW_CONNECTION, PERSISTENCE};

pub type ActionOutcome = Result<(AlertStatus, String), String>;

/// A ação pode ser pedida para este alerta agora?
pub fn check(alert: &Alert, action: ActionKind) -> Result<(), String> {
    if action.is_client_side() {
        return Err("essa ação é feita pelo app, não pelo daemon".into());
    }
    if !alert.actions.contains(&action) {
        return Err(format!(
            "ação {action:?} não está disponível para este alerta"
        ));
    }
    if alert.status != AlertStatus::Open {
        return Err("o alerta já foi resolvido".into());
    }
    Ok(())
}

pub fn perform(cfg: &mut Config, alert: &Alert, action: ActionKind) -> ActionOutcome {
    check(alert, action)?;
    match action {
        ActionKind::Kill => {
            procinfo::kill(alert.process.pid)?;
            Ok((
                AlertStatus::Killed,
                format!(
                    "{} (pid {}) encerrado",
                    alert.process.name(),
                    alert.process.pid
                ),
            ))
        }
        ActionKind::KillTree => {
            let n = procinfo::kill_tree(alert.process.pid)?;
            Ok((
                AlertStatus::Killed,
                format!(
                    "{n} processo(s) encerrado(s) a partir do pid {}",
                    alert.process.pid
                ),
            ))
        }
        ActionKind::Quarantine => quarantine(cfg, alert),
        ActionKind::Allow => {
            let entry = allow_entry_for(alert);
            let msg = describe_allow(&entry);
            cfg.append_allow(entry)
                .map_err(|e| format!("não consegui gravar a exceção: {e}"))?;
            Ok((AlertStatus::Allowed, msg))
        }
        ActionKind::Dismiss => Ok((AlertStatus::Dismissed, "ignorado".into())),
        ActionKind::Reveal | ActionKind::OpenSettings => unreachable!(),
    }
}

/// A exceção mais estreita possível: o assinante (se houver time de desenvolvedor)
/// ou o caminho exato do executável, mais o destino na regra de rede.
pub fn allow_entry_for(a: &Alert) -> AllowEntry {
    let p = &a.process;
    // Com time de desenvolvedor: identificador + time (sobrevive a atualizações do app).
    // Sem time (ad-hoc ou sem assinatura): só o caminho exato, que não dá para forjar
    // sem escrever naquele lugar. Bundle id sem caminho (alertas de TCC) fica como identificador.
    let (signing_id, team_id, path_prefix) = match (&p.signing_id, &p.team_id) {
        (Some(sid), Some(team)) => (Some(sid.clone()), Some(team.clone()), None),
        _ if p.path.starts_with('/') => (None, None, Some(p.path.clone())),
        (Some(sid), None) => (Some(sid.clone()), None, None),
        _ => (None, None, Some(p.path.clone())),
    };
    let target = if a.rule == NEW_CONNECTION {
        a.target.clone()
    } else {
        None
    };
    AllowEntry {
        rule: a.rule.clone(),
        signing_id,
        team_id,
        path_prefix,
        target,
        comment: Some(format!(
            "permitido pelo app em {} (alerta #{})",
            crate::store::now(),
            a.id
        )),
    }
}

fn describe_allow(e: &AllowEntry) -> String {
    let who = e
        .signing_id
        .clone()
        .or(e.path_prefix.clone())
        .unwrap_or_default();
    match &e.target {
        Some(t) => format!("não vou mais alertar {} para {who} em {t}", e.rule),
        None => format!("não vou mais alertar {} para {who}", e.rule),
    }
}

const PROTECTED_PREFIXES: &[&str] = &[
    "/System/",
    "/bin/",
    "/sbin/",
    "/usr/bin/",
    "/usr/sbin/",
    "/usr/lib/",
    "/usr/libexec/",
];

fn quarantine(cfg: &Config, a: &Alert) -> ActionOutcome {
    let target = a
        .target
        .clone()
        .ok_or("o alerta não tem arquivo para pôr em quarentena")?;
    if PROTECTED_PREFIXES.iter().any(|p| target.starts_with(p)) {
        return Err(format!("{target} é um arquivo do sistema; não vou movê-lo"));
    }
    let mut notes = Vec::new();

    if a.rule == PERSISTENCE && target.ends_with(".plist") {
        // Descarrega o item antes de mover, senão o launchd continua com ele na memória.
        let domain = if target.contains("/LaunchDaemons/") {
            "system".to_string()
        } else {
            format!("gui/{}", cfg.user.uid)
        };
        let out = Command::new("/bin/launchctl")
            .args(["bootout", &domain, &target])
            .output();
        match out {
            Ok(o) if o.status.success() => {
                notes.push(format!("descarregado do launchd ({domain})"))
            }
            _ => notes
                .push("launchctl bootout não confirmou (o item pode não estar carregado)".into()),
        }
    }

    if a.process.pid > 1 && a.actions.contains(&ActionKind::KillTree) {
        match procinfo::kill_tree(a.process.pid) {
            Ok(n) => notes.push(format!("{n} processo(s) encerrado(s)")),
            Err(e) => notes.push(e),
        }
    }

    let src = PathBuf::from(&target);
    if !src.exists() {
        return Err(format!("{target} não existe mais. {}", notes.join("; ")));
    }
    let dest = move_to_quarantine(&src, Path::new(&cfg.paths.quarantine_dir), a.id)
        .map_err(|e| format!("não consegui mover {target}: {e}"))?;
    notes.push(format!("arquivo movido para {}", dest.display()));
    Ok((AlertStatus::Quarantined, notes.join("; ")))
}

/// Move o arquivo, tira todas as permissões e grava de onde ele veio.
pub fn move_to_quarantine(src: &Path, qdir: &Path, alert_id: i64) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(qdir)?;
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "arquivo".into());
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let dest = qdir.join(format!("{stamp}-{alert_id}-{name}"));
    if std::fs::rename(src, &dest).is_err() {
        std::fs::copy(src, &dest)?;
        std::fs::remove_file(src)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o000))?;
    }
    let meta = serde_json::json!({ "original_path": src, "alert_id": alert_id, "quarantined_at": crate::store::now() });
    std::fs::write(
        dest.with_extension("vigia.json"),
        serde_json::to_vec_pretty(&meta).unwrap_or_default(),
    )?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vigia_proto::{ProcInfo, Severity};

    fn alert(
        rule: &str,
        process: ProcInfo,
        target: Option<&str>,
        actions: Vec<ActionKind>,
    ) -> Alert {
        Alert {
            id: 9,
            ts: "t".into(),
            first_ts: "t".into(),
            rule: rule.into(),
            severity: Severity::High,
            title: "x".into(),
            detail: "x".into(),
            target: target.map(String::from),
            process,
            chain: vec![],
            actions,
            count: 1,
            status: AlertStatus::Open,
            status_message: None,
            simulated: false,
        }
    }

    fn cfg(dir: &Path) -> Config {
        let mut c = Config::from_yaml("user: { home: /Users/dev, uid: 501 }").unwrap();
        c.paths.allow_file = dir.join("allow.yaml").to_string_lossy().into();
        c.paths.quarantine_dir = dir.join("q").to_string_lossy().into();
        c
    }

    #[test]
    fn allow_prefers_team_signed_identity_then_exact_path() {
        let signed = ProcInfo {
            path: "/Applications/X.app/x".into(),
            signing_id: Some("com.x".into()),
            team_id: Some("ABC".into()),
            ..Default::default()
        };
        let e = allow_entry_for(&alert("secret_read", signed, None, vec![]));
        assert_eq!(e.signing_id.as_deref(), Some("com.x"));
        assert_eq!(e.team_id.as_deref(), Some("ABC"));
        assert!(e.path_prefix.is_none());

        let adhoc = ProcInfo {
            path: "/usr/local/bin/node".into(),
            signing_id: Some("node".into()),
            ..Default::default()
        };
        let e = allow_entry_for(&alert("new_connection", adhoc, Some("1.2.3.0/24"), vec![]));
        assert_eq!(
            e.path_prefix.as_deref(),
            Some("/usr/local/bin/node"),
            "assinatura ad-hoc pode ser forjada; usa o caminho"
        );
        assert_eq!(e.target.as_deref(), Some("1.2.3.0/24"));
    }

    #[test]
    fn allow_writes_exception_and_closes_alert() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cfg(d.path());
        let a = alert(
            "secret_read",
            ProcInfo {
                path: "/opt/tool".into(),
                ..Default::default()
            },
            None,
            vec![ActionKind::Allow],
        );
        let (st, _) = perform(&mut c, &a, ActionKind::Allow).unwrap();
        assert_eq!(st, AlertStatus::Allowed);
        assert!(c.is_allowed(
            "secret_read",
            &ProcInfo {
                path: "/opt/tool".into(),
                ..Default::default()
            },
            None
        ));
    }

    #[test]
    fn rejects_unavailable_or_client_side_actions() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cfg(d.path());
        let a = alert(
            "secret_read",
            ProcInfo::default(),
            None,
            vec![ActionKind::Dismiss],
        );
        assert!(perform(&mut c, &a, ActionKind::Kill).is_err());
        assert!(perform(&mut c, &a, ActionKind::Reveal).is_err());
    }

    #[test]
    fn quarantine_moves_and_locks_file() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cfg(d.path());
        let bin = d.path().join("agent");
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();
        let a = alert(
            "unsigned_exec",
            ProcInfo {
                pid: 0,
                path: bin.to_string_lossy().into(),
                ..Default::default()
            },
            Some(bin.to_str().unwrap()),
            vec![ActionKind::Quarantine],
        );
        let (st, msg) = perform(&mut c, &a, ActionKind::Quarantine).unwrap();
        assert_eq!(st, AlertStatus::Quarantined);
        assert!(!bin.exists());
        assert!(msg.contains("movido"));
        let moved: Vec<_> = std::fs::read_dir(d.path().join("q")).unwrap().collect();
        assert_eq!(moved.len(), 2, "arquivo + metadados");
    }

    #[test]
    fn quarantine_refuses_system_files() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cfg(d.path());
        let a = alert(
            "unsigned_exec",
            ProcInfo::default(),
            Some("/usr/bin/curl"),
            vec![ActionKind::Quarantine],
        );
        assert!(perform(&mut c, &a, ActionKind::Quarantine)
            .unwrap_err()
            .contains("sistema"));
    }
}
