//! Preferências do app (som, fechamento automático, IA, marcas do checklist).
//! Ficam em `~/Library/Application Support/dev.natan.vigia/app.json`, gravadas
//! pelo próprio app (o daemon não participa). Arquivo ausente ou ilegível volta
//! ao padrão. Cada campo é lido sozinho: um campo ausente ou inválido (som
//! apagado, `auto_close_secs` editado à mão, IA desconhecida) volta ao padrão e
//! o resto do arquivo vale. Assim um campo ruim não trava todo salvamento.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub sound: SoundConfig,
    /// Segundos até a pílula recolher sozinha: 10, 15 ou 30.
    pub auto_close_secs: u32,
    pub ai: AiConfig,
    pub checklist: ChecklistMarks,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SoundConfig {
    pub enabled: bool,
    /// De 0.0 a 1.0.
    pub volume: f32,
    /// Som dos alertas crítico e alto (caminho absoluto).
    pub high: String,
    /// Som dos alertas médio e baixo (caminho absoluto).
    pub medium: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AiConfig {
    pub provider: AiProvider,
    /// Vazio = o modelo padrão do CLI.
    pub model: String,
    /// Quando o usuário aceitou o aviso de enviar dados ao Codex.
    pub codex_ack_at: Option<String>,
    pub claude_models: Vec<String>,
    pub codex_models: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    Claude,
    Codex,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct ChecklistMarks {
    pub reviewed_normal: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            sound: SoundConfig::default(),
            auto_close_secs: 15,
            ai: AiConfig::default(),
            checklist: ChecklistMarks::default(),
        }
    }
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 0.7,
            high: "/System/Library/Sounds/Sosumi.aiff".into(),
            medium: "/System/Library/Sounds/Tink.aiff".into(),
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProvider::Claude,
            model: String::new(),
            codex_ack_at: None,
            claude_models: ["fable", "opus", "sonnet", "haiku"].map(String::from).to_vec(),
            codex_models: Vec::new(),
        }
    }
}

pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/Application Support/dev.natan.vigia/app.json")
}

/// Lê as preferências. Ausente vira o padrão; arquivo ilegível também (com aviso
/// no log). Campo a campo, o que não ler ou não passar na validação vira o
/// padrão daquele campo (com aviso), sem perder os outros.
pub fn load(path: &Path) -> AppConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return AppConfig::default(),
        Err(e) => {
            eprintln!("aviso: não consegui ler {}: {e}; usando o padrão", path.display());
            return AppConfig::default();
        }
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => from_map(&m, &path.display().to_string()),
        Ok(_) => {
            eprintln!("aviso: {} não é um objeto JSON; usando o padrão", path.display());
            AppConfig::default()
        }
        Err(e) => {
            eprintln!("aviso: {} inválido ({e}); usando o padrão", path.display());
            AppConfig::default()
        }
    }
}

/// Lê `chave` de `m`; ausente vira `padrao` em silêncio, inválido (não lê ou
/// `ok` recusa) vira `padrao` com aviso.
fn campo<T: DeserializeOwned>(
    m: &Map<String, Value>,
    chave: &str,
    padrao: T,
    ok: impl Fn(&T) -> bool,
    onde: &str,
) -> T {
    let Some(v) = m.get(chave) else {
        return padrao;
    };
    match serde_json::from_value::<T>(v.clone()) {
        Ok(x) if ok(&x) => x,
        _ => {
            eprintln!("aviso: {onde}: valor inválido em \"{chave}\" ({v}); usando o padrão");
            padrao
        }
    }
}

/// Objeto aninhado (`sound`, `ai`, `checklist`); não sendo objeto, fica vazio (tudo padrão).
fn objeto(m: &Map<String, Value>, chave: &str, onde: &str) -> Map<String, Value> {
    match m.get(chave) {
        Some(Value::Object(o)) => o.clone(),
        None => Map::new(),
        Some(v) => {
            eprintln!("aviso: {onde}: \"{chave}\" não é um objeto ({v}); usando o padrão");
            Map::new()
        }
    }
}

/// Campo sem regra além do tipo.
fn sempre<T>(_: &T) -> bool {
    true
}

