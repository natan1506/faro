//! Motor de regras dos eventos do Endpoint Security.
//!
//! Recebe linhas do `eslogger`, descarta rápido o que não interessa e
//! transforma o resto em `Finding`s (alertas ainda sem id).
//! As regras de rede e de permissões (TCC) ficam em `net.rs` e `tcc.rs`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use vigia_proto::{ActionKind, ExecEntry, ProcInfo, Severity};

use crate::config::Config;
use crate::event::{parse_line, EsEvent};
use crate::procinfo::ProcLookup;

pub const SECRET_READ: &str = "secret_read";
pub const PERSISTENCE: &str = "persistence";
pub const THIRD_PARTY_SHELL: &str = "third_party_shell";
pub const UNSIGNED_EXEC: &str = "unsigned_exec";
pub const TCC_CHANGE: &str = "tcc_change";
pub const NEW_CONNECTION: &str = "new_connection";
pub const KEYBOARD_TAP: &str = "keyboard_tap";

const TIMELINE_MAX: usize = 1000;
pub type Timeline = Arc<Mutex<VecDeque<ExecEntry>>>;

/// Um alerta antes de ganhar id e de ser deduplicado.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub rule: &'static str,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub target: Option<String>,
    pub process: ProcInfo,
    pub actions: Vec<ActionKind>,
    /// Eventos com a mesma chave dentro da janela de dedup viram um alerta só.
    pub dedup_key: String,
}

const CWD_CACHE_MAX: usize = 8192;

pub struct Engine {
    cfg: Arc<RwLock<Config>>,
    secret: GlobSet,
    secret_exclude: GlobSet,
    /// pid -> diretório atual, aprendido dos eventos `exec`.
    cwd_cache: HashMap<i32, String>,
    ignore_pids: HashSet<i32>,
    ignore_paths: Vec<String>,
    lookup: Box<dyn ProcLookup>,
    timeline: Option<Timeline>,
}

impl Engine {
    pub fn new(cfg: Arc<RwLock<Config>>, lookup: Box<dyn ProcLookup>) -> Result<Self> {
        let (secret, secret_exclude) = {
            let c = cfg.read().unwrap();
            (
                globs(&c.rules.secret_read.patterns)?,
                globs(&c.rules.secret_read.exclude)?,
            )
        };
        let me = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        let eslogger = cfg.read().unwrap().sensor.eslogger.clone();
        Ok(Self {
            cfg,
            secret,
            secret_exclude,
            cwd_cache: HashMap::new(),
            ignore_pids: HashSet::from([std::process::id() as i32]),
            ignore_paths: me.into_iter().chain([eslogger]).collect(),
            lookup,
            timeline: None,
        })
    }

    /// Guarda os últimos processos iniciados para o painel.
    pub fn with_timeline(mut self, t: Timeline) -> Self {
        self.timeline = Some(t);
        self
    }

    /// pid do eslogger: eventos dele mesmo são ignorados.
    pub fn ignore_pid(&mut self, pid: i32) {
        self.ignore_pids.insert(pid);
    }

    /// Filtro barato antes de interpretar o JSON. Eventos `open` são a imensa
    /// maioria; só passam os que mencionam algum caminho sensível.
    pub fn prefilter(&self, line: &str) -> bool {
        if !line.contains("\"open\":{") {
            return true;
        }
        let c = self.cfg.read().unwrap();
        c.rules.secret_read.enabled
            && c.rules
                .secret_read
                .prefilter
                .iter()
                .any(|n| line.contains(n.as_str()))
    }

    pub fn on_line(&mut self, line: &str) -> Option<Finding> {
        if !self.prefilter(line) {
            return None;
        }
        let ev = parse_line(line)?;
        self.on_event(ev)
    }

