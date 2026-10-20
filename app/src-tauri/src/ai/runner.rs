//! Roda o Claude Code ou o Codex já instalados no Mac, em modo de resposta pura:
//! sem ferramentas (Claude) ou só leitura (Codex), a partir de uma pasta vazia e
//! com o ambiente mínimo. Aqui ficam a montagem do comando, a leitura da saída
//! linha a linha e as mensagens de erro em português.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use super::redact::redact;
use crate::app_config::AiProvider;

/// Quanto do stderr guardamos para a mensagem de erro (o resto é lido e descartado).
const MAX_STDERR: usize = 64 * 1024;
/// Letras do stderr que aparecem na mensagem de erro.
const MAX_ERRO: usize = 300;

#[derive(Debug, PartialEq)]
pub enum StreamEvent {
    Text(String),
    Done,
    Error(String),
}

/// Nome do CLI nas mensagens ("o Claude Code", "o Codex").
pub fn nome(provider: AiProvider) -> &'static str {
    match provider {
        AiProvider::Claude => "Claude Code",
        AiProvider::Codex => "Codex",
    }
}

fn arquivo(provider: AiProvider) -> &'static str {
    match provider {
        AiProvider::Claude => "claude",
        AiProvider::Codex => "codex",
    }
}

/// O shell de login do usuário (`$SHELL`), ou o zsh do macOS.
pub fn login_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| s.starts_with('/'))
        .unwrap_or_else(|| "/bin/zsh".into())
}

/// Procura `claude` e `codex` pelo PATH do shell de login. Um app aberto pelo
/// Finder tem PATH mínimo, por isso o shell; se ele não achar, vale `~/.local/bin`.
pub fn detect(shell: &str) -> Vec<(AiProvider, PathBuf)> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let fallbacks = [AiProvider::Claude, AiProvider::Codex]
        .map(|p| (p, home.join(".local/bin").join(arquivo(p))));
    detect_with(shell, &fallbacks)
}

fn executavel(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.is_absolute()
        && std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Saída de `<shell> -lc '...'`, com prazo: um `.zprofile` travado, ou um
/// processo de fundo que herdou o stdout e nunca fecha, não pode travar o app.
/// A leitura fica numa thread; o que chegou até o prazo é o que vale.
fn saida_do_shell(shell: &str, script: &str, prazo: Duration) -> String {
    use std::io::BufRead;
    use std::sync::mpsc;
    let Ok(mut child) = std::process::Command::new(shell)
        .args(["-lc", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return String::new();
    };
    let (tx, rx) = mpsc::channel::<String>();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for linha in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(linha).is_err() {
                    break;
                }
            }
        });
    }
    let fim = Instant::now() + prazo;
    let mut linhas = Vec::new();
    let mut saiu = false;
    loop {
        let agora = Instant::now();
        if agora >= fim {
            break;
        }
        if !saiu {
            saiu = matches!(child.try_wait(), Ok(Some(_)) | Err(_));
        }
        // Depois que o shell sai, só um instante para o resto da saída chegar.
        let espera = if saiu {
            Duration::from_millis(200)
        } else {
            Duration::from_millis(50)
        }
        .min(fim - agora);
        match rx.recv_timeout(espera) {
            Ok(l) => linhas.push(l),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) if saiu => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    if !saiu {
        let _ = child.kill();
    }
    let _ = child.wait();
    linhas.join("\n")
}

/// `detect` com o shell e os caminhos conhecidos à escolha (para os testes).
pub fn detect_with(shell: &str, fallbacks: &[(AiProvider, PathBuf)]) -> Vec<(AiProvider, PathBuf)> {
    let saida = saida_do_shell(
        shell,
        "command -v claude; command -v codex",
        Duration::from_secs(5),
    );
    let mut achados = Vec::new();
    for p in [AiProvider::Claude, AiProvider::Codex] {
        // Só caminhos absolutos: `command -v` de um alias ou função devolve texto.
        let do_shell = saida
            .lines()
            .map(|l| PathBuf::from(l.trim()))
            .find(|c| c.file_name().and_then(|n| n.to_str()) == Some(arquivo(p)) && executavel(c));
        let conhecido = || {
            fallbacks
                .iter()
                .find(|(fp, c)| *fp == p && executavel(c))
                .map(|(_, c)| c.clone())
        };
        if let Some(c) = do_shell.or_else(conhecido) {
            achados.push((p, c));
        }
    }
    achados
}

/// Texto que vai como valor de opção ou argumento posicional nunca pode
/// começar com `-` (o CLI leria como opção). Um espaço na frente resolve.
fn sem_hifen(s: &str) -> String {
    if s.starts_with('-') {
        format!(" {s}")
    } else {
        s.to_string()
    }
}

