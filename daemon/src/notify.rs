//! Envio de alertas para o celular via ntfy (open-source; ntfy.sh ou servidor próprio).

use anyhow::Result;
use vigia_proto::Alert;

use crate::config::NtfyCfg;

pub fn should_send(cfg: &NtfyCfg, a: &Alert) -> bool {
    cfg.enabled && !cfg.topic.trim().is_empty() && a.severity >= cfg.min_severity
}

pub fn payload(cfg: &NtfyCfg, a: &Alert) -> serde_json::Value {
    use vigia_proto::Severity::*;
    let priority = match a.severity {
        Critical => 5,
        High => 4,
        Medium => 3,
        Low => 2,
    };
    serde_json::json!({
        "topic": cfg.topic,
        "title": format!("Faro: {}", a.title),
        "message": a.detail,
        "priority": priority,
        "tags": ["rotating_light", a.rule],
    })
}

/// Publica pelo endpoint JSON do ntfy (aceita UTF-8 no título, ao contrário dos cabeçalhos HTTP).
pub fn send(cfg: &NtfyCfg, a: &Alert) -> Result<()> {
    let url = cfg.server.trim_end_matches('/').to_string();
    ureq::post(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send_json(payload(cfg, a))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vigia_proto::{AlertStatus, ProcInfo, Severity};

    fn alert(sev: Severity) -> Alert {
        Alert {
            id: 1,
            ts: "t".into(),
            first_ts: "t".into(),
            rule: "secret_read".into(),
            severity: sev,
            title: "node leu .env.prd".into(),
            detail: "detalhe".into(),
            target: None,
            process: ProcInfo::default(),
            chain: vec![],
            actions: vec![],
            count: 1,
            status: AlertStatus::Open,
            status_message: None,
            simulated: false,
        }
    }

    #[test]
    fn respects_min_severity_and_topic() {
        let mut c = NtfyCfg {
            enabled: true,
            topic: "vigia-abc".into(),
            ..Default::default()
        };
        assert!(should_send(&c, &alert(Severity::Critical)));
        assert!(!should_send(&c, &alert(Severity::High)));
        c.topic = " ".into();
        assert!(!should_send(&c, &alert(Severity::Critical)));
    }

    #[test]
    fn payload_shape() {
        let c = NtfyCfg {
            enabled: true,
            topic: "t".into(),
            ..Default::default()
        };
        let p = payload(&c, &alert(Severity::Critical));
        assert_eq!(p["priority"], 5);
        assert_eq!(p["title"], "Faro: node leu .env.prd");
    }
}
