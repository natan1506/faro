//! Configuração do daemon (YAML).
//!
//! Dois arquivos:
//! - `config.yaml`: escrito na instalação, editado à mão.
//! - `allow.yaml`: lista de exceções que o próprio daemon acrescenta quando
//!   você clica em "Permitir sempre" no app.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use vigia_proto::{ProcInfo, Settings, Severity};

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Config {
    pub user: UserCfg,
    #[serde(default)]
    pub paths: Paths,
    #[serde(default)]
    pub sensor: SensorCfg,
    /// Pastas onde ficam repositórios de terceiros (código não confiável).
    #[serde(default)]
    pub third_party_dirs: Vec<String>,
    /// Se preenchida, leituras de `.env` só alertam dentro destas pastas (ou quando
    /// quem lê roda numa pasta de terceiros). Credenciais da home valem sempre.
    #[serde(default)]
    pub watch_dirs: Vec<String>,
    #[serde(default)]
    pub notify: NotifyCfg,
    /// Horas iniciais em que a regra de rede só aprende, sem alertar.
    #[serde(default = "d_learning")]
    pub learning_hours: u64,
    #[serde(default = "d_retention")]
    pub retention_days: u32,
    /// Janela de deduplicação: o mesmo evento dentro dela só incrementa o contador.
    #[serde(default = "d_dedup")]
    pub dedup_secs: i64,
    #[serde(default)]
    pub rules: Rules,
    #[serde(default)]
    pub allow: Vec<AllowEntry>,
}

