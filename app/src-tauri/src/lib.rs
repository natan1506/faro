//! App do Vigia: a raposinha na barra de menus, a pílula de alerta no topo da tela e
//! painel com o histórico. Toda a lógica de detecção fica no daemon; o app só
//! mostra, pergunta e repassa as decisões. A exceção é a checagem de leitores
//! de teclado (event taps), que só funciona dentro da sessão do usuário.

mod ai;
mod app_config;
mod client;
mod notch;
mod sound;
#[cfg(target_os = "macos")]
mod taps;

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use vigia_proto::{
    ActionKind, Alert, AlertStatus, AllowView, ClientMsg, ExecEntry, ServerMsg, Settings, Status,
};

// A raposinha da barra de menus, nas três expressões. São imagens "template"
// (preto + transparência, geradas das imagens do Natan): o macOS pinta de
// branco ou preto conforme o tema, com olhos, nariz e contorno vazados.
const TRAY_CALM: &[u8] = include_bytes!("../icons/tray.png");
const TRAY_SLEEP: &[u8] = include_bytes!("../icons/tray-sleep.png");
const TRAY_ALERT: &[u8] = include_bytes!("../icons/tray-alert.png");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TrayFrame {
    Calm,
    Sleep,
    Alert,
}

impl TrayFrame {
    fn bytes(self) -> &'static [u8] {
        match self {
            TrayFrame::Calm => TRAY_CALM,
            TrayFrame::Sleep => TRAY_SLEEP,
            TrayFrame::Alert => TRAY_ALERT,
        }
    }

    /// Dorme sem daemon, fica assustada com alerta novo, calma no resto.
    fn pick(connected: bool, unseen: usize) -> TrayFrame {
        if !connected {
            TrayFrame::Sleep
        } else if unseen > 0 {
            TrayFrame::Alert
        } else {
            TrayFrame::Calm
        }
    }
}

#[derive(Default)]
struct AppState {
    /// Preferências do usuário (`app.json`).
    config: Mutex<app_config::AppConfig>,
    /// Evita som repetido numa rajada de alertas.
    sound_gate: Mutex<sound::SoundGate>,
    connected: Mutex<bool>,
    /// Alertas abertos que você ainda não viu (deixam o ícone vermelho).
    unseen: Mutex<HashSet<i64>>,
    /// Ids já recebidos: atualizações de um alerta conhecido não reabrem o cartão.
    known: Mutex<HashSet<i64>>,
    tray: Mutex<Option<TrayIcon>>,
    status_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    notch_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    /// Conversa com a IA (só em memória).
    chat: Mutex<ai::Conversation>,
}

#[derive(Serialize, Clone)]
struct AlertEvent {
    alert: Alert,
    is_new: bool,
}

// ---------------------------------------------------------------------------
// Eventos vindos do daemon

pub(crate) fn on_alert(app: &AppHandle, alert: Alert) {
    let state = app.state::<AppState>();
    let is_new = alert.id == 0 || state.known.lock().unwrap().insert(alert.id);
    {
        let mut unseen = state.unseen.lock().unwrap();
        if alert.status == AlertStatus::Open {
            if is_new && alert.id > 0 {
                unseen.insert(alert.id);
            }
        } else {
            unseen.remove(&alert.id);
        }
    }
    refresh_tray(app);
    {
        let cfg = state.config.lock().unwrap().sound.clone();
        let kind = state
            .sound_gate
            .lock()
            .unwrap()
            .should_play(std::time::Instant::now(), is_new, &alert, cfg.enabled);
        match kind {
            Some(sound::SoundKind::High) => sound::play(&cfg.high, cfg.volume),
            Some(sound::SoundKind::Medium) => sound::play(&cfg.medium, cfg.volume),
            None => {}
        }
    }
    let open = alert.status == AlertStatus::Open;
    let _ = app.emit("vigia:alert", AlertEvent { alert, is_new });
    // Alerta novo aparece mesmo se você escondeu a raposinha do topo.
    if is_new && open && !notch::is_visible(app) {
        notch::set_visible(app, true);
        if let Some(item) = app.state::<AppState>().notch_item.lock().unwrap().as_ref() {
            let _ = item.set_checked(true);
        }
    }
}

pub(crate) fn set_connected(app: &AppHandle, connected: bool) {
    let state = app.state::<AppState>();
    let changed = {
        let mut c = state.connected.lock().unwrap();
        let changed = *c != connected;
        *c = connected;
        changed
    };
    if changed {
        let _ = app.emit("vigia:daemon", connected);
        if let Some(item) = state.status_item.lock().unwrap().as_ref() {
            let _ = item.set_text(if connected {
                "Vigiando"
            } else {
                "Daemon desconectado"
            });
        }
        refresh_tray(app);
    }
}

