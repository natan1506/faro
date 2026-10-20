//! Detecção de keylogger: lista os "event taps" de teclado ativos.
//!
//! Um event tap é como um app escuta as teclas de todo o sistema. Só dá para
//! listá-los de dentro da sessão do usuário, por isso esta checagem fica no
//! app e não no daemon (que roda como root, fora da sessão). Quando aparece um
//! programa novo escutando o teclado, o app reporta ao daemon, que grava o
//! alerta e o devolve pela inscrição, como qualquer outro.
//!
//! Os taps que já existem quando o app abre (Raycast, BetterTouchTool...)
//! viram referência e não geram alerta.

use std::collections::HashSet;
use std::time::Duration;

use tauri::AppHandle;
use vigia_proto::{
    ActionKind, Alert, AlertStatus, ClientMsg, ProcInfo, Report, ServerMsg, Severity,
};

use crate::client;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGEventTapInformation {
    event_tap_id: u32,
    tap_point: u32,
    options: u32,
    events_of_interest: u64,
    tapping_process: i32,
    process_being_tapped: i32,
    enabled: bool,
    min_usec_latency: f32,
    avg_usec_latency: f32,
    max_usec_latency: f32,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetEventTapList(max: u32, list: *mut CGEventTapInformation, count: *mut u32) -> i32;
}

// kCGEventKeyDown = 10, kCGEventKeyUp = 11, kCGEventFlagsChanged = 12
const KEYBOARD_MASK: u64 = (1 << 10) | (1 << 11) | (1 << 12);

pub struct Tap {
    pub pid: i32,
    pub exe: String,
    pub listen_only: bool,
}

fn exe_of(pid: i32) -> String {
    let mut buf = vec![0u8; 4096];
    // SAFETY: buffer com o tamanho informado; proc_pidpath escreve no máximo isso.
    let n =
        unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32) };
    if n <= 0 {
        return format!("pid {pid}");
    }
    String::from_utf8_lossy(&buf[..n as usize]).into_owned()
}

pub fn keyboard_taps() -> Vec<Tap> {
    let mut count: u32 = 0;
    // SAFETY: com max = 0 e lista nula, a função só preenche `count`.
    if unsafe { CGGetEventTapList(0, std::ptr::null_mut(), &mut count) } != 0 || count == 0 {
        return Vec::new();
    }
    let mut list = vec![CGEventTapInformation::default(); count as usize];
    let mut got: u32 = 0;
    // SAFETY: a lista tem `count` posições.
    if unsafe { CGGetEventTapList(count, list.as_mut_ptr(), &mut got) } != 0 {
        return Vec::new();
    }
    let me = std::process::id() as i32;
    list.truncate(got as usize);
    list.into_iter()
        .filter(|t| {
            t.enabled && t.events_of_interest & KEYBOARD_MASK != 0 && t.tapping_process != me
        })
        .map(|t| Tap {
            pid: t.tapping_process,
            exe: exe_of(t.tapping_process),
            listen_only: t.options == 1,
        })
        .collect()
}

pub async fn watch(app: AppHandle) {
    let mut known: HashSet<String> = keyboard_taps().into_iter().map(|t| t.exe).collect();
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        for t in keyboard_taps() {
            if !known.insert(t.exe.clone()) {
                continue;
            }
            let name = t.exe.rsplit('/').next().unwrap_or(&t.exe).to_string();
            let how = if t.listen_only {
                "só escutando"
            } else {
                "podendo alterar as teclas"
            };
            let report = Report {
                rule: "keyboard_tap".into(),
                severity: Severity::Critical,
                title: format!("{name} está lendo o teclado"),
                detail: format!(
                    "{} (pid {}) começou a capturar as teclas de todo o sistema, {how}. É assim que um keylogger funciona. Se você não instalou agora um app que faz isso, encerre o processo.",
                    t.exe, t.pid
                ),
                target: Some("teclado".into()),
                process: ProcInfo { pid: t.pid, path: t.exe.clone(), ..Default::default() },
            };
            let sent = client::request(
                &ClientMsg::Report {
                    report: report.clone(),
                },
                |m| match m {
                    ServerMsg::Reported { ok, .. } => Some(ok),
                    _ => None,
                },
            )
            .await;
            // Sem daemon, o app mostra o alerta mesmo assim (sem gravar e sem botões de ação).
            if sent.is_err() {
                let now = chrono_now();
                crate::on_alert(
                    &app,
                    Alert {
                        id: 0,
                        ts: now.clone(),
                        first_ts: now,
                        rule: report.rule,
                        severity: report.severity,
                        title: report.title,
                        detail: report.detail,
                        target: report.target,
                        process: report.process,
                        chain: vec![],
                        actions: vec![ActionKind::Dismiss],
                        count: 1,
                        status: AlertStatus::Open,
                        status_message: None,
                        simulated: false,
                    },
                );
            }
        }
    }
}

fn chrono_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
