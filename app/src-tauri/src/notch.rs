//! A janela do topo da tela ("notch"): pílula preta centralizada, grudada na
//! borda de cima, por cima da barra de menus. Em Macs com notch ela abraça o
//! recorte da câmera; nos outros, desenha o seu próprio.

use serde::Serialize;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow};

pub const LABEL: &str = "notch";

#[derive(Serialize, Clone, Copy, Debug)]
pub struct Metrics {
    /// Altura da barra de menus em pontos.
    pub menubar: f64,
    /// Tela com notch físico (a barra de menus é mais alta nesses Macs: ~37 pt contra ~24 pt).
    pub has_notch: bool,
}

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

pub fn metrics(w: &WebviewWindow) -> Metrics {
    let Ok(Some(m)) = w.primary_monitor() else {
        return Metrics {
            menubar: 24.0,
            has_notch: false,
        };
    };
    let scale = m.scale_factor();
    let bar_px = (m.work_area().position.y - m.position().y).max(0) as f64;
    // Barra de menus escondida automaticamente: a área útil começa no topo.
    let menubar = if bar_px > 0.0 { bar_px / scale } else { 24.0 };
    Metrics {
        menubar: menubar.clamp(22.0, 44.0),
        has_notch: menubar >= 32.0,
    }
}

/// Tamanho fixo da janela, em pontos: cabe a maior vista (alerta, 480 + orelhas).
///
/// A janela não muda de tamanho com a pílula: janela transparente que encolhe
/// fica um quadro em branco no macOS, e a pílula piscava ao fechar. No lugar
/// disso, os cliques atravessam a janela fora da pílula (`watch_cursor`).
const WIDTH: f64 = 520.0;
const HEIGHT: f64 = 420.0;

/// Área da pílula (largura e altura em pontos, centrada no topo da janela)
/// que recebe o mouse. O resto da janela deixa os cliques passarem.
static HIT_AREA: Mutex<(f64, f64)> = Mutex::new((228.0, 24.0));

/// Põe a janela no tamanho fixo, centralizada no topo da tela principal.
fn place(w: &WebviewWindow) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // O AppKit só aceita mexer na janela pela thread principal.
        if objc2::MainThreadMarker::new().is_none() {
            let win = w.clone();
            return w.run_on_main_thread(move || {
                let _ = place(&win);
            });
        }
        if mac::set_frame_top_center(w, WIDTH, HEIGHT) {
            return Ok(());
        }
    }
    w.set_size(LogicalSize::new(WIDTH, HEIGHT))?;
    if let Ok(Some(m)) = w.primary_monitor() {
        let scale = m.scale_factor();
        let x = m.position().x as f64 + (m.size().width as f64 - WIDTH * scale) / 2.0;
        w.set_position(PhysicalPosition::new(x.round() as i32, m.position().y))?;
    }
    Ok(())
}

/// A janela saiu do lugar (outra tela principal, resolução nova)?
fn misplaced(w: &WebviewWindow) -> bool {
    let (Ok(Some(m)), Ok(pos), Ok(size)) = (w.primary_monitor(), w.outer_position(), w.outer_size())
    else {
        return false;
    };
    let scale = m.scale_factor();
    let x = m.position().x as f64 + (m.size().width as f64 - WIDTH * scale) / 2.0;
    (pos.x as f64 - x).abs() > 2.0
        || pos.y != m.position().y
        || (size.width as f64 - WIDTH * scale).abs() > 2.0
        || (size.height as f64 - HEIGHT * scale).abs() > 2.0
}

pub fn setup(app: &AppHandle) {
    let Some(w) = window(app) else { return };
    let m = metrics(&w);
    *HIT_AREA.lock().unwrap() = (if m.has_notch { 358.0 } else { 228.0 }, m.menubar);
    let _ = place(&w);
    let _ = w.set_ignore_cursor_events(true);
    #[cfg(target_os = "macos")]
    mac::float_above_menu_bar(&w);
    let _ = w.show();
    let handle = app.clone();
    #[cfg(target_os = "macos")]
    {
        let h2 = handle.clone();
        std::thread::spawn(move || hold_cursor(h2));
    }
    std::thread::spawn(move || watch_cursor(handle));
}