fn refresh_tray(app: &AppHandle) {
    let state = app.state::<AppState>();
    let unseen = state.unseen.lock().unwrap().len();
    let connected = *state.connected.lock().unwrap();
    let tray = state.tray.lock().unwrap();
    let Some(tray) = tray.as_ref() else { return };
    let tip = match (connected, unseen) {
        (false, _) => "Faro: o daemon não está respondendo".to_string(),
        (true, 0) => "Faro: tudo calmo".to_string(),
        (true, 1) => "Faro: 1 alerta novo".to_string(),
        (true, n) => format!("Faro: {n} alertas novos"),
    };
    let _ = tray.set_tooltip(Some(&tip));
}

/// Mantém a raposinha da barra de menus no humor certo (troca só quando muda).
async fn tray_loop(app: AppHandle) {
    let mut last: Option<TrayFrame> = None;
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let frame = {
            let state = app.state::<AppState>();
            let connected = *state.connected.lock().unwrap();
            let unseen = state.unseen.lock().unwrap().len();
            TrayFrame::pick(connected, unseen)
        };
        if last == Some(frame) {
            continue;
        }
        let state = app.state::<AppState>();
        let tray = state.tray.lock().unwrap();
        if let (Some(tray), Ok(img)) = (tray.as_ref(), Image::from_bytes(frame.bytes())) {
            let _ = tray.set_icon(Some(img));
            let _ = tray.set_icon_as_template(true);
            last = Some(frame);
        }
    }
}

fn show_painel(app: &AppHandle) {
    // Quem abre o painel quer ficar nele: a pílula não devolve mais o foco ao app anterior.
    notch::esquecer_anterior();
    if let Some(w) = app.get_webview_window("painel") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn forget(app: &AppHandle, id: i64) {
    app.state::<AppState>().unseen.lock().unwrap().remove(&id);
    refresh_tray(app);
}

// ---------------------------------------------------------------------------
// Ações locais (não passam pelo daemon)

fn open(args: &[&str]) -> Result<(), String> {
    let st = std::process::Command::new("/usr/bin/open")
        .args(args)
        .status()
        .map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("open {args:?} falhou"))
    }
}

/// Página de Ajustes > Privacidade e Segurança certa para cada permissão.
fn settings_url(rule: &str, target: Option<&str>) -> &'static str {
    match (rule, target) {
        ("keyboard_tap", _) | (_, Some("kTCCServiceListenEvent")) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
        }
        (_, Some("kTCCServiceScreenCapture")) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        (_, Some("kTCCServiceAccessibility")) | (_, Some("kTCCServicePostEvent")) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        _ => "x-apple.systempreferences:com.apple.preference.security",
    }
}

// ---------------------------------------------------------------------------
// Comandos chamados pelas janelas

#[tauri::command]
async fn history(limit: u32) -> Result<Vec<Alert>, String> {
    client::request(&ClientMsg::History { limit }, |m| match m {
        ServerMsg::History { alerts } => Some(alerts),
        _ => None,
    })
    .await
}

#[tauri::command]
async fn timeline(limit: u32) -> Result<Vec<ExecEntry>, String> {
    client::request(&ClientMsg::Timeline { limit }, |m| match m {
        ServerMsg::Timeline { entries } => Some(entries),
        _ => None,
    })
    .await
}

#[tauri::command]
async fn status() -> Result<Status, String> {
    client::request(&ClientMsg::Status, |m| match m {
        ServerMsg::Status { status } => Some(status),
        _ => None,
    })
    .await
}

#[tauri::command]
async fn act(
    app: AppHandle,
    alert_id: i64,
    action: ActionKind,
    target: Option<String>,
    rule: String,
) -> Result<String, String> {
    match action {
        ActionKind::Reveal => {
            let t = target.ok_or("o alerta não tem arquivo")?;
            if !std::path::Path::new(&t).exists() {
                return Err(format!("{t} não existe mais"));
            }
            open(&["-R", &t])?;
            return Ok("mostrado no Finder".into());
        }
        ActionKind::OpenSettings => {
            open(&[settings_url(&rule, target.as_deref())])?;
            return Ok("Ajustes abertos".into());
        }
        _ => {}
    }
    let (ok, message) = client::request(&ClientMsg::Act { alert_id, action }, |m| match m {
        ServerMsg::ActionResult {
            alert_id: id,
            ok,
            message,
            ..
        } if id == alert_id => Some((ok, message)),
        _ => None,
    })
    .await?;
    forget(&app, alert_id);
    if ok {
        Ok(message)
    } else {
        Err(message)
    }
}