pub fn command_for(
    provider: AiProvider,
    bin: &Path,
    model: &str,
    prompt: &str,
    system: &str,
) -> (PathBuf, Vec<String>) {
    let mut a: Vec<String> = Vec::new();
    let (prompt, system) = (sem_hifen(prompt), sem_hifen(system));
    match provider {
        AiProvider::Claude => {
            a.extend(
                [
                    "-p",
                    &prompt,
                    "--append-system-prompt",
                    &system,
                    "--tools",
                    "",
                    "--strict-mcp-config",
                    "--setting-sources",
                    "",
                    "--no-session-persistence",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    // Pedaços da resposta conforme chegam (`stream_event`).
                    "--include-partial-messages",
                ]
                .map(String::from),
            );
            if !model.is_empty() {
                a.extend(["--model".to_string(), model.to_string()]);
            }
        }
        AiProvider::Codex => {
            a.extend(
                [
                    "exec",
                    // Sem o `~/.codex/config.toml`: os servidores MCP de lá rodam fora
                    // do sandbox. O login continua (vem do `CODEX_HOME`). No codex
                    // 0.163, `-c mcp_servers={}` não basta: ele mescla com o arquivo.
                    "--ignore-user-config",
                    "--sandbox",
                    "read-only",
                    "--ephemeral",
                    "--skip-git-repo-check",
                    "--json",
                ]
                .map(String::from),
            );
            if !model.is_empty() {
                a.extend(["-m".to_string(), model.to_string()]);
            }
            // O Codex não manda pedaços: cada mensagem chega inteira (`item.completed`).
            a.push(format!("{system}\n\n{prompt}"));
        }
    }
    (bin.to_path_buf(), a)
}

/// O ambiente do CLI: só o necessário, nada de `AWS_*`, tokens etc.
pub fn clean_env() -> Vec<(String, String)> {
    ["HOME", "PATH", "LANG", "TERM", "USER"]
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .collect()
}

/// `clean_env` com a pasta do binário no PATH (o app aberto pelo Finder tem
/// PATH mínimo, e instaladores que usam scripts precisam achar os vizinhos).
pub fn env_for(bin: &Path) -> Vec<(String, String)> {
    let mut env = clean_env();
    let Some(dir) = bin
        .parent()
        .and_then(|d| d.to_str())
        .filter(|d| !d.is_empty())
    else {
        return env;
    };
    match env.iter_mut().find(|(k, _)| k == "PATH") {
        Some((_, v)) if v.split(':').any(|p| p == dir) => {}
        Some((_, v)) => *v = format!("{dir}:{v}"),
        None => env.push((
            "PATH".into(),
            format!("{dir}:/usr/bin:/bin:/usr/sbin:/sbin"),
        )),
    }
    env
}

/// Redige o texto inteiro e só depois corta: cortar antes poderia deixar meio
/// segredo que o filtro já não reconhece.
fn resumo(s: &str) -> String {
    let limpo = redact(s.trim());
    let mut r: String = limpo.chars().take(MAX_ERRO).collect();
    if limpo.chars().count() > MAX_ERRO {
        r.push('…');
    }
    if r.is_empty() {
        r.push_str("(sem detalhes)");
    }
    r
}

pub fn friendly_error(
    provider: AiProvider,
    missing: bool,
    exit: Option<i32>,
    stderr: &str,
) -> String {
    let n = nome(provider);
    if missing {
        return format!("Não achei o {n} no seu PATH. Instale ou faça login e tente de novo.");
    }
    match exit {
        Some(c) => format!("O {n} respondeu com erro (código {c}): {}", resumo(stderr)),
        None => format!("O {n} respondeu com erro: {}", resumo(stderr)),
    }
}

/// Erro que veio na própria saída do CLI (limite de uso, modelo inválido…).
pub fn stream_error(provider: AiProvider, msg: &str) -> String {
    format!("O {} respondeu com erro: {}", nome(provider), resumo(msg))
}

fn texto(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.is_empty())
}

/// O Codex manda o erro da API como JSON dentro da string; tira a mensagem de lá.
fn mensagem_de_erro(m: &str) -> String {
    serde_json::from_str::<Value>(m)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from))
        .unwrap_or_else(|| m.to_string())
}

/// Lê a saída do CLI linha a linha. Tem estado porque o Claude, com
/// `--include-partial-messages`, manda a resposta em pedaços (`stream_event`) e
/// depois repete a mensagem inteira (`assistant`): a repetição é descartada.
/// Sem pedaços (versão antiga, erro antes do stream), vale a mensagem inteira.
/// Também põe uma linha em branco entre blocos de texto diferentes, para que a
/// soma dos `Text` seja exatamente a resposta.
pub struct Parser {
    provider: AiProvider,
    /// Já saiu algum texto (o próximo bloco precisa de separador).
    escreveu: bool,
    /// Bloco novo começou e ainda não recebeu texto.
    bloco_novo: bool,
    /// Pedaços já mandados desde a última mensagem `assistant`.
    em_pedacos: bool,
}

impl Parser {
    pub fn new(provider: AiProvider) -> Self {
        Self {
            provider,
            escreveu: false,
            bloco_novo: false,
            em_pedacos: false,
        }
    }