/// Olha o mouse ~25 vezes por segundo. Em cima da pílula, a janela recebe os
/// cliques; fora dela, eles atravessam para o que estiver embaixo. Avisa a
/// pílula quando o mouse entra e sai (o `mouseleave` não é confiável numa
/// janela que não recebe foco) e manda a posição para a raposinha olhar.
fn watch_cursor(app: AppHandle) {
    let mut inside_before: Option<bool> = None;
    let mut last_look: Option<(i32, i32)> = None;
    let mut tick: u32 = 0;
    loop {
        let passo = if inside_before == Some(true) { 16 } else { 40 };
        std::thread::sleep(std::time::Duration::from_millis(passo));
        tick = tick.wrapping_add(1);
        let Some(w) = window(&app) else {
            esquecer_mouse(&mut inside_before);
            continue;
        };
        if !w.is_visible().unwrap_or(false) {
            // Escondida com o mouse em cima: avisa a pílula que ele saiu.
            if inside_before == Some(true) {
                let _ = w.emit_to(LABEL, "vigia:notch-hover", false);
            }
            esquecer_mouse(&mut inside_before);
            continue;
        }
        if tick.is_multiple_of(25) && misplaced(&w) {
            let _ = place(&w);
        }
        let (Ok(c), Ok(pos), Ok(scale)) = (app.cursor_position(), w.outer_position(), w.scale_factor())
        else {
            continue;
        };
        let (hw, hh) = *HIT_AREA.lock().unwrap();
        let (hw, hh) = (hw.min(WIDTH) * scale, hh.min(HEIGHT) * scale);
        let left = pos.x as f64 + (WIDTH * scale - hw) / 2.0;
        let top = pos.y as f64;
        let inside = c.x >= left && c.x < left + hw && c.y >= top && c.y < top + hh;
        if inside_before != Some(inside) {
            inside_before = Some(inside);
            let _ = w.set_ignore_cursor_events(!inside);
            let _ = w.emit_to(LABEL, "vigia:notch-hover", inside);
            SOBRE_A_PILULA.store(inside, std::sync::atomic::Ordering::Relaxed);
            // Saiu: esquece o pedido (sem aplicar nada; o app de baixo cuida do
            // cursor). Ao voltar, a pílula pede de novo e combina com o JS, que
            // também volta a "seta" ao sair.
            if !inside {
                *CURSOR_PEDIDO.lock().unwrap() = CursorKind::Seta;
            }
        }
        // Posição do mouse em pontos, relativa ao centro do topo da janela.
        let look = (
            ((c.x - (pos.x as f64 + WIDTH * scale / 2.0)) / scale).round() as i32,
            ((c.y - top) / scale).round() as i32,
        );
        if last_look != Some(look) {
            last_look = Some(look);
            let _ = w.emit_to(LABEL, "vigia:cursor", look);
        }
    }
}

/// A pílula sumiu (janela escondida ou fechada): o mouse não está mais sobre ela.
/// Sem isso o `hold_cursor` continuaria forçando o cursor pedido sobre todos os
/// apps. `inside_before = None` faz a próxima volta reenviar o ignore e o hover.
fn esquecer_mouse(inside_before: &mut Option<bool>) {
    SOBRE_A_PILULA.store(false, std::sync::atomic::Ordering::Relaxed);
    *CURSOR_PEDIDO.lock().unwrap() = CursorKind::Seta;
    *inside_before = None;
}

pub fn set_visible(app: &AppHandle, visible: bool) {
    if let Some(w) = window(app) {
        let _ = if visible { w.show() } else { w.hide() };
    }
}

