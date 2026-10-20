//! Informações de processos fora do fluxo do Endpoint Security: diretório atual,
//! cadeia de pais, descendentes, raiz git. Tudo aqui é bloqueante e só roda
//! no caminho de um alerta (raro), nunca para cada evento.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Command;

use vigia_proto::ProcInfo;

/// Abstração para os testes poderem simular processos.
pub trait ProcLookup: Send {
    fn cwd(&self, pid: i32) -> Option<String>;
    fn git_root(&self, dir: &str) -> Option<String>;
}

pub struct SystemLookup;

impl ProcLookup for SystemLookup {
    fn cwd(&self, pid: i32) -> Option<String> {
        cwd_of(pid)
    }
    fn git_root(&self, dir: &str) -> Option<String> {
        git_root(dir)
    }
}

#[cfg(target_os = "linux")]
pub fn cwd_of(pid: i32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "linux"))]
pub fn cwd_of(pid: i32) -> Option<String> {
    // lsof -a -p PID -d cwd -Fn  ->  "p123\nfcwd\nn/Users/x/proj\n"
    let out = Command::new("/usr/sbin/lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix('n').map(String::from))
}

/// Sobe a partir de `dir` até achar uma pasta com `.git`.
pub fn git_root(dir: &str) -> Option<String> {
    let mut p = Path::new(dir);
    loop {
        if p.join(".git").exists() {
            return Some(p.to_string_lossy().into_owned());
        }
        p = p.parent()?;
    }
}

/// Lê pid, ppid, executável e argumentos de um processo vivo.
pub fn proc_by_pid(pid: i32) -> Option<ProcInfo> {
    if pid <= 0 {
        return None;
    }
    let ppid_out = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let ppid: i32 = String::from_utf8_lossy(&ppid_out.stdout)
        .trim()
        .parse()
        .ok()?;
    let args_out = Command::new("ps")
        .args(["-o", "args=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let args_line = String::from_utf8_lossy(&args_out.stdout).trim().to_string();
    let path = exe_path(pid).unwrap_or_else(|| {
        args_line
            .split_whitespace()
            .next()
            .unwrap_or("?")
            .to_string()
    });
    Some(ProcInfo {
        pid,
        ppid,
        path,
        args: if args_line.is_empty() {
            vec![]
        } else {
            vec![args_line]
        },
        cwd: cwd_of(pid),
        ..Default::default()
    })
}

#[cfg(target_os = "linux")]
fn exe_path(pid: i32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "linux"))]
fn exe_path(pid: i32) -> Option<String> {
    // No macOS, `comm` traz o caminho completo do executável.
    let out = Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Pais do processo, do mais próximo para o mais distante (para no launchd).
pub fn chain(pid: i32, max: usize) -> Vec<ProcInfo> {
    let mut out = Vec::new();
    let mut cur = proc_by_pid(pid).map(|p| p.ppid).unwrap_or(0);
    while cur > 1 && out.len() < max {
        match proc_by_pid(cur) {
            Some(p) => {
                cur = p.ppid;
                out.push(p);
            }
            None => break,
        }
    }
    out
}

/// Lista (pid, ppid) de todos os processos.
pub fn all_pairs() -> Vec<(i32, i32)> {
    let Ok(out) = Command::new("ps")
        .args(["-A", "-o", "pid=", "-o", "ppid="])
        .output()
    else {
        return vec![];
    };
    parse_pairs(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_pairs(s: &str) -> Vec<(i32, i32)> {
    s.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect()
}

/// Descendentes de `root` em largura (pais antes dos filhos).
pub fn descendants(root: i32, pairs: &[(i32, i32)]) -> Vec<i32> {
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for &(pid, ppid) in pairs {
        children.entry(ppid).or_default().push(pid);
    }
    let mut out = Vec::new();
    let mut q = VecDeque::from([root]);
    while let Some(p) = q.pop_front() {
        if let Some(kids) = children.get(&p) {
            for &k in kids {
                if k != root && !out.contains(&k) {
                    out.push(k);
                    q.push_back(k);
                }
            }
        }
    }
    out
}

fn is_protected(pid: i32) -> bool {
    pid <= 1 || pid == std::process::id() as i32
}

fn signal(pid: i32, sig: i32) -> bool {
    // SAFETY: kill(2) só recebe inteiros; não há memória envolvida.
    unsafe { libc::kill(pid, sig) == 0 }
}

pub fn kill(pid: i32) -> Result<(), String> {
    if is_protected(pid) {
        return Err(format!("pid {pid} é protegido"));
    }
    if signal(pid, libc::SIGKILL) {
        Ok(())
    } else {
        Err(format!(
            "kill({pid}) falhou: {}",
            std::io::Error::last_os_error()
        ))
    }
}

/// Congela a árvore inteira (para nenhum filho escapar com um fork), depois mata
/// dos netos para cima. Devolve quantos processos foram mortos.
pub fn kill_tree(pid: i32) -> Result<usize, String> {
    if is_protected(pid) {
        return Err(format!("pid {pid} é protegido"));
    }
    let mut tree = vec![pid];
    tree.extend(descendants(pid, &all_pairs()));
    tree.retain(|p| !is_protected(*p));
    for &p in &tree {
        signal(p, libc::SIGSTOP);
    }
    // Novo snapshot: pega filhos criados entre a listagem e o SIGSTOP.
    for p in descendants(pid, &all_pairs()) {
        if !tree.contains(&p) && !is_protected(p) {
            signal(p, libc::SIGSTOP);
            tree.push(p);
        }
    }
    let mut killed = 0;
    for &p in tree.iter().rev() {
        if signal(p, libc::SIGKILL) {
            killed += 1;
        }
    }
    if killed == 0 {
        Err(format!(
            "nenhum processo morto (pid {pid} já tinha terminado?)"
        ))
    } else {
        Ok(killed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descendants_breadth_first() {
        let pairs = parse_pairs("  10 1\n 11 10\n 12 10\n 13 11\n 99 1\n");
        assert_eq!(descendants(10, &pairs), vec![11, 12, 13]);
        assert!(descendants(99, &pairs).is_empty());
    }

    #[test]
    fn refuses_protected_pids() {
        assert!(kill(1).is_err());
        assert!(kill_tree(std::process::id() as i32).is_err());
    }

    #[test]
    fn git_root_walks_up() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("repo/.git")).unwrap();
        std::fs::create_dir_all(d.path().join("repo/apps/web")).unwrap();
        let sub = d.path().join("repo/apps/web");
        assert_eq!(
            git_root(sub.to_str().unwrap()).unwrap(),
            d.path().join("repo").to_string_lossy()
        );
    }

    #[test]
    fn kill_tree_kills_children() {
        // sh -> sleep; matar a árvore do sh derruba os dois.
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 30 & sleep 30; wait"])
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let n = kill_tree(child.id() as i32).unwrap();
        assert!(n >= 2, "matou {n}");
        let status = child.wait().unwrap();
        assert!(!status.success());
    }

    #[test]
    fn reads_live_process() {
        let me = std::process::id() as i32;
        let p = proc_by_pid(me).unwrap();
        assert_eq!(p.pid, me);
        assert!(p.ppid > 0);
        assert!(cwd_of(me).is_some());
    }
}
