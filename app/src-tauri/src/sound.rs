//! Som de alerta: toca um arquivo de áudio quando chega um alerta novo.
//! Uma rajada de alertas toca uma vez só (`MIN_GAP`).

use std::time::{Duration, Instant};

use vigia_proto::{Alert, AlertStatus, Severity};

/// Intervalo mínimo entre dois sons.
const MIN_GAP: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundKind {
    High,
    Medium,
}

#[derive(Default)]
pub struct SoundGate {
    last: Option<Instant>,
}

impl SoundGate {
    /// Decide se este alerta toca som. Ao devolver `Some`, registra o instante.
    pub fn should_play(
        &mut self,
        now: Instant,
        is_new: bool,
        alert: &Alert,
        enabled: bool,
    ) -> Option<SoundKind> {
        if !enabled || !is_new || alert.status != AlertStatus::Open || alert.severity < Severity::Medium
        {
            return None;
        }
        if self.last.is_some_and(|l| now.duration_since(l) < MIN_GAP) {
            return None;
        }
        self.last = Some(now);
        Some(if alert.severity >= Severity::High {
            SoundKind::High
        } else {
            SoundKind::Medium
        })
    }
}

/// Toca o arquivo sem esperar terminar. Só no macOS (usa o `afplay`).
#[cfg(target_os = "macos")]
pub fn play(path: &str, volume: f32) {
    tocar("/usr/bin/afplay", path, volume);
}

#[cfg(not(target_os = "macos"))]
pub fn play(_path: &str, _volume: f32) {}

/// Roda o tocador e recolhe o processo numa thread quando ele termina (sem
/// deixar zumbi). Devolve a thread para os testes esperarem.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn tocar(bin: &str, path: &str, volume: f32) -> Option<std::thread::JoinHandle<()>> {
    match std::process::Command::new(bin)
        .args(["-v", &format!("{volume:.2}"), path])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut filho) => Some(std::thread::spawn(move || {
            let _ = filho.wait();
        })),
        Err(e) => {
            eprintln!("aviso: não consegui tocar o som {path} com o {bin}: {e}");
            None
        }
    }
}

/// Sons que vêm com o macOS, em ordem, com o caminho completo.
pub fn system_sounds() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/System/Library/Sounds") else {
        return Vec::new();
    };
    let mut v: Vec<String> = dir
        .filter_map(|e| e.ok())
        .map(|e| e.path().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use vigia_proto::ProcInfo;

    fn alert(severity: Severity, status: AlertStatus) -> Alert {
        Alert {
            id: 1,
            ts: String::new(),
            first_ts: String::new(),
            rule: "secret_read".into(),
            severity,
            title: String::new(),
            detail: String::new(),
            target: None,
            process: ProcInfo::default(),
            chain: vec![],
            actions: vec![],
            count: 1,
            status,
            status_message: None,
            simulated: false,
        }
    }

    #[test]
    fn plays_for_new_open_critical() {
        let mut g = SoundGate::default();
        let a = alert(Severity::Critical, AlertStatus::Open);
        assert_eq!(g.should_play(Instant::now(), true, &a, true), Some(SoundKind::High));
    }

    #[test]
    fn plays_medium_sound_for_medium() {
        let mut g = SoundGate::default();
        let a = alert(Severity::Medium, AlertStatus::Open);
        assert_eq!(g.should_play(Instant::now(), true, &a, true), Some(SoundKind::Medium));
        let mut g = SoundGate::default();
        let a = alert(Severity::High, AlertStatus::Open);
        assert_eq!(g.should_play(Instant::now(), true, &a, true), Some(SoundKind::High));
    }

    #[test]
    fn ignores_low() {
        let mut g = SoundGate::default();
        let a = alert(Severity::Low, AlertStatus::Open);
        assert_eq!(g.should_play(Instant::now(), true, &a, true), None);
    }

    #[test]
    fn ignores_repeat_and_closed() {
        let mut g = SoundGate::default();
        let a = alert(Severity::Critical, AlertStatus::Open);
        assert_eq!(g.should_play(Instant::now(), false, &a, true), None);
        let k = alert(Severity::Critical, AlertStatus::Killed);
        assert_eq!(g.should_play(Instant::now(), true, &k, true), None);
    }

    #[test]
    fn burst_plays_once() {
        let mut g = SoundGate::default();
        let a = alert(Severity::Critical, AlertStatus::Open);
        let t = Instant::now();
        let n = (0..8)
            .filter_map(|i| g.should_play(t + Duration::from_millis(50 * i), true, &a, true))
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn plays_again_after_ten_seconds() {
        let a = alert(Severity::Critical, AlertStatus::Open);
        let t = Instant::now();
        let mut g = SoundGate::default();
        assert!(g.should_play(t, true, &a, true).is_some());
        assert!(g.should_play(t + Duration::from_secs(10), true, &a, true).is_some());
        let mut g = SoundGate::default();
        assert!(g.should_play(t, true, &a, true).is_some());
        assert!(g.should_play(t + Duration::from_millis(9900), true, &a, true).is_none());
    }

    #[test]
    fn disabled_never_plays() {
        let mut g = SoundGate::default();
        let a = alert(Severity::Critical, AlertStatus::Open);
        assert_eq!(g.should_play(Instant::now(), true, &a, false), None);
    }

    #[test]
    fn simulated_alerts_play() {
        let mut g = SoundGate::default();
        let mut a = alert(Severity::Critical, AlertStatus::Open);
        a.simulated = true;
        assert_eq!(g.should_play(Instant::now(), true, &a, true), Some(SoundKind::High));
    }

    #[test]
    fn player_is_reaped() {
        // `/usr/bin/true` aceita e ignora os argumentos: termina na hora.
        let t = tocar("/usr/bin/true", "/x.aiff", 0.5).expect("devia rodar");
        t.join().unwrap();
        // Tocador que não existe: só avisa no log.
        assert!(tocar("/nao/existe/afplay", "/x.aiff", 0.5).is_none());
    }
}