pub fn is_visible(app: &AppHandle) -> bool {
    window(app)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

#[tauri::command]
pub fn notch_metrics(app: AppHandle) -> Metrics {
    window(&app).map(|w| metrics(&w)).unwrap_or(Metrics {
        menubar: 24.0,
        has_notch: false,
    })
}

/// A pílula avisa o tamanho que ocupa; só essa área recebe o mouse.
#[tauri::command]
pub fn notch_hit_area(width: f64, height: f64) {
    *HIT_AREA.lock().unwrap() = (width.clamp(120.0, WIDTH), height.clamp(20.0, HEIGHT));
}

/// App que estava na frente antes de a pílula pegar o teclado (para devolver depois).
static ANTERIOR: Mutex<Option<i32>> = Mutex::new(None);

/// O mouse está sobre a pílula agora (atualizado pelo `watch_cursor`).
static SOBRE_A_PILULA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// O app ativo (o editor embaixo, por exemplo) recebe os movimentos do mouse e
/// reafirma o cursor dele mesmo com o ponteiro sobre a pílula: piscava a barra
/// de texto ao sair de um botão (confirmado com log; o cursor do próprio Vigia
/// nunca mudava). Enquanto o mouse está sobre a pílula, reaplica o cursor pedido
/// a cada 4 ms, menos de um quadro mesmo a 120 Hz. Fora dela, só dorme.
#[cfg(target_os = "macos")]
fn hold_cursor(app: AppHandle) {
    use std::sync::atomic::Ordering;
    loop {
        if SOBRE_A_PILULA.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(4));
            if let Some(w) = window(&app) {
                let k = *CURSOR_PEDIDO.lock().unwrap();
                let _ = w.run_on_main_thread(move || mac::ensure_cursor(k));
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
    }
}

/// Cursor que a pílula pediu por último (veja `notch_cursor`).
static CURSOR_PEDIDO: Mutex<CursorKind> = Mutex::new(CursorKind::Seta);

/// Decide qual app guardar: o da frente, se existir e não for o próprio Vigia.
fn guardar_anterior(da_frente: Option<i32>, proprio: i32) -> Option<i32> {
    da_frente.filter(|p| *p != proprio && *p > 0)
}

/// O painel foi aberto e pegou o foco: a pílula não tem mais o que devolver.
pub fn esquecer_anterior() {
    ANTERIOR.lock().unwrap().take();
}

/// Ao largar o teclado, devolver a frente ao app anterior só se o Vigia está na
/// frente **e** a pílula ainda é a janela-chave. Se o usuário trocou de app, ou
/// se outra janela do Vigia (o painel, por "Abrir no painel") pegou o foco,
/// reativar o anterior puxaria o usuário de volta ou mandaria o painel para trás.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn devolver_foco(vigia_ativo: bool, pilula_e_chave: bool) -> bool {
    vigia_ativo && pilula_e_chave
}

/// A pílula normalmente não rouba o foco do app em uso. Para digitar na conversa
/// ela precisa virar focável por um instante; ao terminar, volta a não ser e o
/// teclado volta para o app que estava em uso.
#[tauri::command]
pub fn notch_focusable(app: AppHandle, on: bool) {
    let Some(w) = window(&app) else { return };
    let w2 = w.clone();
    let tarefa = move || {
        if on {
            #[cfg(target_os = "macos")]
            {
                // Atualiza a cada "ligar" para não ficar velho; o próprio Vigia nunca é guardado.
                let mut ant = ANTERIOR.lock().unwrap();
                if let Some(p) = guardar_anterior(mac::frontmost_pid(), std::process::id() as i32) {
                    *ant = Some(p);
                }
            }
            let _ = w2.set_focusable(true);
            let _ = w2.set_focus();
        } else {
            // Se o Vigia já não é o app da frente (o usuário trocou de app) ou o painel
            // pegou o foco, só larga o foco (veja `devolver_foco`).
            // Lido antes de `set_focusable(false)`, que tira a chave da pílula.
            #[cfg(target_os = "macos")]
            let devolver = devolver_foco(mac::vigia_ativo(), mac::e_janela_chave(&w2));
            let _ = w2.set_focusable(false);
            let anterior = ANTERIOR.lock().unwrap().take();
            #[cfg(target_os = "macos")]
            if devolver {
                match anterior {
                    Some(pid) => mac::activate_pid(pid),
                    None => mac::deactivate_self(),
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = anterior;
        }
    };
    let _ = w.run_on_main_thread(tarefa);
}

/// Cursor que a pílula pede para o elemento sob o mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorKind {
    Seta,
    Mao,
    Texto,
}

/// "pointer" vira mão, "text" vira barra de texto; o resto, seta.
pub fn cursor_kind(nome: &str) -> CursorKind {
    match nome {
        "pointer" => CursorKind::Mao,
        "text" => CursorKind::Texto,
        _ => CursorKind::Seta,
    }
}

/// A pílula não é a janela ativa (de propósito, para não roubar o foco), e o
/// WebKit só troca o cursor em janela ativa: o `cursor: pointer` do CSS nunca
/// aparecia. Ela então pede o cursor pelo nome conforme o elemento sob o mouse
/// (vigia:cursor + elementFromPoint), e o Rust o aplica com o NSCursor.
#[tauri::command]
pub fn notch_cursor(app: AppHandle, kind: String) {
    let Some(w) = window(&app) else { return };
    let k = cursor_kind(&kind);
    *CURSOR_PEDIDO.lock().unwrap() = k;
    let _ = w.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        mac::set_cursor(k);
        #[cfg(not(target_os = "macos"))]
        let _ = k;
    });
}

