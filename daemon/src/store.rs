//! Persistência em SQLite: alertas (com retenção), conexões já vistas e
//! permissões TCC já vistas.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use vigia_proto::{Alert, AlertStatus, ProcInfo};

use crate::rules::Finding;

pub struct Store {
    conn: Connection,
}

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Store {
    pub fn open(path: &str) -> Result<Self> {
        if let Some(dir) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            CREATE TABLE IF NOT EXISTS alerts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT NOT NULL,
                first_ts TEXT NOT NULL,
                rule TEXT NOT NULL,
                severity TEXT NOT NULL,
                dedup_key TEXT NOT NULL,
                status TEXT NOT NULL,
                count INTEGER NOT NULL DEFAULT 1,
                body TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS alerts_dedup ON alerts(dedup_key, ts);
            CREATE INDEX IF NOT EXISTS alerts_ts ON alerts(ts);
            CREATE TABLE IF NOT EXISTS net_seen (key TEXT PRIMARY KEY, first_seen TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS tcc_seen (service TEXT NOT NULL, client TEXT NOT NULL, first_seen TEXT NOT NULL, PRIMARY KEY (service, client));
            CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            "#,
        )?;
        Ok(Self { conn })
    }

    /// Cria um alerta ou, se houver um aberto com a mesma chave dentro da janela,
    /// só incrementa o contador. Devolve o alerta atual e se ele é novo.
    pub fn upsert_finding(
        &mut self,
        f: &Finding,
        chain: Vec<ProcInfo>,
        dedup_secs: i64,
        simulated: bool,
    ) -> Result<(Alert, bool)> {
        let ts = now();
        let since = (Utc::now() - Duration::seconds(dedup_secs))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let existing: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, body FROM alerts WHERE dedup_key = ?1 AND status = 'open' AND ts >= ?2 ORDER BY id DESC LIMIT 1",
                params![f.dedup_key, since],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, body)) = existing {
            let mut a: Alert = serde_json::from_str(&body)?;
            a.count += 1;
            a.ts = ts.clone();
            self.conn.execute(
                "UPDATE alerts SET ts = ?1, count = ?2, body = ?3 WHERE id = ?4",
                params![ts, a.count, serde_json::to_string(&a)?, id],
            )?;
            return Ok((a, false));
        }
        let mut a = Alert {
            id: 0,
            ts: ts.clone(),
            first_ts: ts.clone(),
            rule: f.rule.to_string(),
            severity: f.severity,
            title: f.title.clone(),
            detail: f.detail.clone(),
            target: f.target.clone(),
            process: f.process.clone(),
            chain,
            actions: f.actions.clone(),
            count: 1,
            status: AlertStatus::Open,
            status_message: None,
            simulated,
        };
        self.conn.execute(
            "INSERT INTO alerts (ts, first_ts, rule, severity, dedup_key, status, count, body) VALUES (?1, ?1, ?2, ?3, ?4, 'open', 1, '')",
            params![ts, f.rule, serde_json::to_string(&f.severity)?, f.dedup_key],
        )?;
        a.id = self.conn.last_insert_rowid();
        self.conn.execute(
            "UPDATE alerts SET body = ?1 WHERE id = ?2",
            params![serde_json::to_string(&a)?, a.id],
        )?;
        Ok((a, true))
    }

    pub fn get(&self, id: i64) -> Result<Option<Alert>> {
        let body: Option<String> = self
            .conn
            .query_row("SELECT body FROM alerts WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        Ok(match body {
            Some(b) => Some(serde_json::from_str(&b)?),
            None => None,
        })
    }

    pub fn set_status(
        &mut self,
        id: i64,
        status: AlertStatus,
        message: Option<String>,
    ) -> Result<Option<Alert>> {
        let Some(mut a) = self.get(id)? else {
            return Ok(None);
        };
        a.status = status;
        a.status_message = message;
        let st = serde_json::to_value(status)?
            .as_str()
            .unwrap_or("open")
            .to_string();
        self.conn.execute(
            "UPDATE alerts SET status = ?1, body = ?2 WHERE id = ?3",
            params![st, serde_json::to_string(&a)?, id],
        )?;
        Ok(Some(a))
    }

    pub fn history(&self, limit: u32) -> Result<Vec<Alert>> {
        let mut st = self
            .conn
            .prepare("SELECT body FROM alerts ORDER BY id DESC LIMIT ?1")?;
        let rows = st.query_map([limit], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(serde_json::from_str(&r?)?);
        }
        Ok(out)
    }

    pub fn open_count(&self) -> Result<u32> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM alerts WHERE status = 'open'",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn purge_older_than(&mut self, days: u32) -> Result<usize> {
        let cutoff = (Utc::now() - Duration::days(days as i64))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        Ok(self
            .conn
            .execute("DELETE FROM alerts WHERE ts < ?1", [cutoff])?)
    }

    /// Marca a chave como vista. Devolve `true` se ela é nova.
    pub fn net_mark_seen(&mut self, key: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "INSERT OR IGNORE INTO net_seen (key, first_seen) VALUES (?1, ?2)",
            params![key, now()],
        )? == 1)
    }

    pub fn tcc_mark_seen(&mut self, service: &str, client: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "INSERT OR IGNORE INTO tcc_seen (service, client, first_seen) VALUES (?1, ?2, ?3)",
            params![service, client, now()],
        )? == 1)
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn meta_set(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Momento em que o aprendizado da regra de rede começou (gravado na primeira vez).
    pub fn learning_started(&mut self) -> Result<DateTime<Utc>> {
        if let Some(v) = self.meta_get("learning_started")? {
            if let Ok(t) = DateTime::parse_from_rfc3339(&v) {
                return Ok(t.with_timezone(&Utc));
            }
        }
        let t = Utc::now();
        self.meta_set("learning_started", &t.to_rfc3339())?;
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vigia_proto::{ActionKind, Severity};

    fn finding(key: &str) -> Finding {
        Finding {
            rule: "secret_read",
            severity: Severity::Critical,
            title: "node leu .env".into(),
            detail: "d".into(),
            target: Some("/x/.env".into()),
            process: ProcInfo {
                pid: 1,
                path: "/usr/local/bin/node".into(),
                ..Default::default()
            },
            actions: vec![ActionKind::Kill],
            dedup_key: key.into(),
        }
    }

    #[test]
    fn dedups_within_window() {
        let mut s = Store::in_memory().unwrap();
        let (a, new) = s.upsert_finding(&finding("k"), vec![], 60, false).unwrap();
        assert!(new);
        let (b, new2) = s.upsert_finding(&finding("k"), vec![], 60, false).unwrap();
        assert!(!new2);
        assert_eq!(a.id, b.id);
        assert_eq!(b.count, 2);
        let (c, new3) = s
            .upsert_finding(&finding("outra"), vec![], 60, false)
            .unwrap();
        assert!(new3);
        assert_ne!(c.id, a.id);
        assert_eq!(s.open_count().unwrap(), 2);
    }

    #[test]
    fn closed_alert_is_not_reused() {
        let mut s = Store::in_memory().unwrap();
        let (a, _) = s.upsert_finding(&finding("k"), vec![], 60, false).unwrap();
        s.set_status(a.id, AlertStatus::Dismissed, None).unwrap();
        let (b, new) = s.upsert_finding(&finding("k"), vec![], 60, false).unwrap();
        assert!(new);
        assert_ne!(a.id, b.id);
        let h = s.history(10).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[1].status, AlertStatus::Dismissed);
    }

    #[test]
    fn seen_sets_and_meta() {
        let mut s = Store::in_memory().unwrap();
        assert!(s.net_mark_seen("node|1.2.3.0/24").unwrap());
        assert!(!s.net_mark_seen("node|1.2.3.0/24").unwrap());
        assert!(s.tcc_mark_seen("kTCCServiceListenEvent", "com.x").unwrap());
        assert!(!s.tcc_mark_seen("kTCCServiceListenEvent", "com.x").unwrap());
        let t1 = s.learning_started().unwrap();
        let t2 = s.learning_started().unwrap();
        assert_eq!(t1.timestamp(), t2.timestamp());
    }
}