fn d_learning() -> u64 {
    24
}
fn d_retention() -> u32 {
    30
}
fn d_dedup() -> i64 {
    120
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct UserCfg {
    /// Home do usuário monitorado (o daemon roda como root, então `~` não serve).
    pub home: String,
    /// UID do usuário: dono do socket.
    pub uid: u32,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct Paths {
    pub socket: String,
    pub db: String,
    pub allow_file: String,
    pub quarantine_dir: String,
    /// Escrito pela tela de Ajustes do app; vale por cima deste arquivo.
    pub settings_file: String,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            socket: vigia_proto::DEFAULT_SOCKET.into(),
            db: "/Library/Application Support/Vigia/vigia.db".into(),
            allow_file: "/Library/Application Support/Vigia/allow.yaml".into(),
            quarantine_dir: "/Library/Application Support/Vigia/quarantine".into(),
            settings_file: "/Library/Application Support/Vigia/ajustes.yaml".into(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct SensorCfg {
    pub eslogger: String,
    pub events: Vec<String>,
    /// Fila entre o sensor e o motor de regras. Se encher, eventos são descartados (e contados).
    pub queue: usize,
}

impl Default for SensorCfg {
    fn default() -> Self {
        Self {
            eslogger: "/usr/bin/eslogger".into(),
            events: vec!["exec".into(), "open".into(), "btm_launch_item_add".into()],
            queue: 20_000,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(default)]
pub struct NotifyCfg {
    pub ntfy: NtfyCfg,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct NtfyCfg {
    pub enabled: bool,
    pub server: String,
    /// Use um tópico longo e aleatório: no ntfy.sh qualquer um que saiba o nome lê.
    pub topic: String,
    pub min_severity: Severity,
}

impl Default for NtfyCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            server: "https://ntfy.sh".into(),
            topic: String::new(),
            min_severity: Severity::Critical,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Rules {
    pub secret_read: SecretReadRule,
    pub persistence: PersistenceRule,
    pub third_party_shell: ThirdPartyShellRule,
    pub unsigned_exec: UnsignedExecRule,
    pub tcc_change: TccRule,
    pub new_connection: NetRule,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct SecretReadRule {
    pub enabled: bool,
    pub severity: Severity,
    pub patterns: Vec<String>,
    pub exclude: Vec<String>,
    /// Um processo pode ler segredos do próprio projeto (mesmo repositório git
    /// do diretório onde ele roda), desde que não seja um repo de terceiros.
    pub same_project_ok: bool,
    /// Substrings baratas checadas antes de interpretar o JSON de um evento `open`.
    pub prefilter: Vec<String>,
}

impl Default for SecretReadRule {
    fn default() -> Self {
        Self {
            enabled: true,
            severity: Severity::Critical,
            patterns: vec![
                "**/.env".into(),
                "**/.env.*".into(),
                "~/.aws/**".into(),
                "~/.ssh/id_*".into(),
                "~/.npmrc".into(),
                "~/.netrc".into(),
                "~/.config/gh/hosts.yml".into(),
                "~/.docker/config.json".into(),
                "~/Library/Application Support/Google/Chrome/**/Cookies".into(),
                "~/Library/Application Support/Google/Chrome/**/Login Data".into(),
                "~/Library/Application Support/BraveSoftware/**/Cookies".into(),
                "~/Library/Application Support/BraveSoftware/**/Login Data".into(),
                "~/Library/Keychains/**".into(),
            ],
            exclude: vec![
                "**/.env.example".into(),
                "**/.env.sample".into(),
                "**/.env.template".into(),
                "**/node_modules/**".into(),
            ],
            same_project_ok: true,
            prefilter: vec![
                "/.env".into(),
                "/.aws/".into(),
                "/.ssh/".into(),
                "/.npmrc".into(),
                "/.netrc".into(),
                "/.config/gh/".into(),
                "/.docker/".into(),
                "/Cookies".into(),
                "/Login Data".into(),
                "/Keychains/".into(),
            ],
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct PersistenceRule {
    pub enabled: bool,
    pub severity: Severity,
}

impl Default for PersistenceRule {
    fn default() -> Self {
        Self {
            enabled: true,
            severity: Severity::Critical,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct ThirdPartyShellRule {
    pub enabled: bool,
    pub severity: Severity,
    /// Processos "pais" que rodam código de pacotes (um pacote malicioso roda dentro do node).
    pub parents: Vec<String>,
    /// Sempre suspeitos quando lançados por um desses pais, em qualquer pasta.
    pub always_suspicious: Vec<String>,
    /// Suspeitos só quando o pai roda dentro de `third_party_dirs` (scripts npm usam sh o tempo todo).
    pub suspicious_in_third_party: Vec<String>,
}

impl Default for ThirdPartyShellRule {
    fn default() -> Self {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect();
        Self {
            enabled: true,
            severity: Severity::High,
            parents: s(&["node", "npm", "npx", "pnpm", "yarn", "bun", "deno"]),
            always_suspicious: s(&[
                "osascript",
                "curl",
                "wget",
                "nc",
                "ncat",
                "socat",
                "security",
                "screencapture",
            ]),
            suspicious_in_third_party: s(&[
                "sh",
                "bash",
                "zsh",
                "dash",
                "python",
                "python3",
                "perl",
                "ruby",
                "launchctl",
                "chmod",
            ]),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct UnsignedExecRule {
    pub enabled: bool,
    pub severity: Severity,
    pub dirs: Vec<String>,
}

impl Default for UnsignedExecRule {
    fn default() -> Self {
        Self {
            enabled: true,
            severity: Severity::High,
            dirs: vec![
                "/tmp/".into(),
                "/private/tmp/".into(),
                "/private/var/tmp/".into(),
                "/private/var/folders/".into(),
                "~/Downloads/".into(),
                "~/Library/Caches/".into(),
                "~/Library/Application Support/".into(),
                "~/.local/share/".into(),
            ],
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct TccRule {
    pub enabled: bool,
    pub severity: Severity,
    pub interval_secs: u64,
    pub db: String,
    /// Monitoramento de Entrada (keylogger), Acessibilidade, envio de eventos e gravação de tela.
    pub services: Vec<String>,
}

impl Default for TccRule {
    fn default() -> Self {
        Self {
            enabled: true,
            severity: Severity::Critical,
            interval_secs: 10,
            db: "/Library/Application Support/com.apple.TCC/TCC.db".into(),
            services: vec![
                "kTCCServiceListenEvent".into(),
                "kTCCServiceAccessibility".into(),
                "kTCCServicePostEvent".into(),
                "kTCCServiceScreenCapture".into(),
            ],
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct NetRule {
    pub enabled: bool,
    pub severity: Severity,
    pub interval_secs: u64,
    /// Só esses comandos são observados (navegadores falam com milhares de hosts).
    pub commands: Vec<String>,
    pub lsof: String,
}

impl Default for NetRule {
    fn default() -> Self {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect();
        Self {
            enabled: true,
            severity: Severity::Medium,
            interval_secs: 5,
            commands: s(&[
                "node",
                "python",
                "python3",
                "ruby",
                "perl",
                "osascript",
                "curl",
                "wget",
                "nc",
                "ncat",
                "bash",
                "sh",
                "zsh",
                "deno",
                "bun",
            ]),
            lsof: "/usr/sbin/lsof".into(),
        }
    }
}

/// Exceção: "não alerte para esta regra quando o processo for este".
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct AllowEntry {
    /// Id da regra ou `*` para todas.
    #[serde(default = "d_star")]
    pub rule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_id: Option<String>,
    /// Time de desenvolvedor da Apple. Junto com `signing_id`, impede que um
    /// binário ad-hoc se passe por outro só declarando o mesmo identificador.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    /// Para a regra de rede: limita a exceção a um destino (`host/prefixo`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

fn d_star() -> String {
    "*".into()
}

impl AllowEntry {
    pub fn matches(&self, rule: &str, p: &ProcInfo, target: Option<&str>) -> bool {
        if self.rule != "*" && self.rule != rule {
            return false;
        }
        if self.signing_id.is_none() && self.path_prefix.is_none() {
            return false; // entrada vazia nunca libera tudo
        }
        if let Some(sid) = &self.signing_id {
            if p.signing_id.as_deref() != Some(sid.as_str()) {
                return false;
            }
        }
        if let Some(team) = &self.team_id {
            if p.team_id.as_deref() != Some(team.as_str()) {
                return false;
            }
        }
        if let Some(prefix) = &self.path_prefix {
            if !p.path.starts_with(prefix.as_str()) {
                return false;
            }
        }
        if let Some(t) = &self.target {
            if target != Some(t.as_str()) {
                return false;
            }
        }
        true
    }
}

#[derive(Deserialize, Serialize, Default)]
struct AllowFile {
    #[serde(default)]
    allow: Vec<AllowEntry>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("lendo {}", path.display()))?;
        let mut cfg: Config = serde_yaml::from_str(&raw)
            .with_context(|| format!("interpretando {}", path.display()))?;
        let sp = PathBuf::from(&cfg.paths.settings_file);
        if sp.exists() {
            let raw =
                std::fs::read_to_string(&sp).with_context(|| format!("lendo {}", sp.display()))?;
            let s: Settings = serde_yaml::from_str(&raw)
                .with_context(|| format!("interpretando {}", sp.display()))?;
            cfg.overlay(&s);
        }
        cfg.expand_all();
        let user_allow = cfg.load_allow_file()?;
        cfg.allow.extend(user_allow);
        Ok(cfg)
    }

    #[cfg(test)]
    pub fn from_yaml(raw: &str) -> Result<Self> {
        let mut cfg: Config = serde_yaml::from_str(raw)?;
        cfg.expand_all();
        Ok(cfg)
    }

    fn expand_all(&mut self) {
        let home = self.user.home.trim_end_matches('/').to_string();
        self.user.home = home.clone();
        let e = |v: &mut Vec<String>| v.iter_mut().for_each(|s| *s = expand_home(s, &home));
        e(&mut self.third_party_dirs);
        e(&mut self.watch_dirs);
        e(&mut self.rules.secret_read.patterns);
        e(&mut self.rules.secret_read.exclude);
        e(&mut self.rules.unsigned_exec.dirs);
        for a in &mut self.allow {
            if let Some(p) = &a.path_prefix {
                a.path_prefix = Some(expand_home(p, &home));
            }
        }
        // Pastas sempre terminam com '/', para "repos-x" não casar com "repos-xyz".
        for d in self
            .third_party_dirs
            .iter_mut()
            .chain(self.watch_dirs.iter_mut())
        {
            if !d.ends_with('/') {
                d.push('/');
            }
        }
    }

    fn load_allow_file(&self) -> Result<Vec<AllowEntry>> {
        let p = PathBuf::from(&self.paths.allow_file);
        if !p.exists() {
            return Ok(vec![]);
        }
        let raw = std::fs::read_to_string(&p)?;
        let f: AllowFile = serde_yaml::from_str(&raw).unwrap_or_default();
        Ok(f.allow)
    }

    /// Acrescenta uma exceção ao `allow.yaml` (o `config.yaml` não é tocado).
    pub fn append_allow(&mut self, entry: AllowEntry) -> Result<()> {
        if self.allow.contains(&entry) {
            return Ok(());
        }
        let p = PathBuf::from(&self.paths.allow_file);
        let mut f: AllowFile = if p.exists() {
            serde_yaml::from_str(&std::fs::read_to_string(&p)?).unwrap_or_default()
        } else {
            AllowFile::default()
        };
        f.allow.push(entry.clone());
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = p.with_extension("yaml.tmp");
        std::fs::write(&tmp, serde_yaml::to_string(&f)?)?;
        std::fs::rename(&tmp, &p)?;
        self.allow.push(entry);
        Ok(())
    }

    pub fn is_allowed(&self, rule: &str, p: &ProcInfo, target: Option<&str>) -> bool {
        self.allow.iter().any(|a| a.matches(rule, p, target))
    }

    /// Os ajustes atuais, com a home trocada por `~`.
    pub fn settings(&self) -> Settings {
        let home = format!("{}/", self.user.home);
        let show = |v: &[String]| -> Vec<String> {
            v.iter()
                .map(|d| {
                    let d = d.trim_end_matches('/');
                    match format!("{d}/").strip_prefix(&home) {
                        Some(rest) if !rest.is_empty() => {
                            format!("~/{}", rest.trim_end_matches('/'))
                        }
                        _ if d == self.user.home => "~".into(),
                        _ => d.to_string(),
                    }
                })
                .collect()
        };
        let r = &self.rules;
        let rules = [
            ("secret_read", r.secret_read.enabled),
            ("third_party_shell", r.third_party_shell.enabled),
            ("unsigned_exec", r.unsigned_exec.enabled),
            ("persistence", r.persistence.enabled),
            ("tcc_change", r.tcc_change.enabled),
            ("new_connection", r.new_connection.enabled),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        Settings {
            third_party_dirs: show(&self.third_party_dirs),
            watch_dirs: show(&self.watch_dirs),
            rules,
            ntfy_enabled: self.notify.ntfy.enabled,
            ntfy_topic: self.notify.ntfy.topic.clone(),
        }
    }

    /// Aplica os ajustes por cima da configuração (sem expandir `~`).
    fn overlay(&mut self, s: &Settings) {
        self.third_party_dirs = s.third_party_dirs.clone();
        self.watch_dirs = s.watch_dirs.clone();
        let r = &mut self.rules;
        for (k, &on) in &s.rules {
            match k.as_str() {
                "secret_read" => r.secret_read.enabled = on,
                "third_party_shell" => r.third_party_shell.enabled = on,
                "unsigned_exec" => r.unsigned_exec.enabled = on,
                "persistence" => r.persistence.enabled = on,
                "tcc_change" => r.tcc_change.enabled = on,
                "new_connection" => r.new_connection.enabled = on,
                _ => {}
            }
        }
        self.notify.ntfy.enabled = s.ntfy_enabled;
        self.notify.ntfy.topic = s.ntfy_topic.clone();
    }

    /// Confere, grava em `ajustes.yaml` e passa a valer na hora.
    /// `exists` é injetado para os testes não dependerem do disco.
    pub fn save_settings(
        &mut self,
        s: &Settings,
        exists: impl Fn(&str) -> bool,
    ) -> std::result::Result<(), String> {
        let home = self.user.home.clone();
        let clean = |v: &[String], what: &str| -> std::result::Result<Vec<String>, String> {
            let mut out: Vec<String> = vec![];
            for raw in v {
                let d = raw.trim();
                if d.is_empty() {
                    continue;
                }
                let d = d.trim_end_matches('/');
                let d = if d.is_empty() { "/" } else { d };
                if !(d.starts_with('/') || d == "~" || d.starts_with("~/")) {
                    return Err(format!("{what}: use um caminho completo ({raw})"));
                }
                let full = expand_home(d, &home);
                if !exists(&full) {
                    return Err(format!("{what}: a pasta {d} não existe"));
                }
                if !out.iter().any(|x| x == d) {
                    out.push(d.to_string());
                }
            }
            Ok(out)
        };
        let third = clean(&s.third_party_dirs, "Pastas de terceiros")?;
        for d in &third {
            let full = expand_home(d, &home);
            if full == "/" || full == home || full == format!("{home}/Documents") {
                return Err(format!(
                    "Pastas de terceiros: {d} é ampla demais; tudo ali dentro seria tratado como código não confiável"
                ));
            }
        }
        let watch = clean(&s.watch_dirs, "Pastas monitoradas")?;
        let topic = s.ntfy_topic.trim().to_string();
        if s.ntfy_enabled && topic.len() < 16 {
            return Err("Celular: use um tópico com pelo menos 16 caracteres; no ntfy.sh quem souber o nome lê os alertas".into());
        }
        let saved = Settings {
            third_party_dirs: third,
            watch_dirs: watch,
            rules: s.rules.clone(),
            ntfy_enabled: s.ntfy_enabled,
            ntfy_topic: topic,
        };
        let p = PathBuf::from(&self.paths.settings_file);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("criando {}: {e}", dir.display()))?;
        }
        let body = format!(
            "# Escrito pela tela de Ajustes do Faro. Vale por cima do config.yaml.\n{}",
            serde_yaml::to_string(&saved).map_err(|e| e.to_string())?
        );
        let tmp = p.with_extension("yaml.tmp");
        std::fs::write(&tmp, body)
            .and_then(|_| std::fs::rename(&tmp, &p))
            .map_err(|e| format!("gravando {}: {e}", p.display()))?;
        self.overlay(&saved);
        self.expand_all();
        Ok(())
    }

    /// Com `watch_dirs` preenchida, um `.env` fora delas não interessa.
    pub fn env_outside_watch(&self, path: &str) -> bool {
        if self.watch_dirs.is_empty() {
            return false;
        }
        let file = path.rsplit('/').next().unwrap_or(path);
        if !file.starts_with(".env") {
            return false; // credenciais da home: sempre
        }
        !self
            .watch_dirs
            .iter()
            .chain(self.third_party_dirs.iter())
            .any(|d| path.starts_with(d.as_str()))
    }

    /// O diretório está dentro de alguma pasta de repositórios de terceiros?
    pub fn in_third_party(&self, dir: &str) -> bool {
        let d = if dir.ends_with('/') {
            dir.to_string()
        } else {
            format!("{dir}/")
        };
        self.third_party_dirs
            .iter()
            .any(|t| d.starts_with(t.as_str()))
    }
}

pub fn expand_home(s: &str, home: &str) -> String {
    if s == "~" {
        home.to_string()
    } else if let Some(rest) = s.strip_prefix("~/") {
        format!("{}/{}", home.trim_end_matches('/'), rest)
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::from_yaml(
            r#"
user: { home: /Users/dev/, uid: 501 }
third_party_dirs: ["~/Documents/www/third-party"]
allow:
  - { rule: secret_read, path_prefix: "~/bin/" }
"#,
        )
        .unwrap()
    }

    #[test]
    fn expands_home_everywhere() {
        let c = cfg();
        assert_eq!(c.user.home, "/Users/dev");
        assert_eq!(
            c.third_party_dirs,
            vec!["/Users/dev/Documents/www/third-party/"]
        );
        assert!(c
            .rules
            .secret_read
            .patterns
            .contains(&"/Users/dev/.aws/**".to_string()));
        assert_eq!(c.allow[0].path_prefix.as_deref(), Some("/Users/dev/bin/"));
    }

    #[test]
    fn third_party_prefix_is_exact_dir() {
        let c = cfg();
        assert!(c.in_third_party("/Users/dev/Documents/www/third-party/repo"));
        assert!(c.in_third_party("/Users/dev/Documents/www/third-party"));
        assert!(!c.in_third_party("/Users/dev/Documents/www/third-party-2/repo"));
    }

    #[test]
    fn allow_entry_needs_a_selector() {
        let e = AllowEntry {
            rule: "*".into(),
            signing_id: None,
            team_id: None,
            path_prefix: None,
            target: None,
            comment: None,
        };
        assert!(!e.matches("secret_read", &ProcInfo::default(), None));
    }

    #[test]
    fn allow_entry_matches_rule_and_signer() {
        let e = AllowEntry {
            rule: "secret_read".into(),
            signing_id: Some("com.microsoft.VSCode".into()),
            team_id: Some("UBF8T346G9".into()),
            path_prefix: None,
            target: None,
            comment: None,
        };
        let p = ProcInfo {
            signing_id: Some("com.microsoft.VSCode".into()),
            team_id: Some("UBF8T346G9".into()),
            ..Default::default()
        };
        assert!(e.matches("secret_read", &p, None));
        assert!(!e.matches("persistence", &p, None));
        let impostor = ProcInfo {
            signing_id: Some("com.microsoft.VSCode".into()),
            ..Default::default()
        };
        assert!(
            !e.matches("secret_read", &impostor, None),
            "ad-hoc com o mesmo identificador não passa"
        );
    }

    #[test]
    fn append_allow_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.paths.allow_file = dir.path().join("allow.yaml").to_string_lossy().into();
        let e = AllowEntry {
            rule: "new_connection".into(),
            signing_id: None,
            team_id: None,
            path_prefix: Some("/usr/local/bin/node".into()),
            target: Some("104.16.0.0/24".into()),
            comment: Some("teste".into()),
        };
        c.append_allow(e.clone()).unwrap();
        c.append_allow(e.clone()).unwrap(); // idempotente
        let raw = std::fs::read_to_string(&c.paths.allow_file).unwrap();
        assert_eq!(raw.matches("new_connection").count(), 1);
        assert!(c.allow.contains(&e));
    }

    fn settings_cfg(dir: &std::path::Path) -> Config {
        Config::from_yaml(&format!(
            "user: {{ home: /Users/dev, uid: 501 }}\nthird_party_dirs: [\"~/repos-terceiros\"]\npaths: {{ settings_file: {}/ajustes.yaml }}\n",
            dir.display()
        ))
        .unwrap()
    }

    #[test]
    fn settings_show_home_as_tilde_and_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = settings_cfg(dir.path());
        let s = c.settings();
        assert_eq!(s.third_party_dirs, vec!["~/repos-terceiros"]);
        assert!(s.watch_dirs.is_empty());
        assert_eq!(s.rules.get("secret_read"), Some(&true));

        let mut new = s.clone();
        new.third_party_dirs.push("~/Downloads/".into());
        new.watch_dirs = vec!["~/projetos".into()];
        new.rules.insert("new_connection".into(), false);
        c.save_settings(&new, |_| true).unwrap();
        assert!(c.in_third_party("/Users/dev/Downloads/x"));
        assert!(!c.rules.new_connection.enabled);
        assert_eq!(
            c.settings().third_party_dirs,
            vec!["~/repos-terceiros", "~/Downloads"]
        );

        // Recarregando do disco, o ajustes.yaml vale por cima do config.
        let raw = format!(
            "user: {{ home: /Users/dev, uid: 501 }}\nthird_party_dirs: [\"~/outra\"]\npaths: {{ settings_file: {}/ajustes.yaml, allow_file: {}/allow.yaml }}\n",
            dir.path().display(),
            dir.path().display()
        );
        let cfgp = dir.path().join("config.yaml");
        std::fs::write(&cfgp, raw).unwrap();
        let again = Config::load(&cfgp).unwrap();
        assert_eq!(again.settings(), c.settings());
    }

    #[test]
    fn settings_validation() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = settings_cfg(dir.path());
        let base = c.settings();
        let try_ = |c: &mut Config, f: &dyn Fn(&mut Settings)| {
            let mut s = base.clone();
            f(&mut s);
            c.save_settings(&s, |p| p != "/Users/dev/nao-existe")
        };
        assert!(try_(&mut c, &|s| s.third_party_dirs = vec!["~".into()])
            .unwrap_err()
            .contains("ampla"));
        assert!(
            try_(&mut c, &|s| s.third_party_dirs = vec!["relativo".into()])
                .unwrap_err()
                .contains("caminho completo")
        );
        assert!(
            try_(&mut c, &|s| s.watch_dirs = vec!["~/nao-existe".into()])
                .unwrap_err()
                .contains("não existe")
        );
        assert!(try_(&mut c, &|s| {
            s.ntfy_enabled = true;
            s.ntfy_topic = "curto".into()
        })
        .unwrap_err()
        .contains("16"));
        // Nada foi gravado depois de um erro.
        assert!(!dir.path().join("ajustes.yaml").exists());
    }

    #[test]
    fn watch_dirs_limit_only_env_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = settings_cfg(dir.path());
        assert!(
            !c.env_outside_watch("/Users/dev/qualquer/.env"),
            "vazio = Mac inteiro"
        );
        let mut s = c.settings();
        s.watch_dirs = vec!["~/projetos".into()];
        c.save_settings(&s, |_| true).unwrap();
        assert!(!c.env_outside_watch("/Users/dev/projetos/a/.env.prd"));
        assert!(c.env_outside_watch("/Users/dev/outros/a/.env"));
        assert!(
            !c.env_outside_watch("/Users/dev/repos-terceiros/x/.env"),
            "terceiros contam como vigiadas"
        );
        assert!(
            !c.env_outside_watch("/Users/dev/.aws/credentials"),
            "credenciais sempre"
        );
    }
}