    pub fn on_event(&mut self, ev: EsEvent) -> Option<Finding> {
        match ev {
            EsEvent::Exec { process, target } => {
                if let Some(cwd) = &target.cwd {
                    if self.cwd_cache.len() >= CWD_CACHE_MAX {
                        self.cwd_cache.clear();
                    }
                    self.cwd_cache.insert(target.pid, cwd.clone());
                }
                if let Some(t) = &self.timeline {
                    let mut t = t.lock().unwrap();
                    if t.len() >= TIMELINE_MAX {
                        t.pop_front();
                    }
                    t.push_back(ExecEntry {
                        ts: crate::store::now(),
                        pid: target.pid,
                        ppid: target.ppid,
                        path: target.path.clone(),
                        args: target.args.iter().take(32).cloned().collect(),
                        cwd: target.cwd.clone(),
                        apple: target
                            .signing_id
                            .as_deref()
                            .is_some_and(|s| s.starts_with("com.apple.")),
                    });
                }
                if self.ignored(&process) {
                    return None;
                }
                self.third_party_shell(&process, &target)
                    .or_else(|| self.unsigned_exec(&target))
            }
            EsEvent::Open { process, path } => {
                if self.ignored(&process) {
                    return None;
                }
                self.secret_read(process, path)
            }
            EsEvent::BtmAdd {
                process,
                instigator,
                item_path,
                executable_path,
            } => self.persistence(process, instigator, item_path, executable_path),
            EsEvent::Other => None,
        }
    }

    fn ignored(&self, p: &ProcInfo) -> bool {
        self.ignore_pids.contains(&p.pid) || self.ignore_paths.iter().any(|x| x == &p.path)
    }

    fn cwd_for(&mut self, pid: i32) -> Option<String> {
        if let Some(c) = self.cwd_cache.get(&pid) {
            return Some(c.clone());
        }
        let c = self.lookup.cwd(pid)?;
        self.cwd_cache.insert(pid, c.clone());
        Some(c)
    }

    fn secret_read(&mut self, mut process: ProcInfo, path: String) -> Option<Finding> {
        let (enabled, severity, same_project_ok) = {
            let c = self.cfg.read().unwrap();
            let r = &c.rules.secret_read;
            (r.enabled, r.severity, r.same_project_ok)
        };
        if !enabled || !self.secret.is_match(&path) || self.secret_exclude.is_match(&path) {
            return None;
        }
        if self
            .cfg
            .read()
            .unwrap()
            .is_allowed(SECRET_READ, &process, Some(&path))
        {
            return None;
        }
        let cwd = self.cwd_for(process.pid);
        process.cwd = cwd.clone();
        {
            // Fora das pastas escolhidas nos Ajustes o .env não interessa, a não ser que
            // quem lê rode numa pasta de terceiros (o caso do incidente).
            let c = self.cfg.read().unwrap();
            let reader_untrusted = cwd.as_deref().is_some_and(|d| c.in_third_party(d));
            if c.env_outside_watch(&path) && !reader_untrusted {
                return None;
            }
        }
        if same_project_ok {
            if let Some(cwd) = &cwd {
                if self.same_project(cwd, &path) {
                    return None;
                }
            }
        }
        let file = path.rsplit('/').next().unwrap_or(&path).to_string();
        let where_ = cwd
            .as_deref()
            .map(|c| format!(" (rodando em {c})"))
            .unwrap_or_default();
        Some(Finding {
            rule: SECRET_READ,
            severity,
            title: format!("{} leu {}", process.name(), file),
            detail: format!(
                "{} (pid {}){} abriu {}",
                process.path, process.pid, where_, path
            ),
            target: Some(path.clone()),
            dedup_key: format!("{SECRET_READ}|{}|{}", process.path, path),
            actions: vec![
                ActionKind::KillTree,
                ActionKind::Kill,
                ActionKind::Reveal,
                ActionKind::Allow,
                ActionKind::Dismiss,
            ],
            process,
        })
    }

    /// O arquivo pertence ao mesmo repositório git em que o processo roda,
    /// e esse repositório não é de terceiros?
    fn same_project(&self, cwd: &str, path: &str) -> bool {
        let c = self.cfg.read().unwrap();
        if c.in_third_party(cwd) {
            return false;
        }
        let Some(root) = self.lookup.git_root(cwd) else {
            return false;
        };
        let home = c.user.home.as_str();
        if root == "/" || root == home || !root.starts_with(&format!("{home}/")) {
            return false;
        }
        path.starts_with(&format!("{root}/"))
    }