#[tauri::command]
fn mark_seen(app: AppHandle, ids: Vec<i64>) {
    {
        let state = app.state::<AppState>();
        let mut u = state.unseen.lock().unwrap();
        for id in ids {
            u.remove(&id);
        }
    }
    refresh_tray(&app);
}

#[tauri::command]
fn daemon_connected(app: AppHandle) -> bool {
    *app.state::<AppState>().connected.lock().unwrap()
}

#[tauri::command]
fn open_painel(app: AppHandle, alert_id: Option<i64>) {
    show_painel(&app);
    if let Some(id) = alert_id {
        let _ = app.emit_to("painel", "vigia:focus-alert", id);
    }
}

/// Abre um link no navegador ou nos Ajustes do macOS. Só aceita esses dois tipos.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url_permitida(&url) {
        return Err("Só abro links https:// ou dos Ajustes do macOS".into());
    }
    open(&[&url])
}

/// Limite de segurança do `open_url`: só `https://` e `x-apple.systempreferences:`.
/// A comparação é sensível a maiúsculas e não tira espaços: na dúvida, recusa
/// (evita `javascript:`, `file://`, opções do `open` como `-a` e variações de grafia).
fn url_permitida(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("x-apple.systempreferences:")
}

/// Abre o painel direto na aba de Ajustes (engrenagem da pílula, menu da barra).
#[tauri::command]
fn open_settings(app: AppHandle) {
    show_settings(&app);
}

/// Abre o painel na seção Conversar ("Abrir no painel" da conversa da pílula).
#[tauri::command]
fn open_conversa(app: AppHandle) {
    show_painel(&app);
    let _ = app.emit_to("painel", "vigia:open-conversa", ());
}

fn show_settings(app: &AppHandle) {
    show_painel(app);
    let _ = app.emit_to("painel", "vigia:open-settings", ());
}

#[tauri::command]
async fn get_settings() -> Result<Settings, String> {
    client::request(&ClientMsg::GetSettings, |m| match m {
        ServerMsg::Settings { settings } => Some(settings),
        _ => None,
    })
    .await
}

/// Exceções em vigor no daemon (só leitura).
#[tauri::command]
async fn allowlist() -> Result<Vec<AllowView>, String> {
    client::request(&ClientMsg::Allowlist, |m| match m {
        ServerMsg::Allowlist { entries } => Some(entries),
        _ => None,
    })
    .await
}

/// Devolve os ajustes que ficaram valendo; em erro, a mensagem do daemon.
#[tauri::command]
async fn save_settings(settings: Settings) -> Result<Settings, String> {
    let (ok, message, saved) =
        client::request(&ClientMsg::SaveSettings { settings }, |m| match m {
            ServerMsg::SettingsSaved {
                ok,
                message,
                settings,
            } => Some((ok, message, settings)),
            _ => None,
        })
        .await?;
    if ok {
        Ok(saved)
    } else {
        Err(message)
    }
}

/// Preferências do usuário (som, fechamento automático, IA, checklist).
#[tauri::command]
fn get_app_config(state: tauri::State<'_, AppState>) -> app_config::AppConfig {
    state.config.lock().unwrap().clone()
}

/// Valida, grava em `app.json` e avisa as duas janelas.
#[tauri::command]
fn save_app_config(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    config: app_config::AppConfig,
) -> Result<app_config::AppConfig, String> {
    // Grava segurando o lock (o aceite do Codex faz o mesmo): um não apaga o outro.
    {
        let mut atual = state.config.lock().unwrap();
        app_config::save(&app_config::config_path(), &config, &atual)?;
        *atual = config.clone();
    }
    let _ = app.emit("vigia:app-config", config.clone());
    Ok(config)
}

/// Seletor de pasta do macOS. Devolve o caminho com `~` no lugar da home.
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Escolha uma pasta")
        .pick_folder(move |p| {
            let _ = tx.send(p.and_then(|p| p.into_path().ok()));
        });
    let path = rx.await.ok().flatten()?;
    let path = path.to_string_lossy().to_string();
    let home = std::env::var("HOME").unwrap_or_default();
    Some(match path.strip_prefix(&format!("{home}/")) {
        Some(rest) if !home.is_empty() => format!("~/{rest}"),
        _ => path,
    })
}

/// Sons que vêm com o macOS.
#[tauri::command]
fn list_system_sounds() -> Vec<String> {
    sound::system_sounds()
}