    fn texto_novo(&mut self, t: &str, inicio_de_bloco: bool) -> StreamEvent {
        let sep = inicio_de_bloco && self.escreveu && !t.starts_with('\n');
        self.escreveu = true;
        StreamEvent::Text(if sep {
            format!("\n\n{t}")
        } else {
            t.to_string()
        })
    }

    pub fn feed(&mut self, line: &str) -> Vec<StreamEvent> {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        match self.provider {
            AiProvider::Claude => self.claude(&v),
            AiProvider::Codex => self.codex(&v),
        }
    }

    fn claude(&mut self, v: &Value) -> Vec<StreamEvent> {
        match v["type"].as_str() {
            Some("stream_event") => {
                let ev = &v["event"];
                match ev["type"].as_str() {
                    Some("content_block_start") if ev["content_block"]["type"] == "text" => {
                        self.bloco_novo = true;
                        Vec::new()
                    }
                    Some("content_block_delta") if ev["delta"]["type"] == "text_delta" => {
                        let Some(t) = texto(&ev["delta"]["text"]) else {
                            return Vec::new();
                        };
                        let inicio = std::mem::take(&mut self.bloco_novo);
                        self.em_pedacos = true;
                        vec![self.texto_novo(t, inicio)]
                    }
                    _ => Vec::new(),
                }
            }
            // Com `error` (limite de uso etc.), o texto é o aviso do erro, que o `result` repete.
            Some("assistant") if v.get("error").is_none_or(Value::is_null) => {
                if std::mem::take(&mut self.em_pedacos) {
                    return Vec::new();
                }
                let blocos: Vec<String> = v["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["type"] == "text")
                    .filter_map(|c| texto(&c["text"]).map(String::from))
                    .collect();
                blocos.iter().map(|t| self.texto_novo(t, true)).collect()
            }
            Some("result") if v["is_error"] == true => {
                let m = texto(&v["result"])
                    .or_else(|| texto(&v["subtype"]))
                    .unwrap_or("erro sem detalhes");
                vec![StreamEvent::Error(m.to_string())]
            }
            Some("result") => vec![StreamEvent::Done],
            _ => Vec::new(),
        }
    }

    /// O Codex `--json` (0.163.0) não tem pedaços: cada mensagem do agente
    /// chega inteira num `item.completed`.
    fn codex(&mut self, v: &Value) -> Vec<StreamEvent> {
        match v["type"].as_str() {
            Some("item.completed") if v["item"]["type"] == "agent_message" => {
                match texto(&v["item"]["text"]) {
                    Some(t) => vec![self.texto_novo(t, true)],
                    None => Vec::new(),
                }
            }
            Some("turn.completed") => vec![StreamEvent::Done],
            Some("error") => vec![StreamEvent::Error(mensagem_de_erro(
                texto(&v["message"]).unwrap_or("erro sem detalhes"),
            ))],
            Some("turn.failed") => vec![StreamEvent::Error(mensagem_de_erro(
                texto(&v["error"]["message"]).unwrap_or("erro sem detalhes"),
            ))],
            _ => Vec::new(),
        }
    }
}

/// Como terminou uma execução.
pub struct Finished {
    pub exit: Option<i32>,
    pub stderr: String,
    pub timed_out: bool,
}

/// Mata o grupo de processos do CLI (ele roda num grupo próprio).
pub fn kill_group(pid: u32) {
    // pid 0 ou 1 viraria "meu grupo" / "todos": nunca.
    if pid > 1 {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

/// O que rodar: o binário, os argumentos de `command_for`, o ambiente e a pasta.
pub struct Comando<'a> {
    pub provider: AiProvider,
    pub bin: &'a Path,
    pub args: &'a [String],
    pub env: &'a [(String, String)],
    pub cwd: &'a Path,
}

/// Roda o CLI e repassa cada evento da saída. `on_spawn` recebe o pid logo que o
/// processo nasce; se devolver `false` (o usuário já clicou em Parar), o processo
/// é morto na hora. Passado o `limite`, o grupo inteiro é morto. Erro só quando
/// o processo nem chega a nascer.
pub async fn run(
    cmd: Comando<'_>,
    limite: Duration,
    on_spawn: impl FnOnce(u32) -> bool,
    mut on_event: impl FnMut(StreamEvent),
) -> std::io::Result<Finished> {
    let Comando {
        provider,
        bin,
        args,
        env,
        cwd,
    } = cmd;
    let mut child = tokio::process::Command::new(bin)
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()?;
    let pid = child.id().unwrap_or(0);
    if !on_spawn(pid) {
        kill_group(pid);
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let saida = async {
        if let Some(out) = stdout {
            let mut parser = Parser::new(provider);
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                for ev in parser.feed(&l) {
                    on_event(ev);
                }
            }
        }
    };
    let erro = async {
        let mut guardado = Vec::new();
        if let Some(mut err) = stderr {
            let mut buf = [0u8; 8192];
            while let Ok(n) = err.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let cabe = MAX_STDERR.saturating_sub(guardado.len()).min(n);
                guardado.extend_from_slice(&buf[..cabe]);
            }
        }
        String::from_utf8_lossy(&guardado).into_owned()
    };
    let tudo = async {
        let ((), stderr) = tokio::join!(saida, erro);
        // Netos que tenham ficado para trás saem junto. Antes do `wait`: enquanto
        // o filho não é recolhido, o id do grupo não pode ter sido reusado.
        kill_group(pid);
        (child.wait().await, stderr)
    };
    let resultado = tokio::time::timeout(limite, tudo).await;
    match resultado {
        Ok((status, stderr)) => Ok(Finished {
            exit: status.ok().and_then(|s| s.code()),
            stderr,
            timed_out: false,
        }),
        Err(_) => {
            kill_group(pid);
            let _ = child.wait().await;
            Ok(Finished {
                exit: None,
                stderr: String::new(),
                timed_out: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(provider: AiProvider, model: &str) -> Vec<String> {
        command_for(provider, Path::new("/x/bin"), model, "pergunta", "sistema").1
    }

    fn tem_seguidos(args: &[String], par: &[&str]) -> bool {
        args.windows(par.len())
            .any(|w| w.iter().zip(par).all(|(a, b)| a == b))
    }

    fn resumo(evs: &[StreamEvent]) -> Vec<String> {
        evs.iter()
            .map(|e| match e {
                StreamEvent::Text(t) => format!("Text({t})"),
                StreamEvent::Done => "Done".into(),
                StreamEvent::Error(m) => format!("Error({m})"),
            })
            .collect()
    }

    fn todas(provider: AiProvider, linhas: &[&str]) -> Vec<String> {
        resumo(
            &linhas
                .iter()
                .scan(Parser::new(provider), |p, l| Some(p.feed(l)))
                .flatten()
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn claude_has_no_tools() {
        let (bin, a) = command_for(AiProvider::Claude, Path::new("/x/claude"), "", "p", "s");
        assert_eq!(bin, PathBuf::from("/x/claude"));
        assert!(tem_seguidos(&a, &["--tools", ""]), "{a:?}");
        assert!(a.contains(&"--strict-mcp-config".to_string()));
        assert!(tem_seguidos(&a, &["--setting-sources", ""]));
        assert!(a.contains(&"--no-session-persistence".to_string()));
        assert!(tem_seguidos(&a, &["-p", "p"]));
        assert!(tem_seguidos(&a, &["--append-system-prompt", "s"]));
        assert!(tem_seguidos(&a, &["--output-format", "stream-json"]));
        assert!(a.contains(&"--include-partial-messages".to_string()));
        // Nenhuma opção que dê ferramentas ou permissões de volta.
        for proibida in [
            "--allowedTools",
            "--dangerously-skip-permissions",
            "--mcp-config",
        ] {
            assert!(!a.iter().any(|x| x.starts_with(proibida)), "{proibida}");
        }
    }

    #[test]
    fn prompt_never_starts_with_hyphen() {
        for p in [AiProvider::Claude, AiProvider::Codex] {
            let (_, a) = command_for(p, Path::new("/x/bin"), "", "--help", "-s");
            match p {
                AiProvider::Claude => {
                    assert!(tem_seguidos(&a, &["-p", " --help"]), "{a:?}");
                    assert!(
                        tem_seguidos(&a, &["--append-system-prompt", " -s"]),
                        "{a:?}"
                    );
                }
                AiProvider::Codex => assert!(!a.last().unwrap().starts_with('-'), "{a:?}"),
            }
        }
    }

    #[test]
    fn claude_model_only_when_set() {
        assert!(!args_of(AiProvider::Claude, "").contains(&"--model".to_string()));
        assert!(tem_seguidos(
            &args_of(AiProvider::Claude, "sonnet"),
            &["--model", "sonnet"]
        ));
    }

    #[test]
    fn codex_is_read_only() {
        let a = args_of(AiProvider::Codex, "");
        assert_eq!(a[0], "exec");
        assert!(tem_seguidos(&a, &["--sandbox", "read-only"]), "{a:?}");
        assert!(a.contains(&"--ephemeral".to_string()));
        assert!(a.contains(&"--json".to_string()));
        // Sem os servidores MCP do `~/.codex/config.toml`.
        assert!(a.contains(&"--ignore-user-config".to_string()), "{a:?}");
        assert!(!a.contains(&"-m".to_string()));
        assert_eq!(a.last().unwrap(), "sistema\n\npergunta");
        let a = args_of(AiProvider::Codex, "gpt-5");
        assert!(tem_seguidos(&a, &["-m", "gpt-5"]));
        assert_eq!(a.last().unwrap(), "sistema\n\npergunta");
    }

    #[test]
    fn env_is_minimal() {
        // Nome só deste teste, desfeito no fim (não vaza para os outros testes).
        const VAR: &str = "AWS_SECRET_ACCESS_KEY_VIGIA_TESTE";
        std::env::set_var(VAR, "abc123");
        let env = clean_env();
        std::env::remove_var(VAR);
        assert!(env.iter().all(|(k, _)| k != VAR));
        assert!(env.iter().all(|(k, _)| k != "AWS_SECRET_ACCESS_KEY"));
        for (k, _) in &env {
            assert!(
                ["HOME", "PATH", "LANG", "TERM", "USER"].contains(&k.as_str()),
                "{k}"
            );
        }
        assert!(env.iter().any(|(k, _)| k == "HOME"));
    }

    // Linhas reais do `claude` 2.1.303 (`--output-format stream-json --verbose`),
    // com os campos que não importam cortados.
    const CLAUDE_INIT: &str = r#"{"type":"system","subtype":"init","cwd":"/tmp/x","session_id":"488b988f","tools":[],"mcp_servers":[],"model":"claude-opus-5-5","permissionMode":"default"}"#;
    const CLAUDE_TEXTO: &str = r#"{"type":"assistant","message":{"id":"msg_1","model":"claude-opus-5-5","role":"assistant","type":"message","content":[{"type":"text","text":"Oi! Tudo calmo por aqui."}],"stop_reason":"end_turn"},"parent_tool_use_id":null,"session_id":"488b988f"}"#;
    const CLAUDE_FIM: &str = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":2100,"num_turns":1,"result":"Oi! Tudo calmo por aqui.","session_id":"488b988f","total_cost_usd":0.01}"#;
    const CLAUDE_LIMITE_TEXTO: &str = r#"{"type":"assistant","message":{"id":"2997d549","model":"<synthetic>","role":"assistant","stop_reason":"stop_sequence","type":"message","content":[{"type":"text","text":"You've hit your session limit · resets 6:10pm (America/Sao_Paulo)"}]},"error":"rate_limit","session_id":"488b988f"}"#;
    const CLAUDE_LIMITE_FIM: &str = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":"You've hit your session limit · resets 6:10pm (America/Sao_Paulo)","session_id":"488b988f"}"#;

    #[test]
    fn parses_claude_stream() {
        let r = todas(AiProvider::Claude, &[CLAUDE_INIT, CLAUDE_TEXTO, CLAUDE_FIM]);
        assert_eq!(r, ["Text(Oi! Tudo calmo por aqui.)", "Done"]);
    }

    // Linhas reais do `claude` 2.1.303 com `--include-partial-messages` ("diga oi"),
    // campos que não importam cortados.
    const CLAUDE_PEDACOS: [&str; 13] = [
        r#"{"type":"system","subtype":"init","cwd":"/tmp/cl","session_id":"a92e68b0","tools":[],"mcp_servers":[],"model":"claude-opus-5-5"}"#,
        r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-opus-5-5","id":"msg_011CgCMKAqZ1bkgEH9xJG2C5","type":"message","role":"assistant","content":[]}},"session_id":"a92e68b0"}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"session_id":"a92e68b0","parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Oi! T"}},"session_id":"a92e68b0","parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"udo bem por"}},"session_id":"a92e68b0","parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" a\u00ed"}},"session_id":"a92e68b0","parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"? Estou p"}},"session_id":"a92e68b0","parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ronto para ajudar no"}},"session_id":"a92e68b0","parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" que voc\u00ea precisar hoje."}},"session_id":"a92e68b0","parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"model":"claude-opus-5-5","id":"msg_011CgCMKAqZ1bkgEH9xJG2C5","type":"message","role":"assistant","content":[{"type":"text","text":"Oi! Tudo bem por a\u00ed? Estou pronto para ajudar no que voc\u00ea precisar hoje."}]},"session_id":"a92e68b0"}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0},"session_id":"a92e68b0"}"#,
        r#"{"type":"stream_event","event":{"type":"message_stop"},"session_id":"a92e68b0"}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"Oi! Tudo bem por a\u00ed? Estou pronto para ajudar no que voc\u00ea precisar hoje.","session_id":"a92e68b0"}"#,
    ];

    #[test]
    fn parses_claude_partial_stream() {
        let mut p = Parser::new(AiProvider::Claude);
        let evs: Vec<StreamEvent> = CLAUDE_PEDACOS.iter().flat_map(|l| p.feed(l)).collect();
        let pedacos: Vec<&str> = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        // Seis pedaços, e a mensagem inteira do fim não repete o texto.
        assert_eq!(pedacos.len(), 6, "{pedacos:?}");
        assert_eq!(
            pedacos.concat(),
            "Oi! Tudo bem por aí? Estou pronto para ajudar no que você precisar hoje."
        );
        assert_eq!(evs.last(), Some(&StreamEvent::Done));
    }

    #[test]
    fn separates_text_blocks() {
        let bloco = |t: &str| {
            [
                r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#.to_string(),
                format!(r#"{{"type":"stream_event","event":{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"{t}"}}}}}}"#),
            ]
        };
        let mut p = Parser::new(AiProvider::Claude);
        let linhas: Vec<String> = bloco("um").into_iter().chain(bloco("dois")).collect();
        let soma: String = linhas
            .iter()
            .flat_map(|l| p.feed(l))
            .filter_map(|e| match e {
                StreamEvent::Text(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(soma, "um\n\ndois");
        // Duas mensagens do Codex também.
        let mut p = Parser::new(AiProvider::Codex);
        let a = p.feed(CODEX_OK[2]);
        let b = p.feed(CODEX_OK[2]);
        assert_eq!(a, [StreamEvent::Text("Oi!".into())]);
        assert_eq!(b, [StreamEvent::Text("\n\nOi!".into())]);
    }

    #[test]
    fn parses_claude_error() {
        // O aviso de limite vem como texto do assistente marcado com `error`:
        // não é resposta, só o erro do `result` conta.
        let r = todas(
            AiProvider::Claude,
            &[CLAUDE_INIT, CLAUDE_LIMITE_TEXTO, CLAUDE_LIMITE_FIM],
        );
        assert_eq!(
            r,
            ["Error(You've hit your session limit · resets 6:10pm (America/Sao_Paulo))"]
        );
    }

    // Linhas reais do `codex exec --json` 0.163.0 ("diga oi" e um modelo inexistente).
    const CODEX_OK: [&str; 4] = [
        r#"{"type":"thread.started","thread_id":"01a155f8-888f-7113-ada7-d21149bd910f"}"#,
        r#"{"type":"turn.started"}"#,
        r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"Oi!"}}"#,
        r#"{"type":"turn.completed","usage":{"input_tokens":15148,"cached_input_tokens":12288,"cache_write_input_tokens":0,"output_tokens":6,"reasoning_output_tokens":0}}"#,
    ];
    const CODEX_ERRO: [&str; 5] = [
        r#"{"type":"thread.started","thread_id":"01a155f8-d0b4-76b3-b6c9-d359fb9b62e6"}"#,
        r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Model metadata for `modelo-que-nao-existe` not found. Defaulting to fallback metadata; this can degrade performance and cause issues."}}"#,
        r#"{"type":"turn.started"}"#,
        r#"{"type":"error","message":"{\"type\":\"error\",\"status\":400,\"error\":{\"type\":\"invalid_request_error\",\"message\":\"The 'modelo-que-nao-existe' model is not supported when using Codex with a ChatGPT account.\"}}"}"#,
        r#"{"type":"turn.failed","error":{"message":"{\"type\":\"error\",\"status\":400,\"error\":{\"type\":\"invalid_request_error\",\"message\":\"The 'modelo-que-nao-existe' model is not supported when using Codex with a ChatGPT account.\"}}"}}"#,
    ];

    #[test]
    fn parses_codex_stream() {
        assert_eq!(todas(AiProvider::Codex, &CODEX_OK), ["Text(Oi!)", "Done"]);
    }

    #[test]
    fn parses_codex_error() {
        // O aviso de metadados (item do tipo error) não é fatal; o erro da API é,
        // e a mensagem de dentro do JSON sai limpa.
        let msg = "Error(The 'modelo-que-nao-existe' model is not supported when using Codex with a ChatGPT account.)";
        assert_eq!(todas(AiProvider::Codex, &CODEX_ERRO), [msg, msg]);
    }

    #[test]
    fn garbage_line_is_ignored() {
        for p in [AiProvider::Claude, AiProvider::Codex] {
            let mut parser = Parser::new(p);
            assert!(parser.feed("não é json").is_empty());
            assert!(parser.feed("").is_empty());
            assert!(parser.feed("[1,2]").is_empty());
            assert!(parser
                .feed(r#"{"type":"assistant","message":{"content":"x"}}"#)
                .is_empty());
        }
    }

    #[test]
    fn friendly_errors() {
        assert_eq!(
            friendly_error(AiProvider::Claude, true, None, ""),
            "Não achei o Claude Code no seu PATH. Instale ou faça login e tente de novo."
        );
        assert_eq!(
            friendly_error(AiProvider::Codex, true, None, ""),
            "Não achei o Codex no seu PATH. Instale ou faça login e tente de novo."
        );
        let token = "sk-abcdefghijklmnopqrstuvwx1234567890";
        let m = friendly_error(
            AiProvider::Claude,
            false,
            Some(1),
            &format!("falhou com {token}\n"),
        );
        assert!(
            m.starts_with("O Claude Code respondeu com erro (código 1): "),
            "{m}"
        );
        assert!(!m.contains(token), "{m}");
        assert!(!m.contains("abcdefghij"), "{m}");
        let m = friendly_error(AiProvider::Codex, false, Some(2), &"a".repeat(1000));
        assert!(m.starts_with("O Codex respondeu com erro (código 2): "));
        assert_eq!(m.chars().filter(|c| *c == 'a').count(), 300);
        // Segredo depois das 300 letras também não pode sobrar pela metade.
        let longo = format!("{}{token}", "b".repeat(290));
        assert!(!friendly_error(AiProvider::Codex, false, Some(1), &longo).contains("sk-abcdefgh"));
    }

    #[test]
    fn detect_reads_login_shell() {
        // `sh -lc` com um PATH só nosso: acha o que existir, ignora o resto.
        let d = std::env::temp_dir().join(format!("vigia-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let script = d.join("fake-shell");
        let claude = d.join("claude");
        std::fs::write(&claude, "#!/bin/sh\n").unwrap();
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho alias codex=foo\necho {}\n",
                claude.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        for f in [&script, &claude] {
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let found = detect_with(script.to_str().unwrap(), &[]);
        assert_eq!(found, vec![(AiProvider::Claude, claude.clone())]);
        // Shell que não acha nada: vale o caminho conhecido, se existir.
        let found = detect_with("/usr/bin/false", &[(AiProvider::Codex, claude.clone())]);
        assert_eq!(found, vec![(AiProvider::Codex, claude)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Pasta temporária com um "CLI" falso (script sh).
    fn falso(nome: &str, corpo: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("vigia-run-{}-{nome}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let bin = d.join("cli");
        std::fs::write(&bin, format!("#!/bin/sh\n{corpo}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (d, bin)
    }

    async fn rodar_falso(
        bin: &Path,
        args: &[String],
        env: &[(String, String)],
        cwd: &Path,
        limite: Duration,
        parar: bool,
    ) -> (Finished, Vec<StreamEvent>, u32) {
        let mut evs = Vec::new();
        let mut pid = 0;
        let cmd = Comando {
            provider: AiProvider::Claude,
            bin,
            args,
            env,
            cwd,
        };
        let fim = run(
            cmd,
            limite,
            |p| {
                pid = p;
                !parar
            },
            |e| evs.push(e),
        )
        .await
        .unwrap();
        (fim, evs, pid)
    }

    fn vivo(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[tokio::test]
    async fn run_reports_exit_and_stderr() {
        let (d, bin) = falso(
            "erro",
            r#"echo '{"type":"assistant","message":{"content":[{"type":"text","text":"meio"}]}}'
echo "deu ruim com sk-abcdefghijklmnopqrstuvwx1234567890" >&2
exit 1"#,
        );
        let (fim, evs, _) = rodar_falso(
            &bin,
            &[],
            &env_for(&bin),
            &d,
            Duration::from_secs(10),
            false,
        )
        .await;
        assert_eq!(fim.exit, Some(1));
        assert!(!fim.timed_out);
        assert_eq!(evs, [StreamEvent::Text("meio".into())]);
        let m = friendly_error(AiProvider::Claude, false, fim.exit, &fim.stderr);
        assert!(
            m.starts_with("O Claude Code respondeu com erro (código 1): deu ruim com"),
            "{m}"
        );
        assert!(!m.contains("abcdefgh"), "{m}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn run_times_out_and_kills_the_group() {
        // O neto (sleep de fundo) também tem que morrer. O prazo é folgado porque
        // o macOS às vezes demora mais de 1 s para começar a rodar um script
        // recém-criado (verificação de arquivo novo), ainda mais com testes em paralelo.
        let (d, bin) = falso("prazo", "sleep 30 &\necho $! > neto.pid\nsleep 30");
        let inicio = Instant::now();
        let (fim, _, pid) =
            rodar_falso(&bin, &[], &env_for(&bin), &d, Duration::from_secs(4), false).await;
        assert!(fim.timed_out);
        assert_eq!(fim.exit, None);
        assert!(inicio.elapsed() < Duration::from_secs(8));
        let neto: i32 = std::fs::read_to_string(d.join("neto.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!vivo(neto), "neto {neto} sobreviveu");
        assert!(!vivo(pid as i32), "cli {pid} sobreviveu");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn run_normal_end_kills_leftover_grandchild() {
        // Neto que largou o stdout/stderr e ficou rodando depois que o CLI saiu:
        // o grupo é morto antes de recolher o CLI (o id do grupo ainda é dele).
        let (d, bin) = falso(
            "sobra",
            "sleep 30 >/dev/null 2>&1 &\necho $! > neto.pid\nexit 0",
        );
        let (fim, _, _) = rodar_falso(
            &bin,
            &[],
            &env_for(&bin),
            &d,
            Duration::from_secs(10),
            false,
        )
        .await;
        assert!(!fim.timed_out);
        assert_eq!(fim.exit, Some(0));
        let neto: i32 = std::fs::read_to_string(d.join("neto.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!vivo(neto), "neto {neto} sobreviveu");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn run_stopped_before_or_during() {
        let (d, bin) = falso("parar", "sleep 30");
        // Parar antes do processo nascer: morre logo.
        let inicio = Instant::now();
        let (fim, _, _) =
            rodar_falso(&bin, &[], &env_for(&bin), &d, Duration::from_secs(20), true).await;
        assert!(!fim.timed_out && fim.exit.is_none());
        assert!(inicio.elapsed() < Duration::from_secs(5));
        // Parar no meio (o que o `ai_stop` faz): `kill_group` com o pid recebido.
        let pid = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let pid2 = pid.clone();
        let env = env_for(&bin);
        let cmd = Comando {
            provider: AiProvider::Claude,
            bin: &bin,
            args: &[],
            env: &env,
            cwd: &d,
        };
        let rodando = run(
            cmd,
            Duration::from_secs(20),
            move |p| {
                pid2.store(p, std::sync::atomic::Ordering::SeqCst);
                true
            },
            |_| {},
        );
        let parar = async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            kill_group(pid.load(std::sync::atomic::Ordering::SeqCst));
        };
        let inicio = Instant::now();
        let (fim, ()) = tokio::join!(rodando, parar);
        let fim = fim.unwrap();
        assert!(!fim.timed_out && fim.exit.is_none());
        assert!(inicio.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn run_child_sees_only_minimal_env() {
        // awk lê o ambiente que recebeu de verdade (sem um shell no meio que
        // acrescente PWD, SHLVL…) e devolve cada nome como uma linha JSON.
        const VAR: &str = "AWS_SECRET_ACCESS_KEY_VIGIA_RUN";
        std::env::set_var(VAR, "segredo");
        let awk = PathBuf::from("/usr/bin/awk");
        let prog = r#"BEGIN { for (k in ENVIRON) printf "{\"type\":\"result\",\"is_error\":true,\"result\":\"%s=%s\"}\n", k, ENVIRON[k] }"#;
        let d = std::env::temp_dir().join(format!("vigia-run-{}-env", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let env = env_for(Path::new("/opt/ia/bin/claude"));
        let (_, evs, _) = rodar_falso(
            &awk,
            &[prog.to_string()],
            &env,
            &d,
            Duration::from_secs(10),
            false,
        )
        .await;
        std::env::remove_var(VAR);
        let recebidas: Vec<(String, String)> = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Error(kv) => kv.split_once('=').map(|(k, v)| (k.into(), v.into())),
                _ => None,
            })
            .collect();
        assert!(!recebidas.is_empty());
        for (k, _) in &recebidas {
            assert!(
                ["HOME", "PATH", "LANG", "TERM", "USER"].contains(&k.as_str()),
                "{k} vazou"
            );
        }
        let path = &recebidas.iter().find(|(k, _)| k == "PATH").unwrap().1;
        assert!(path.starts_with("/opt/ia/bin:"), "{path}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn detect_does_not_hang_on_background_stdout() {
        // Um processo de fundo herda o stdout do shell e o mantém aberto.
        let (d, claude) = falso("fundo-claude", "");
        let (_, shell) = falso(
            "fundo-shell",
            &format!("echo {}\nsleep 20 &", claude.display()),
        );
        let shell = shell.to_string_lossy().to_string();
        let inicio = Instant::now();
        let saida = saida_do_shell(&shell, "", Duration::from_secs(5));
        // Não espera o `sleep 20` de fundo (só o atraso de início de script novo).
        assert!(
            inicio.elapsed() < Duration::from_millis(4500),
            "{:?}",
            inicio.elapsed()
        );
        assert_eq!(saida.trim(), claude.to_string_lossy());
        // Shell que nunca termina: volta no prazo.
        let (_, lento) = falso("lento-shell", "sleep 20");
        let inicio = Instant::now();
        assert_eq!(
            saida_do_shell(&lento.to_string_lossy(), "", Duration::from_millis(300)),
            ""
        );
        assert!(inicio.elapsed() < Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn child_path_includes_bin_dir() {
        let env = env_for(Path::new("/opt/ia/bin/claude"));
        let path = &env.iter().find(|(k, _)| k == "PATH").unwrap().1;
        assert!(path.split(':').any(|p| p == "/opt/ia/bin"), "{path}");
    }

    /// Roda o CLI de verdade (precisa estar instalado e logado; gasta uma pergunta).
    async fn smoke(provider: AiProvider) {
        let bin = detect(&login_shell())
            .into_iter()
            .find(|(p, _)| *p == provider)
            .map(|(_, b)| b)
            .expect("CLI não encontrado");
        let dir = std::env::temp_dir().join(format!(
            "vigia-smoke-{}-{}",
            arquivo(provider),
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (bin, args) = command_for(
            provider,
            &bin,
            "",
            "Responda só: oi",
            "Você é um teste do Faro.",
        );
        let env = env_for(&bin);
        let mut evs = Vec::new();
        let mut pid = 0;
        let cmd = Comando {
            provider,
            bin: &bin,
            args: &args,
            env: &env,
            cwd: &dir,
        };
        let fim = run(
            cmd,
            Duration::from_secs(180),
            |p| {
                pid = p;
                true
            },
            |e| evs.push(e),
        )
        .await
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        println!(
            "bin {} pid {pid} saída {:?} stderr {:?}",
            bin.display(),
            fim.exit,
            fim.stderr
        );
        println!("eventos {:?}", resumo(&evs));
        assert!(pid > 1);
        assert!(
            evs.iter().any(|e| matches!(e, StreamEvent::Text(_))),
            "sem Text"
        );
        assert!(evs.contains(&StreamEvent::Done), "sem Done");
        assert_eq!(fim.exit, Some(0));
    }

    #[tokio::test]
    #[ignore = "roda o claude de verdade"]
    async fn real_claude_smoke() {
        smoke(AiProvider::Claude).await;
    }

    #[tokio::test]
    #[ignore = "roda o codex de verdade"]
    async fn real_codex_smoke() {
        smoke(AiProvider::Codex).await;
    }
}