#[cfg(test)]
mod cursor_tests {
    use super::{cursor_kind, CursorKind};

    #[test]
    fn nomes_de_cursor() {
        assert_eq!(cursor_kind("pointer"), CursorKind::Mao);
        assert_eq!(cursor_kind("text"), CursorKind::Texto);
        assert_eq!(cursor_kind("default"), CursorKind::Seta);
        assert_eq!(cursor_kind("qualquer coisa"), CursorKind::Seta);
    }

    #[test]
    fn pilula_escondida_esquece_o_mouse() {
        use super::{esquecer_mouse, CURSOR_PEDIDO, SOBRE_A_PILULA};
        use std::sync::atomic::Ordering;
        SOBRE_A_PILULA.store(true, Ordering::Relaxed);
        *CURSOR_PEDIDO.lock().unwrap() = CursorKind::Mao;
        let mut antes = Some(true);
        esquecer_mouse(&mut antes);
        assert!(!SOBRE_A_PILULA.load(Ordering::Relaxed));
        assert_eq!(*CURSOR_PEDIDO.lock().unwrap(), CursorKind::Seta);
        // A próxima volta reenvia o ignore e o hover.
        assert_eq!(antes, None);
    }
}

#[cfg(test)]
mod foco_tests {
    use super::{devolver_foco, guardar_anterior};

    #[test]
    fn devolve_so_com_a_pilula_como_chave() {
        assert!(devolver_foco(true, true));
        // "Abrir no painel": o painel virou a janela-chave; nada de reativar o anterior.
        assert!(!devolver_foco(true, false));
        // O usuário trocou de app.
        assert!(!devolver_foco(false, false));
        assert!(!devolver_foco(false, true));
    }

    #[test]
    fn guarda_so_outro_app() {
        assert_eq!(guardar_anterior(Some(123), 999), Some(123));
        assert_eq!(guardar_anterior(Some(999), 999), None);
        assert_eq!(guardar_anterior(None, 999), None);
        assert_eq!(guardar_anterior(Some(0), 999), None);
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send, MainThreadMarker};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use tauri::WebviewWindow;

    /// Pid do app da frente (None se não der para saber). Só na thread principal.
    pub fn frontmost_pid() -> Option<i32> {
        // SAFETY: seletores padrão de NSWorkspace/NSRunningApplication; retornos
        // checados contra nil.
        unsafe {
            let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            if ws.is_null() {
                return None;
            }
            let app: *mut AnyObject = msg_send![ws, frontmostApplication];
            if app.is_null() {
                return None;
            }
            let pid: i32 = msg_send![app, processIdentifier];
            Some(pid)
        }
    }

    /// O cursor compartilhado do NSCursor para cada tipo. Só na thread principal.
    fn cursor_de(k: super::CursorKind) -> *mut AnyObject {
        // SAFETY: os três seletores de classe de NSCursor existem e devolvem
        // cursores compartilhados (sempre o mesmo objeto).
        unsafe {
            match k {
                super::CursorKind::Mao => msg_send![class!(NSCursor), pointingHandCursor],
                super::CursorKind::Texto => msg_send![class!(NSCursor), IBeamCursor],
                super::CursorKind::Seta => msg_send![class!(NSCursor), arrowCursor],
            }
        }
    }

    /// Troca o cursor do sistema (mão, barra de texto ou seta). Só na thread principal.
    pub fn set_cursor(k: super::CursorKind) {
        let c = cursor_de(k);
        if !c.is_null() {
            // SAFETY: `c` é um NSCursor válido (não nulo).
            unsafe {
                let _: () = msg_send![c, set];
            }
        }
    }

    /// Reaplica o cursor pedido. Os movimentos do mouse vão para o app ativo
    /// (o editor embaixo, por exemplo), que reafirma o cursor dele mesmo com o
    /// ponteiro sobre a pílula; o cursor do Vigia não muda, então não adianta
    /// comparar: reaplica sempre (é barato). Só na thread principal.
    pub fn ensure_cursor(k: super::CursorKind) {
        set_cursor(k);
    }

