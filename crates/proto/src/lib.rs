//! Protocolo do socket Unix entre o daemon (root) e o app (usuário).
//!
//! Cada mensagem é um objeto JSON numa linha (newline-delimited JSON).
//! O cliente manda `ClientMsg`; o daemon responde e também empurra
//! `ServerMsg::Alert` sempre que um alerta é criado ou atualizado.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_SOCKET: &str = "/var/run/vigia.sock";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn label_pt(self) -> &'static str {
        match self {
            Severity::Low => "baixa",
            Severity::Medium => "média",
            Severity::High => "alta",
            Severity::Critical => "crítica",
        }
    }
}

/// Um processo como o daemon o enxerga.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ProcInfo {
    pub pid: i32,
    pub ppid: i32,
    /// Caminho do executável.
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// `Some(false)` = sem assinatura válida ou só ad-hoc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed: Option<bool>,
}

impl ProcInfo {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// SIGKILL no processo.
    Kill,
    /// Congela e mata o processo e todos os descendentes.
    KillTree,
    /// Mata e move o binário ou o item de persistência para a quarentena.
    Quarantine,
    /// Não alerta mais para esta regra + este processo.
    Allow,
    /// Fecha o alerta sem fazer nada.
    Dismiss,
    /// Ação local do app: mostra o arquivo no Finder.
    Reveal,
    /// Ação local do app: abre Ajustes > Privacidade.
    OpenSettings,
}

impl ActionKind {
    /// Ações que o app executa sozinho, sem pedir ao daemon.
    pub fn is_client_side(self) -> bool {
        matches!(self, ActionKind::Reveal | ActionKind::OpenSettings)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertStatus {
    Open,
    Killed,
    Quarantined,
    Allowed,
    Dismissed,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Alert {
    pub id: i64,
    /// RFC 3339, UTC.
    pub ts: String,
    pub first_ts: String,
    pub rule: String,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    /// Arquivo, item de persistência ou host envolvido.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub process: ProcInfo,
    /// Ancestrais do processo, do pai para cima.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<ProcInfo>,
    pub actions: Vec<ActionKind>,
    /// Quantas vezes o mesmo evento se repetiu (deduplicação).
    pub count: u32,
    pub status: AlertStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    /// Veio de um replay (gravação), não do Mac de verdade. Os botões não executam nada.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub simulated: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SensorState {
    Starting,
    Running,
    Restarting { reason: String },
    Replay,
    Down { reason: String },
}

/// O que o app pode mudar pela tela de Ajustes. O daemon grava em `ajustes.yaml`,
/// que vale por cima do `config.yaml`. Pastas vêm com `~` para o usuário ler.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Settings {
    /// Onde você clona código em que não confia: regras mais rígidas lá dentro.
    pub third_party_dirs: Vec<String>,
    /// Onde os `.env` são vigiados. Vazio = o Mac inteiro (recomendado).
    pub watch_dirs: Vec<String>,
    /// Liga/desliga por regra (`secret_read`, `persistence`…).
    pub rules: std::collections::BTreeMap<String, bool>,
    pub ntfy_enabled: bool,
    pub ntfy_topic: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Status {
    pub version: String,
    pub started_at: String,
    pub sensor: SensorState,
    pub events_total: u64,
    pub events_dropped: u64,
    pub events_per_sec: f64,
    pub alerts_open: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_until: Option<String>,
}

/// Um processo iniciado, para a aba "Processos recentes" do painel.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExecEntry {
    pub ts: String,
    pub pid: i32,
    pub ppid: i32,
    pub path: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Programa da Apple (assinatura `com.apple.*`).
    pub apple: bool,
}

/// Achado feito pelo app na sessão do usuário (hoje: leitores de teclado).
/// O daemon grava e devolve como alerta normal.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Report {
    pub rule: String,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub process: ProcInfo,
}

pub const REPORTABLE_RULES: &[&str] = &["keyboard_tap"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ClientMsg {
    Hello {
        version: u32,
    },
    History {
        limit: u32,
    },
    Timeline {
        limit: u32,
    },
    Act {
        alert_id: i64,
        action: ActionKind,
    },
    Report {
        report: Report,
    },
    Status,
    Ping,
    GetSettings,
    SaveSettings {
        settings: Settings,
    },
    /// Pede a lista de exceções (`allow`) em vigor. Só leitura.
    Allowlist,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ServerMsg {
    Welcome {
        version: u32,
        status: Status,
    },
    /// Alerta novo ou atualizado (o cliente faz upsert pelo `id`).
    Alert {
        alert: Alert,
    },
    History {
        alerts: Vec<Alert>,
    },
    Timeline {
        entries: Vec<ExecEntry>,
    },
    /// Resposta a `Report` (o alerta em si chega como `Alert`).
    Reported {
        ok: bool,
        message: String,
    },
    ActionResult {
        alert_id: i64,
        action: ActionKind,
        ok: bool,
        message: String,
    },
    Status {
        status: Status,
    },
    Pong,
    Settings {
        settings: Settings,
    },
    /// Resposta a `SaveSettings`: em caso de sucesso, `settings` é o que ficou valendo.
    SettingsSaved {
        ok: bool,
        message: String,
        settings: Settings,
    },
    Error {
        message: String,
    },
    /// Resposta a `Allowlist`.
    Allowlist {
        #[serde(default)]
        entries: Vec<AllowView>,
    },
}

/// Exceção em vigor, como o app a vê (espelho de `AllowEntry` do daemon).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct AllowView {
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub signing_id: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub path_prefix: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
}

pub fn encode<T: Serialize>(msg: &T) -> String {
    let mut s = serde_json::to_string(msg).expect("mensagem serializável");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_msg_roundtrip() {
        let m = ClientMsg::Act {
            alert_id: 7,
            action: ActionKind::KillTree,
        };
        let s = encode(&m);
        assert_eq!(
            s,
            "{\"type\":\"act\",\"alert_id\":7,\"action\":\"kill_tree\"}\n"
        );
        let back: ClientMsg = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn severity_orders() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::Medium > Severity::Low);
    }

    #[test]
    fn allowlist_roundtrip() {
        let s = encode(&ClientMsg::Allowlist);
        assert_eq!(s, "{\"type\":\"allowlist\"}\n");
        assert_eq!(
            serde_json::from_str::<ClientMsg>(&s).unwrap(),
            ClientMsg::Allowlist
        );

        let m = ServerMsg::Allowlist {
            entries: vec![AllowView {
                rule: "secret_read".into(),
                signing_id: Some("com.exemplo.app".into()),
                team_id: Some("ABCDE12345".into()),
                ..Default::default()
            }],
        };
        let s = encode(&m);
        assert_eq!(serde_json::from_str::<ServerMsg>(&s).unwrap(), m);

        // Sem `entries` (mensagem antiga ou enxuta): lista vazia.
        match serde_json::from_str::<ServerMsg>(r#"{"type":"allowlist"}"#).unwrap() {
            ServerMsg::Allowlist { entries } => assert!(entries.is_empty()),
            outro => panic!("inesperado: {outro:?}"),
        }
    }
}