    fn third_party_shell(&self, process: &ProcInfo, target: &ProcInfo) -> Option<Finding> {
        let c = self.cfg.read().unwrap();
        let r = &c.rules.third_party_shell;
        if !r.enabled {
            return None;
        }
        let parent = process.name();
        let child = target.name();
        if !r.parents.iter().any(|p| name_matches(parent, p)) {
            return None;
        }
        let cwd = target.cwd.as_deref().unwrap_or("");
        let in_tp = !cwd.is_empty() && c.in_third_party(cwd);
        let always = r.always_suspicious.iter().any(|x| name_matches(child, x));
        let tp_only = r
            .suspicious_in_third_party
            .iter()
            .any(|x| name_matches(child, x));
        if !(always || (tp_only && in_tp)) {
            return None;
        }
        // O processo que fez o exec é um fork do node; quem deve morrer é o node (o pai).
        let node = ProcInfo {
            pid: target.ppid,
            ppid: 0,
            path: process.path.clone(),
            cwd: target.cwd.clone(),
            signing_id: process.signing_id.clone(),
            team_id: process.team_id.clone(),
            signed: process.signed,
            args: vec![],
        };
        if c.is_allowed(THIRD_PARTY_SHELL, &node, None)
            || c.is_allowed(THIRD_PARTY_SHELL, target, None)
        {
            return None;
        }
        let cmd = if target.args.is_empty() {
            target.path.clone()
        } else {
            target.args.join(" ")
        };
        let place = if in_tp {
            "repositório de terceiros"
        } else {
            "projeto"
        };
        Some(Finding {
            rule: THIRD_PARTY_SHELL,
            severity: r.severity,
            title: format!("{parent} executou {child}"),
            detail: format!(
                "{parent} (pid {}) num {place} ({}) executou: {cmd}",
                node.pid,
                if cwd.is_empty() { "?" } else { cwd }
            ),
            target: Some(cmd),
            dedup_key: format!(
                "{THIRD_PARTY_SHELL}|{}|{}|{}",
                process.path, target.path, cwd
            ),
            actions: vec![
                ActionKind::KillTree,
                ActionKind::Kill,
                ActionKind::Allow,
                ActionKind::Dismiss,
            ],
            process: node,
        })
    }

    fn unsigned_exec(&self, target: &ProcInfo) -> Option<Finding> {
        let c = self.cfg.read().unwrap();
        let r = &c.rules.unsigned_exec;
        if !r.enabled || target.signed != Some(false) {
            return None;
        }
        if !r.dirs.iter().any(|d| target.path.starts_with(d.as_str())) {
            return None;
        }
        if c.is_allowed(UNSIGNED_EXEC, target, None) {
            return None;
        }
        Some(Finding {
            rule: UNSIGNED_EXEC,
            severity: r.severity,
            title: format!("Binário sem assinatura: {}", target.name()),
            detail: format!(
                "{} (pid {}) rodou de uma pasta temporária ou de usuário, sem assinatura de desenvolvedor",
                target.path, target.pid
            ),
            target: Some(target.path.clone()),
            dedup_key: format!("{UNSIGNED_EXEC}|{}", target.path),
            actions: vec![ActionKind::Quarantine, ActionKind::KillTree, ActionKind::Reveal, ActionKind::Allow, ActionKind::Dismiss],
            process: target.clone(),
        })
    }

    fn persistence(
        &self,
        process: ProcInfo,
        instigator: Option<ProcInfo>,
        item_path: Option<String>,
        executable_path: Option<String>,
    ) -> Option<Finding> {
        let c = self.cfg.read().unwrap();
        let r = &c.rules.persistence;
        if !r.enabled {
            return None;
        }
        let target = item_path.clone().or(executable_path.clone())?;
        if target.starts_with("/System/") || target.starts_with("/Library/Apple/") {
            return None;
        }
        let has_instigator = instigator.is_some();
        let who = instigator.unwrap_or(process);
        if c.is_allowed(PERSISTENCE, &who, Some(&target)) {
            return None;
        }
        let mut actions = vec![ActionKind::Quarantine];
        if has_instigator {
            actions.push(ActionKind::KillTree);
        }
        actions.extend([ActionKind::Reveal, ActionKind::Allow, ActionKind::Dismiss]);
        let name = target.rsplit('/').next().unwrap_or(&target).to_string();
        let exe = executable_path
            .map(|e| format!(", que executa {e}"))
            .unwrap_or_default();
        let by = if has_instigator {
            format!("{} (pid {})", who.path, who.pid)
        } else {
            "um processo do sistema".into()
        };
        Some(Finding {
            rule: PERSISTENCE,
            severity: r.severity,
            title: format!("Novo item de inicialização: {name}"),
            detail: format!("{by} registrou {target}{exe}. Ele roda sozinho a cada login."),
            target: Some(target.clone()),
            dedup_key: format!("{PERSISTENCE}|{target}"),
            actions,
            process: who,
        })
    }
}