    /// O Vigia é o app ativo agora (NSApplication.isActive). Só na thread principal.
    pub fn vigia_ativo() -> bool {
        // SAFETY: NSApplication.sharedApplication existe com o app rodando.
        unsafe {
            let ns: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            if ns.is_null() {
                return false;
            }
            msg_send![ns, isActive]
        }
    }

    /// A janela é a janela-chave (recebe o teclado) agora. Só na thread principal.
    pub fn e_janela_chave(w: &WebviewWindow) -> bool {
        let Ok(ptr) = w.ns_window() else { return false };
        let ns = ptr as *mut AnyObject;
        if ns.is_null() {
            return false;
        }
        // SAFETY: `ns` é o NSWindow vivo desta janela; `isKeyWindow` é público e
        // devolve BOOL. Chamado dentro de `run_on_main_thread`.
        unsafe { msg_send![ns, isKeyWindow] }
    }

    /// Traz de volta o app com esse pid (opções 0 = padrão).
    pub fn activate_pid(pid: i32) {
        // SAFETY: runningApplicationWithProcessIdentifier: devolve nil se o app saiu.
        unsafe {
            let app: *mut AnyObject =
                msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
            if app.is_null() {
                deactivate_self();
                return;
            }
            let _: bool = msg_send![app, activateWithOptions: 0usize];
        }
    }

    /// Sem app guardado: só larga a frente, para o macOS escolher o próximo.
    pub fn deactivate_self() {
        // SAFETY: NSApplication.sharedApplication existe com o app rodando; -deactivate é público.
        unsafe {
            let ns: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            if !ns.is_null() {
                let _: () = msg_send![ns, deactivate];
            }
        }
    }

    /// Põe a janela acima da barra de menus e em todas as mesas (Spaces),
    /// inclusive por cima de apps em tela cheia.
    pub fn float_above_menu_bar(w: &WebviewWindow) {
        let Ok(ptr) = w.ns_window() else { return };
        let ns = ptr as *mut AnyObject;
        if ns.is_null() {
            return;
        }
        // NSMainMenuWindowLevel = 24, NSStatusWindowLevel = 25.
        const LEVEL: isize = 26;
        // canJoinAllSpaces | stationary | ignoresCycle | fullScreenAuxiliary
        const BEHAVIOR: usize = (1 << 0) | (1 << 4) | (1 << 6) | (1 << 8);
        // SAFETY: `ns` é o NSWindow vivo desta janela; os seletores existem em NSWindow
        // e recebem NSInteger/NSUInteger. Chamado na thread principal (setup do Tauri).
        unsafe {
            let _: () = msg_send![ns, setLevel: LEVEL];
            let _: () = msg_send![ns, setCollectionBehavior: BEHAVIOR];
        }
    }

    /// Tamanho e posição numa chamada só. Com `set_size` + `set_position` a
    /// janela passava um quadro no lugar errado (o Cocoa mantém o canto de
    /// baixo fixo ao mudar a altura), e a pílula piscava ao fechar.
    /// Devolve `false` se não deu para usar o NSWindow; aí o chamador faz do jeito portátil.
    pub fn set_frame_top_center(w: &WebviewWindow, width: f64, height: f64) -> bool {
        if MainThreadMarker::new().is_none() {
            return false;
        }
        let Ok(ptr) = w.ns_window() else { return false };
        let ns = ptr as *mut AnyObject;
        if ns.is_null() {
            return false;
        }
        // SAFETY: thread principal (checado acima); `ns` é o NSWindow vivo desta
        // janela; `+[NSScreen screens]`, `firstObject`, `frame` e
        // `setFrame:display:` existem com essas assinaturas.
        unsafe {
            // A primeira tela da lista é a que tem a barra de menus.
            let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
            if screens.is_null() {
                return false;
            }
            let screen: *mut AnyObject = msg_send![screens, firstObject];
            if screen.is_null() {
                return false;
            }
            let sf: NSRect = msg_send![screen, frame];
            // Coordenadas do Cocoa: origem embaixo à esquerda.
            let x = (sf.origin.x + (sf.size.width - width) / 2.0).round();
            let y = sf.origin.y + sf.size.height - height;
            let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
            let _: () = msg_send![ns, setFrame: frame, display: true];
        }
        true
    }
}
