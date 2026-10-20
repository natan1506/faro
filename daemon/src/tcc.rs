//! Regra de keylogger: observa o banco de permissões do macOS (TCC) e alerta
//! quando um app novo ganha Monitoramento de Entrada, Acessibilidade, envio de
//! eventos ou gravação de tela. Desde o Catalina, um keylogger precisa de
//! Monitoramento de Entrada para ler o teclado, então essa permissão é o rastro.

use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use vigia_proto::{ActionKind, ProcInfo};

use crate::config::Config;
use crate::rules::{Finding, TCC_CHANGE};

#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    pub service: String,
    pub client: String,
}

pub fn read_grants(db: &str, services: &[String]) -> Result<Vec<Grant>> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let placeholders = vec!["?"; services.len()].join(",");
    let sql = format!(
        "SELECT service, client FROM access WHERE auth_value = 2 AND service IN ({placeholders})"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(rusqlite::params_from_iter(services.iter()), |r| {
        Ok(Grant {
            service: r.get(0)?,
            client: r.get(1)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn service_label(s: &str) -> &str {
    match s {
        "kTCCServiceListenEvent" => "Monitoramento de Entrada (pode ler o teclado)",
        "kTCCServiceAccessibility" => "Acessibilidade (pode controlar o Mac)",
        "kTCCServicePostEvent" => "Envio de eventos (pode digitar e clicar)",
        "kTCCServiceScreenCapture" => "Gravação de tela",
        other => other,
    }
}

/// Na primeira passada (`baseline`), só registra o que já existe.
pub fn diff(
    cfg: &Config,
    grants: &[Grant],
    baseline: bool,
    mut is_new: impl FnMut(&Grant) -> bool,
) -> Vec<Finding> {
    let r = &cfg.rules.tcc_change;
    let mut out = Vec::new();
    for g in grants {
        if !is_new(g) || baseline {
            continue;
        }
        let proc = if g.client.starts_with('/') {
            ProcInfo {
                path: g.client.clone(),
                ..Default::default()
            }
        } else {
            ProcInfo {
                path: g.client.clone(),
                signing_id: Some(g.client.clone()),
                ..Default::default()
            }
        };
        if cfg.is_allowed(TCC_CHANGE, &proc, Some(&g.service)) {
            continue;
        }
        let label = service_label(&g.service);
        out.push(Finding {
            rule: TCC_CHANGE,
            severity: r.severity,
            title: format!(
                "{} ganhou {}",
                short(&g.client),
                label.split(" (").next().unwrap_or(label)
            ),
            detail: format!(
                "{} recebeu a permissão {}. Se não foi você que deu, remova em Ajustes.",
                g.client, label
            ),
            target: Some(g.service.clone()),
            dedup_key: format!("{TCC_CHANGE}|{}|{}", g.service, g.client),
            actions: vec![
                ActionKind::OpenSettings,
                ActionKind::Allow,
                ActionKind::Dismiss,
            ],
            process: proc,
        });
    }
    out
}

/// Caminho vira o nome do arquivo; bundle id fica inteiro.
fn short(client: &str) -> &str {
    if client.starts_with('/') {
        client.rsplit('/').next().unwrap_or(client)
    } else {
        client
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn fake_db(path: &std::path::Path, rows: &[(&str, &str, i32)]) {
        let c = Connection::open(path).unwrap();
        c.execute_batch("CREATE TABLE access (service TEXT, client TEXT, client_type INTEGER, auth_value INTEGER, auth_reason INTEGER, last_modified INTEGER)").unwrap();
        for (s, cl, v) in rows {
            c.execute(
                "INSERT INTO access VALUES (?1, ?2, 0, ?3, 0, 0)",
                rusqlite::params![s, cl, v],
            )
            .unwrap();
        }
    }

    #[test]
    fn baseline_then_new_grant_alerts() {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("TCC.db");
        fake_db(
            &db,
            &[
                ("kTCCServiceListenEvent", "com.raycast.macos", 2),
                ("kTCCServiceCamera", "us.zoom.xos", 2),
            ],
        );
        let cfg = Config::from_yaml("user: { home: /Users/dev, uid: 501 }").unwrap();
        let services = &cfg.rules.tcc_change.services;
        let mut seen = HashSet::new();

        let g = read_grants(db.to_str().unwrap(), services).unwrap();
        assert_eq!(g.len(), 1, "câmera não é observada");
        assert!(diff(&cfg, &g, true, |x| seen
            .insert((x.service.clone(), x.client.clone())))
        .is_empty());

        let c = Connection::open(&db).unwrap();
        c.execute("INSERT INTO access VALUES ('kTCCServiceListenEvent', '/Users/dev/.cache/twh/agent', 1, 2, 0, 0)", []).unwrap();
        c.execute(
            "INSERT INTO access VALUES ('kTCCServiceAccessibility', 'com.denied', 0, 0, 0, 0)",
            [],
        )
        .unwrap();
        let g = read_grants(db.to_str().unwrap(), services).unwrap();
        let f = diff(&cfg, &g, false, |x| {
            seen.insert((x.service.clone(), x.client.clone()))
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].title, "agent ganhou Monitoramento de Entrada");
        assert_eq!(f[0].process.path, "/Users/dev/.cache/twh/agent");
    }
}