fn from_map(m: &Map<String, Value>, onde: &str) -> AppConfig {
    let d = AppConfig::default();

    let s = objeto(m, "sound", onde);
    let som_ok = |p: &String| validate_sound("", p).is_ok();
    let sound = SoundConfig {
        enabled: campo(&s, "enabled", d.sound.enabled, sempre, onde),
        volume: campo(&s, "volume", d.sound.volume, |v: &f32| (0.0..=1.0).contains(v), onde),
        high: campo(&s, "high", d.sound.high.clone(), som_ok, onde),
        medium: campo(&s, "medium", d.sound.medium.clone(), som_ok, onde),
    };

    let a = objeto(m, "ai", onde);
    let ai = AiConfig {
        provider: campo(&a, "provider", d.ai.provider, sempre, onde),
        model: campo(&a, "model", d.ai.model.clone(), sempre, onde),
        codex_ack_at: campo(&a, "codex_ack_at", d.ai.codex_ack_at.clone(), sempre, onde),
        claude_models: campo(&a, "claude_models", d.ai.claude_models.clone(), sempre, onde),
        codex_models: campo(&a, "codex_models", d.ai.codex_models.clone(), sempre, onde),
    };

    let c = objeto(m, "checklist", onde);
    let checklist = ChecklistMarks {
        reviewed_normal: campo(&c, "reviewed_normal", d.checklist.reviewed_normal, sempre, onde),
    };

    AppConfig {
        sound,
        auto_close_secs: campo(m, "auto_close_secs", d.auto_close_secs, |v: &u32| auto_close_ok(*v), onde),
        ai,
        checklist,
    }
}

const AUDIO_EXTS: [&str; 6] = ["aiff", "aif", "wav", "mp3", "m4a", "caf"];

pub(crate) fn validate_sound(label: &str, path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err(format!("Escolha um som para {label}."));
    }
    let p = Path::new(path);
    let ext_ok = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()));
    if !ext_ok {
        return Err(format!("{path} não é um arquivo de áudio (use aiff, wav, mp3, m4a ou caf)."));
    }
    if !p.is_file() {
        return Err(format!("O arquivo de som {path} não existe."));
    }
    Ok(())
}

fn auto_close_ok(v: u32) -> bool {
    matches!(v, 10 | 15 | 30)
}

/// Valida as preferências novas contra as que valem agora. Um som que não mudou
/// não é checado de novo: se o arquivo sumiu com o app aberto, mudar outra coisa
/// (alto-falante, aceite do Codex, marcas) continua funcionando.
pub fn validate(cfg: &AppConfig, atual: &AppConfig) -> Result<(), String> {
    if !auto_close_ok(cfg.auto_close_secs) {
        return Err("O tempo para recolher a pílula deve ser 10, 15 ou 30 segundos.".into());
    }
    if !(0.0..=1.0).contains(&cfg.sound.volume) {
        return Err("O volume deve ficar entre 0 e 1.".into());
    }
    if cfg.sound.high != atual.sound.high {
        validate_sound("alertas críticos e altos", &cfg.sound.high)?;
    }
    if cfg.sound.medium != atual.sound.medium {
        validate_sound("alertas médios e baixos", &cfg.sound.medium)?;
    }
    Ok(())
}

