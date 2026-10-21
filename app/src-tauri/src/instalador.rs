//! Instala o daemon a partir do próprio Faro.app, como num app comum: baixar,
//! arrastar para Aplicativos, abrir e clicar em "Instalar" no Início.
//!
//! O daemon vai dentro do app (recurso `resources/faro-daemon`, posto lá pelo
//! build). A instalação roda como root pela janela nativa de senha do macOS
//! (`osascript ... with administrator privileges`). O script inteiro, com a
//! config e o LaunchDaemon, vai em memória para o osascript: nada é gravado num
//! arquivo temporário, que um processo do usuário poderia trocar antes de o
//! root executar (o tipo de ameaça que o Faro existe para pegar).

use std::path::Path;

pub const LABEL: &str = "dev.natan.vigia";
pub const BIN_DIR: &str = "/usr/local/faro";
pub const BIN: &str = "/usr/local/faro/faro-daemon";
pub const DATA_DIR: &str = "/Library/Application Support/Vigia";
pub const CONFIG: &str = "/Library/Application Support/Vigia/config.yaml";
pub const LOG_DIR: &str = "/Library/Logs/Vigia";
pub const PLIST_DESTINO: &str = "/Library/LaunchDaemons/dev.natan.vigia.plist";

const CONFIG_MODELO: &str = include_str!("../../../config/config.exemplo.yaml");
const PLIST: &str = include_str!("../../../launchd/dev.natan.vigia.plist");

/// A config inicial a partir do modelo. Sem pasta de terceiros: ela é escolhida
/// depois, no passo "Marcar a pasta de repositórios de terceiros" do Início.
pub fn render_config(home: &str, uid: u32, topico: &str) -> String {
    CONFIG_MODELO
        .replace("third_party_dirs:\n  - __TERCEIROS__", "third_party_dirs: []")
        .replace("__HOME__", home)
        .replace("__UID__", &uid.to_string())
        .replace("__TOPICO__", topico)
}

/// Tópico aleatório do ntfy (o celular está desligado; o campo só precisa existir).
pub fn topico_aleatorio() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut s = String::from("vigia-");
    while s.len() < 30 {
        let mut h = RandomState::new().build_hasher();
        h.write_usize(s.len());
        s.push_str(&format!("{:016x}", h.finish()));
    }
    s.truncate(30);
    s
}

/// Aspas simples do shell: `'` vira `'\''`.
fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// O script que roda como root. Config e plist vão em heredocs com delimitador
/// entre aspas (conteúdo literal, sem expansão).
pub fn script(daemon_src: &Path, config: &str) -> Result<String, String> {
    let src = daemon_src.to_str().ok_or("caminho do daemon com caracteres inválidos")?;
    for (nome, texto) in [("config", config), ("plist", PLIST)] {
        if texto.contains("FARO_FIM") {
            return Err(format!("{nome} contém o delimitador do script"));
        }
    }
    Ok(format!(
        r#"set -euo pipefail
install -d -o root -g wheel -m 755 {bin_dir}
install -o root -g wheel -m 755 {src} {bin}
install -d -o root -g wheel -m 700 {data}
install -d -o root -g wheel -m 755 {log}
if [ ! -f {config_path} ]; then
cat > {config_path} <<'FARO_FIM'
{config}
FARO_FIM
chown root:wheel {config_path}
chmod 600 {config_path}
fi
{bin} check --config {config_path} >/dev/null
cat > {plist_path} <<'FARO_FIM'
{plist}
FARO_FIM
chown root:wheel {plist_path}
chmod 644 {plist_path}
launchctl bootout system/{label} 2>/dev/null || true
rm -rf /usr/local/vigia
launchctl bootstrap system {plist_path}
"#,
        bin_dir = sh(BIN_DIR),
        src = sh(src),
        bin = sh(BIN),
        data = sh(DATA_DIR),
        log = sh(LOG_DIR),
        config_path = sh(CONFIG),
        config = config.trim_end(),
        plist_path = sh(PLIST_DESTINO),
        plist = PLIST.trim_end(),
        label = LABEL,
    ))
}