fn globs(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(Glob::new(p)?);
    }
    Ok(b.build()?)
}

/// `python3.12` casa com `python3`; `bash` não casa com `ba`.
pub fn name_matches(name: &str, want: &str) -> bool {
    name == want
        || name
            .strip_prefix(want)
            .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeLookup {
        cwds: HashMap<i32, String>,
        git_roots: Vec<String>,
    }

    impl ProcLookup for FakeLookup {
        fn cwd(&self, pid: i32) -> Option<String> {
            self.cwds.get(&pid).cloned()
        }
        fn git_root(&self, dir: &str) -> Option<String> {
            self.git_roots
                .iter()
                .find(|r| dir == r.as_str() || dir.starts_with(&format!("{r}/")))
                .cloned()
        }
    }

    const HOME: &str = "/Users/dev";
    const PROJ: &str = "/Users/dev/Documents/www/projects/loja-backoffice";
    const TP: &str = "/Users/dev/Documents/www/third-party/ws-demo";

    fn engine(cwds: &[(i32, &str)]) -> Engine {
        let cfg = Config::from_yaml(&format!(
            r#"
user: {{ home: {HOME}, uid: 501 }}
third_party_dirs: ["~/Documents/www/third-party"]
allow:
  - {{ rule: secret_read, signing_id: com.microsoft.VSCode }}
"#
        ))
        .unwrap();
        let lookup = FakeLookup {
            cwds: cwds.iter().map(|(p, c)| (*p, c.to_string())).collect(),
            git_roots: vec![PROJ.into(), TP.into()],
        };
        Engine::new(Arc::new(RwLock::new(cfg)), Box::new(lookup)).unwrap()
    }

    fn node(pid: i32) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: 1,
            path: "/usr/local/bin/node".into(),
            signed: Some(false),
            ..Default::default()
        }
    }

    fn open(pid: i32, path: &str) -> EsEvent {
        EsEvent::Open {
            process: node(pid),
            path: path.into(),
        }
    }

    #[test]
    fn incident_fixture_reading_other_projects_env_alerts() {
        let mut e = engine(&[(4100, TP)]);
        let f = e
            .on_line(include_str!("../tests/fixtures/open_env.json"))
            .expect("deveria alertar");
        assert_eq!(f.rule, SECRET_READ);
        assert_eq!(f.severity, Severity::Critical);
        assert_eq!(f.process.cwd.as_deref(), Some(TP));
        assert!(f.actions.contains(&ActionKind::KillTree));
    }

    #[test]
    fn own_project_env_is_fine() {
        let mut e = engine(&[(10, PROJ)]);
        assert!(e
            .on_event(open(10, &format!("{PROJ}/.env.local")))
            .is_none());
    }

    #[test]
    fn third_party_repo_reading_its_own_env_still_alerts() {
        let mut e = engine(&[(10, TP)]);
        assert!(e.on_event(open(10, &format!("{TP}/.env"))).is_some());
    }

    #[test]
    fn unknown_cwd_alerts() {
        let mut e = engine(&[]);
        assert!(e
            .on_event(open(10, &format!("{HOME}/.aws/credentials")))
            .is_some());
    }

    #[test]
    fn examples_and_node_modules_are_excluded() {
        let mut e = engine(&[]);
        assert!(e
            .on_event(open(10, &format!("{PROJ}/.env.example")))
            .is_none());
        assert!(e
            .on_event(open(10, &format!("{PROJ}/node_modules/dotenv/.env")))
            .is_none());
        assert!(e
            .on_event(open(10, &format!("{PROJ}/src/index.ts")))
            .is_none());
    }

    #[test]
    fn allowlisted_signer_is_fine() {
        let mut e = engine(&[]);
        let p = ProcInfo {
            pid: 20,
            path: "/Applications/Visual Studio Code.app/x".into(),
            signing_id: Some("com.microsoft.VSCode".into()),
            ..Default::default()
        };
        assert!(e
            .on_event(EsEvent::Open {
                process: p,
                path: format!("{PROJ}/.env")
            })
            .is_none());
    }

    #[test]
    fn prefilter_skips_boring_opens_but_keeps_exec() {
        let e = engine(&[]);
        assert!(!e.prefilter(r#"{"event":{"open":{"file":{"path":"/usr/lib/libz.dylib"}}}}"#));
        assert!(e.prefilter(r#"{"event":{"open":{"file":{"path":"/Users/dev/p/.env"}}}}"#));
        assert!(e.prefilter(r#"{"event":{"exec":{}}}"#));
    }

    #[test]
    fn incident_fixture_node_running_curl_alerts_on_node() {
        let mut e = engine(&[]);
        let f = e
            .on_line(include_str!("../tests/fixtures/exec_curl.json"))
            .expect("deveria alertar");
        assert_eq!(f.rule, THIRD_PARTY_SHELL);
        assert_eq!(f.process.pid, 4100, "o alvo da ação é o node, não o curl");
        assert!(f.detail.contains("repositório de terceiros"));
        assert!(f.target.unwrap().contains("exfil.example"));
    }

    fn exec(parent: &str, child: &str, cwd: &str) -> EsEvent {
        EsEvent::Exec {
            process: ProcInfo {
                pid: 50,
                ppid: 40,
                path: parent.into(),
                ..Default::default()
            },
            target: ProcInfo {
                pid: 50,
                ppid: 40,
                path: child.into(),
                cwd: Some(cwd.into()),
                signed: Some(true),
                ..Default::default()
            },
        }
    }

    #[test]
    fn npm_scripts_using_sh_in_own_project_are_fine() {
        let mut e = engine(&[]);
        assert!(e
            .on_event(exec("/usr/local/bin/node", "/bin/sh", PROJ))
            .is_none());
        assert!(e
            .on_event(exec("/usr/local/bin/node", "/bin/sh", TP))
            .is_some());
        assert!(e
            .on_event(exec("/usr/local/bin/node", "/usr/bin/osascript", PROJ))
            .is_some());
        assert!(
            e.on_event(exec("/bin/zsh", "/usr/bin/curl", PROJ))
                .is_none(),
            "curl no seu terminal é normal"
        );
    }

    #[test]
    fn exec_learns_cwd_for_later_opens() {
        let mut e = engine(&[]);
        e.on_event(exec("/bin/zsh", "/usr/local/bin/node", PROJ));
        assert!(e.on_event(open(50, &format!("{PROJ}/.env"))).is_none());
    }

    #[test]
    fn unsigned_binary_in_tmp_alerts() {
        let mut e = engine(&[]);
        let ev = EsEvent::Exec {
            process: ProcInfo {
                pid: 60,
                path: "/bin/sh".into(),
                ..Default::default()
            },
            target: ProcInfo {
                pid: 60,
                path: "/private/var/folders/xy/T/agent".into(),
                signed: Some(false),
                ..Default::default()
            },
        };
        let f = e.on_event(ev).unwrap();
        assert_eq!(f.rule, UNSIGNED_EXEC);
        assert_eq!(f.actions[0], ActionKind::Quarantine);

        let signed = EsEvent::Exec {
            process: ProcInfo {
                pid: 61,
                path: "/bin/sh".into(),
                ..Default::default()
            },
            target: ProcInfo {
                pid: 61,
                path: "/private/tmp/ok".into(),
                signed: Some(true),
                ..Default::default()
            },
        };
        assert!(e.on_event(signed).is_none());
    }

    #[test]
    fn incident_fixture_launch_agent_alerts() {
        let mut e = engine(&[]);
        let f = e
            .on_line(include_str!("../tests/fixtures/btm_add.json"))
            .unwrap();
        assert_eq!(f.rule, PERSISTENCE);
        assert_eq!(
            f.process.pid, 4100,
            "culpa vai para quem registrou, não para o btm daemon"
        );
        assert!(f.actions.contains(&ActionKind::KillTree));
        assert!(f.detail.contains("/Users/dev/.cache/twh/agent"));
    }

    #[test]
    fn apple_launch_items_are_ignored() {
        let e = engine(&[]);
        assert!(e
            .persistence(
                node(1),
                None,
                Some("/System/Library/LaunchDaemons/x.plist".into()),
                None
            )
            .is_none());
    }

    #[test]
    fn ignores_own_events() {
        let mut e = engine(&[]);
        e.ignore_pid(4100);
        assert!(e
            .on_line(include_str!("../tests/fixtures/open_env.json"))
            .is_none());
    }

    #[test]
    fn name_matching() {
        assert!(name_matches("python3.12", "python3"));
        assert!(name_matches("python3", "python"));
        assert!(!name_matches("bashful", "bash"));
        assert!(!name_matches("nc-tool", "nc"));
    }
}
