//! Regra de conexão nova: lê `lsof` a cada poucos segundos e alerta quando
//! um processo observado (node, python, curl...) fala com uma rede que ele
//! nunca usou antes. O Endpoint Security não tem evento de conexão de rede.

use std::net::IpAddr;
use std::process::Command;

use vigia_proto::{ActionKind, ProcInfo};

use crate::config::Config;
use crate::rules::{name_matches, Finding, NEW_CONNECTION};

#[derive(Debug, Clone, PartialEq)]
pub struct Conn {
    pub pid: i32,
    pub command: String,
    pub remote_ip: IpAddr,
    pub remote_port: u16,
}

/// Saída de `lsof -nP +c 0 -iTCP -sTCP:ESTABLISHED -Fpcn`.
pub fn parse_lsof(s: &str) -> Vec<Conn> {
    let mut out = Vec::new();
    let mut pid = 0;
    let mut cmd = String::new();
    for line in s.lines() {
        let (tag, rest) = line.split_at(line.len().min(1));
        match tag {
            "p" => pid = rest.parse().unwrap_or(0),
            "c" => cmd = rest.to_string(),
            "n" => {
                if let Some((ip, port)) = rest
                    .split_once("->")
                    .and_then(|(_, remote)| split_host_port(remote))
                {
                    out.push(Conn {
                        pid,
                        command: cmd.clone(),
                        remote_ip: ip,
                        remote_port: port,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// `1.2.3.4:443` ou `[2606:4700::1]:443`.
fn split_host_port(s: &str) -> Option<(IpAddr, u16)> {
    let (host, port) = s.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Some((host.parse().ok()?, port.parse().ok()?))
}

pub fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_unspecified()
                || v.octets()[0] == 100 && (64..128).contains(&v.octets()[1])
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            v.is_loopback()
                || v.is_unspecified()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Agrupa por rede (/24 no IPv4, /48 no IPv6): CDNs trocam de IP o tempo todo.
pub fn net_prefix(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            format!("{}.{}.{}.0/24", o[0], o[1], o[2])
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            format!("{:x}:{:x}:{:x}::/48", s[0], s[1], s[2])
        }
    }
}

pub fn run_lsof(bin: &str) -> String {
    Command::new(bin)
        .args(["-nP", "+c", "0", "-iTCP", "-sTCP:ESTABLISHED", "-Fpcn"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Uma passada: devolve (chaves novas para marcar, alertas). `is_new` consulta
/// e marca o estado persistido; `learning` suprime os alertas.
pub fn scan(
    cfg: &Config,
    conns: &[Conn],
    learning: bool,
    mut is_new: impl FnMut(&str) -> bool,
    resolve: impl Fn(i32) -> Option<ProcInfo>,
) -> Vec<Finding> {
    let r = &cfg.rules.new_connection;
    let mut out = Vec::new();
    for c in conns {
        if is_private(&c.remote_ip) || !r.commands.iter().any(|w| name_matches(&c.command, w)) {
            continue;
        }
        let prefix = net_prefix(&c.remote_ip);
        // A chave usa o nome do comando para não precisar de `ps` a cada passada;
        // o processo completo só é resolvido quando a rede é nova.
        let key = format!("{}|{}", c.command, prefix);
        if !is_new(&key) || learning {
            continue;
        }
        let proc = resolve(c.pid).unwrap_or(ProcInfo {
            pid: c.pid,
            path: c.command.clone(),
            ..Default::default()
        });
        if cfg.is_allowed(NEW_CONNECTION, &proc, Some(&prefix)) {
            continue;
        }
        let where_ = proc
            .cwd
            .as_deref()
            .map(|d| format!(" em {d}"))
            .unwrap_or_default();
        out.push(Finding {
            rule: NEW_CONNECTION,
            severity: r.severity,
            title: format!("{} conectou a uma rede nova", c.command),
            detail: format!(
                "{} (pid {}){} abriu conexão com {}:{} (rede {}), que ele nunca tinha usado",
                proc.path, c.pid, where_, c.remote_ip, c.remote_port, prefix
            ),
            target: Some(prefix.clone()),
            dedup_key: key,
            actions: vec![
                ActionKind::KillTree,
                ActionKind::Kill,
                ActionKind::Allow,
                ActionKind::Dismiss,
            ],
            process: proc,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const LSOF: &str = "p4100\ncnode\nf23\nn192.168.0.10:50123->104.16.24.35:443\nf24\nn127.0.0.1:3000->127.0.0.1:50999\np900\ncGoogle Chrome Helper\nf30\nn192.168.0.10:50200->142.250.1.1:443\np4242\ncnode\nf5\nn[2804:14c::1]:50000->[2606:4700:10::6816:1823]:443\n";

    fn cfg() -> Config {
        Config::from_yaml("user: { home: /Users/dev, uid: 501 }").unwrap()
    }

    #[test]
    fn parses_ipv4_and_ipv6() {
        let c = parse_lsof(LSOF);
        assert_eq!(c.len(), 4);
        assert_eq!(c[0].pid, 4100);
        assert_eq!(c[0].remote_port, 443);
        assert_eq!(c[2].command, "Google Chrome Helper");
        assert_eq!(c[3].remote_ip.to_string(), "2606:4700:10::6816:1823");
    }

    #[test]
    fn prefixes() {
        assert_eq!(
            net_prefix(&"104.16.24.35".parse().unwrap()),
            "104.16.24.0/24"
        );
        assert_eq!(
            net_prefix(&"2606:4700:10::1".parse().unwrap()),
            "2606:4700:10::/48"
        );
        assert!(is_private(&"127.0.0.1".parse().unwrap()));
        assert!(is_private(&"100.64.1.1".parse().unwrap()));
        assert!(!is_private(&"104.16.24.35".parse().unwrap()));
    }

    #[test]
    fn alerts_only_new_public_nets_of_watched_commands() {
        let c = cfg();
        let conns = parse_lsof(LSOF);
        let mut seen = HashSet::new();
        let f = scan(&c, &conns, false, |k| seen.insert(k.to_string()), |_| None);
        assert_eq!(
            f.len(),
            2,
            "node v4 e node v6; chrome e localhost ficam de fora"
        );
        let again = scan(&c, &conns, false, |k| seen.insert(k.to_string()), |_| None);
        assert!(again.is_empty());
    }

    #[test]
    fn learning_records_without_alerting() {
        let c = cfg();
        let conns = parse_lsof(LSOF);
        let mut seen = HashSet::new();
        assert!(scan(&c, &conns, true, |k| seen.insert(k.to_string()), |_| None).is_empty());
        assert_eq!(seen.len(), 2);
    }
}