/// Texto entre aspas duplas do AppleScript: escapa `\` e `"`.
fn applescript_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// O comando do osascript: roda o script com a janela de senha do macOS.
pub fn osascript_arg(script: &str) -> String {
    format!(
        "do shell script \"/bin/bash -c \" & quoted form of {} with prompt {} with administrator privileges",
        applescript_str(script),
        applescript_str("O Faro precisa instalar o serviço que vigia o seu Mac."),
    )
}

/// Instala ou atualiza o daemon. Mantém a config existente (atualizar o app não
/// apaga seus ajustes).
#[cfg(target_os = "macos")]
pub fn instalar(daemon_src: &Path) -> Result<(), String> {
    if !daemon_src.is_file() {
        return Err("Este Faro.app não traz o serviço dentro (build de desenvolvimento). Use ./scripts/instalar.sh.".into());
    }
    let home = std::env::var("HOME").map_err(|_| "não achei a sua pasta de usuário (HOME)")?;
    // SAFETY: getuid não falha nem tem efeitos colaterais.
    let uid = unsafe { libc::getuid() };
    let config = render_config(&home, uid, &topico_aleatorio());
    let roteiro = script(daemon_src, &config)?;
    let saida = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(osascript_arg(&roteiro))
        .output()
        .map_err(|e| format!("não consegui pedir a senha: {e}"))?;
    if saida.status.success() {
        return Ok(());
    }
    let erro = String::from_utf8_lossy(&saida.stderr);
    // -128: o usuário cancelou a janela de senha.
    if erro.contains("-128") {
        return Err("Instalação cancelada.".into());
    }
    Err(format!("A instalação falhou: {}", erro.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_sem_terceiros_e_com_usuario() {
        let c = render_config("/Users/dev", 501, "vigia-abc");
        assert!(c.contains("third_party_dirs: []"));
        assert!(c.contains("home: /Users/dev"));
        assert!(c.contains("uid: 501"));
        assert!(c.contains("topic: \"vigia-abc\""));
        assert!(!c.contains("__"));
    }

    #[test]
    fn topico_longo_e_aleatorio() {
        let a = topico_aleatorio();
        assert!(a.starts_with("vigia-") && a.len() >= 16);
        assert_ne!(a, topico_aleatorio());
    }

    #[test]
    fn script_cita_os_caminhos() {
        let s = script(Path::new("/Applications/Faro.app/Contents/Resources/resources/faro-daemon"), "x: 1").unwrap();
        assert!(s.contains("install -o root -g wheel -m 755 '/Applications/Faro.app/Contents/Resources/resources/faro-daemon' '/usr/local/faro/faro-daemon'"));
        assert!(s.contains("'/Library/Application Support/Vigia/config.yaml'"));
        assert!(s.contains("launchctl bootstrap system '/Library/LaunchDaemons/dev.natan.vigia.plist'"));
        assert!(s.contains("<<'FARO_FIM'"));
        // O binário antigo (até a 0.1.0) sai depois de o serviço parar.
        assert!(s.find("bootout").unwrap() < s.find("rm -rf /usr/local/vigia\n").unwrap());
    }

    #[test]
    fn script_aguenta_aspas_no_caminho() {
        let s = script(Path::new("/Apps/O'Faro.app/vigia-daemon"), "x: 1").unwrap();
        assert!(s.contains("'/Apps/O'\\''Faro.app/vigia-daemon'"));
    }

    #[test]
    fn script_recusa_delimitador_na_config() {
        assert!(script(Path::new("/a"), "FARO_FIM").is_err());
    }

    #[test]
    fn osascript_escapa_aspas_e_barras() {
        let a = osascript_arg("echo \"oi\" \\ fim");
        assert!(a.contains("quoted form of \"echo \\\"oi\\\" \\\\ fim\""));
        assert!(a.ends_with("with administrator privileges"));
    }
}