/// Valida (contra `atual`) e grava de forma atômica (arquivo temporário e rename).
pub fn save(path: &Path, cfg: &AppConfig, atual: &AppConfig) -> Result<(), String> {
    validate(cfg, atual)?;
    let dir = path.parent().ok_or("Caminho de preferências inválido.")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Não consegui criar {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("Não consegui gravar {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Não consegui gravar {}: {e}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vigia-app-config-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(load(Path::new("/nao/existe/app.json")), AppConfig::default());
    }

    #[test]
    fn corrupt_file_gives_defaults() {
        let p = tmpdir("corrupt").join("app.json");
        std::fs::write(&p, "{{").unwrap();
        assert_eq!(load(&p), AppConfig::default());
    }

    #[test]
    fn old_file_fills_new_fields() {
        let p = tmpdir("old").join("app.json");
        std::fs::write(&p, r#"{"auto_close_secs": 30}"#).unwrap();
        let c = load(&p);
        assert_eq!(c.auto_close_secs, 30);
        assert_eq!(c, AppConfig { auto_close_secs: 30, ..AppConfig::default() });
    }

    #[test]
    fn rejects_bad_auto_close() {
        let c = AppConfig { auto_close_secs: 20, ..AppConfig::default() };
        assert!(validate(&c, &AppConfig::default()).unwrap_err().contains("10, 15 ou 30"));
    }

    #[test]
    fn rejects_volume_out_of_range() {
        for v in [1.5, -0.1] {
            let mut c = AppConfig::default();
            c.sound.volume = v;
            assert!(validate(&c, &AppConfig::default()).is_err(), "volume {v}");
        }
    }

    #[test]
    fn rejects_sound_that_is_not_audio() {
        let mut c = AppConfig::default();
        c.sound.high = "/etc/hosts".into();
        assert!(validate(&c, &AppConfig::default()).unwrap_err().contains("áudio"));
    }

    #[test]
    fn rejects_missing_sound_file() {
        let mut c = AppConfig::default();
        c.sound.high = "/nao/existe.aiff".into();
        assert!(validate(&c, &AppConfig::default()).is_err());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let d = tmpdir("roundtrip");
        let p = d.join("sub/app.json");
        let snd = d.join("som.WAV");
        std::fs::write(&snd, b"x").unwrap();
        let mut c = AppConfig { auto_close_secs: 10, ..AppConfig::default() };
        c.sound.medium = snd.to_string_lossy().into();
        c.ai.provider = AiProvider::Codex;
        c.checklist.reviewed_normal = true;
        save(&p, &c, &AppConfig::default()).unwrap();
        assert_eq!(load(&p), c);
    }

    /// Grava `json` num app.json temporário e lê de volta.
    fn ler(nome: &str, json: &str) -> AppConfig {
        let p = tmpdir(nome).join("app.json");
        std::fs::write(&p, json).unwrap();
        load(&p)
    }

    #[test]
    fn missing_sound_file_falls_back_and_keeps_the_rest() {
        let c = ler(
            "som-sumiu",
            r#"{"sound":{"enabled":false,"volume":0.3,"high":"/nao/existe/meu.mp3"},
               "auto_close_secs":30,"checklist":{"reviewed_normal":true},
               "ai":{"provider":"codex","codex_ack_at":"2026-10-19T12:00:00Z"}}"#,
        );
        assert_eq!(c.sound.high, SoundConfig::default().high);
        assert!(!c.sound.enabled);
        assert_eq!(c.sound.volume, 0.3);
        assert_eq!(c.auto_close_secs, 30);
        assert!(c.checklist.reviewed_normal);
        assert_eq!(c.ai.provider, AiProvider::Codex);
        assert_eq!(c.ai.codex_ack_at.as_deref(), Some("2026-10-19T12:00:00Z"));
        // E o que foi lido salva de novo.
        assert_eq!(validate(&c, &c), Ok(()));
    }

    #[test]
    fn bad_auto_close_falls_back_and_keeps_the_rest() {
        let c = ler(
            "vinte",
            r#"{"auto_close_secs":20,"sound":{"volume":7},"campo_antigo":"x",
               "checklist":{"reviewed_normal":true}}"#,
        );
        assert_eq!(c.auto_close_secs, 15);
        assert_eq!(c.sound.volume, SoundConfig::default().volume);
        assert!(c.checklist.reviewed_normal);
        assert_eq!(validate(&c, &c), Ok(()));
    }

    #[test]
    fn unknown_provider_falls_back_and_keeps_ack_and_marks() {
        let c = ler(
            "gemini",
            r#"{"ai":{"provider":"gemini","model":"x","codex_ack_at":"2026-10-19T12:00:00Z",
               "codex_models":["gpt-5"]},"checklist":{"reviewed_normal":true},
               "auto_close_secs":10}"#,
        );
        assert_eq!(c.ai.provider, AiProvider::Claude);
        assert_eq!(c.ai.model, "x");
        assert_eq!(c.ai.codex_ack_at.as_deref(), Some("2026-10-19T12:00:00Z"));
        assert_eq!(c.ai.codex_models, ["gpt-5"]);
        assert!(c.checklist.reviewed_normal);
        assert_eq!(c.auto_close_secs, 10);
    }

    #[test]
    fn wrong_types_fall_back_per_field() {
        let c = ler("tipos", r#"{"sound":"x","ai":{"claude_models":"opus"},"auto_close_secs":"x"}"#);
        assert_eq!(c, AppConfig::default());
        assert_eq!(ler("lista", "[1,2]"), AppConfig::default());
    }

    #[test]
    fn unchanged_missing_sound_does_not_block_saving() {
        let mut atual = AppConfig::default();
        atual.sound.high = "/nao/existe/meu.mp3".into();
        let mut novo = atual.clone();
        novo.sound.enabled = false;
        novo.checklist.reviewed_normal = true;
        assert_eq!(validate(&novo, &atual), Ok(()));
        // Escolher agora um som que não existe continua recusado.
        novo.sound.medium = "/nao/existe/outro.mp3".into();
        assert!(validate(&novo, &atual).is_err());
    }
}