/// Toca um som para você ouvir antes de escolher.
#[tauri::command]
fn preview_sound(path: String, volume: f32) -> Result<(), String> {
    if !(0.0..=1.0).contains(&volume) {
        return Err("O volume precisa ficar entre 0 e 1.".into());
    }
    app_config::validate_sound("o teste", &path)?;
    sound::play(&path, volume);
    Ok(())
}

/// Seletor de arquivo de áudio. Devolve o caminho completo.
#[tauri::command]
async fn pick_sound_file(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Escolha um som")
        .add_filter("Áudio", &["aiff", "aif", "wav", "mp3", "m4a", "caf"])
        .pick_file(move |p| {
            let _ = tx.send(p.and_then(|p| p.into_path().ok()));
        });
    let path = rx.await.ok().flatten()?;
    Some(path.to_string_lossy().to_string())
}

// ---------------------------------------------------------------------------

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Conectando ao daemon…", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Abrir painel", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Ajustes…", true, Some("CmdOrCtrl+,"))?;
    let notch_item = CheckMenuItem::with_id(
        app,
        "notch",
        "Raposinha no topo da tela",
        true,
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Sair do Faro", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&status, &sep, &open, &settings, &notch_item, &sep2, &quit],
    )?;
    let tray = TrayIconBuilder::with_id("vigia")
        .icon(Image::from_bytes(TRAY_SLEEP)?)
        .icon_as_template(true)
        .tooltip("Faro")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_painel(app),
            "settings" => show_settings(app),
            "notch" => {
                let checked = app
                    .state::<AppState>()
                    .notch_item
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|i| i.is_checked().ok())
                    .unwrap_or(true);
                notch::set_visible(app, checked);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    let state = app.state::<AppState>();
    *state.tray.lock().unwrap() = Some(tray);
    *state.status_item.lock().unwrap() = Some(status);
    *state.notch_item.lock().unwrap() = Some(notch_item);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            history,
            timeline,
            status,
            act,
            mark_seen,
            daemon_connected,
            open_painel,
            open_settings,
            open_conversa,
            open_url,
            get_settings,
            allowlist,
            save_settings,
            pick_folder,
            get_app_config,
            save_app_config,
            list_system_sounds,
            preview_sound,
            pick_sound_file,
            notch::notch_metrics,
            notch::notch_hit_area,
            notch::notch_focusable,
            notch::notch_cursor,
            ai::ai_providers,
            ai::ai_conversation,
            ai::ai_ask,
            ai::ai_stop,
            ai::ai_clear,
            ai::ai_accept_codex
        ])
        .setup(|app| {
            // Sem ícone no Dock: o Vigia vive na barra de menus.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            *app.state::<AppState>().config.lock().unwrap() =
                app_config::load(&app_config::config_path());
            build_tray(app)?;
            notch::setup(app.handle());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(tray_loop(handle.clone()));
            tauri::async_runtime::spawn(client::subscribe_loop(handle.clone()));
            #[cfg(target_os = "macos")]
            tauri::async_runtime::spawn(taps::watch(handle));
            Ok(())
        })
        .on_window_event(|window, event| {
            // Fechar o painel só esconde; o Vigia continua na barra de menus.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "painel" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("erro ao iniciar o Faro")
        .run(|app, event| {
            // Nenhum `claude`/`codex` da conversa fica rodando depois que o app sai.
            if let tauri::RunEvent::Exit = event {
                ai::shutdown(app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{settings_url, url_permitida, TrayFrame};

    #[test]
    fn open_url_so_aceita_https_e_ajustes() {
        for ok in [
            "https://example.com",
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
        ] {
            assert!(url_permitida(ok), "{ok}");
        }
        for ruim in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            " https://x",
            "HTTPS://x",
            "-a",
            "x-apple.systempreferences",
            "http://example.com",
            "",
        ] {
            assert!(!url_permitida(ruim), "{ruim:?}");
        }
    }

    #[test]
    fn tray_frames() {
        assert_eq!(TrayFrame::pick(false, 3), TrayFrame::Sleep);
        assert_eq!(TrayFrame::pick(true, 1), TrayFrame::Alert);
        assert_eq!(TrayFrame::pick(true, 0), TrayFrame::Calm);
    }

    #[test]
    fn settings_pages() {
        assert!(settings_url("keyboard_tap", Some("teclado")).ends_with("Privacy_ListenEvent"));
        assert!(settings_url("tcc_change", Some("kTCCServiceScreenCapture"))
            .ends_with("Privacy_ScreenCapture"));
        assert!(settings_url("tcc_change", Some("kTCCServicePostEvent"))
            .ends_with("Privacy_Accessibility"));
    }
}
