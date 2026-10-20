//! Sugestões de ação na resposta da IA. A IA nunca age: quando recomenda uma
//! ação sobre um alerta, termina a resposta com linhas `vigia-acao: {...}`. Aqui
//! essas linhas saem do texto e as válidas viram botões (que chamam o `act`).

use serde::{Deserialize, Serialize};
use vigia_proto::{ActionKind, Alert, AlertStatus};

const PREFIXO: &str = "vigia-acao:";

/// Vai no prompt de sistema.
pub const ACTION_INSTRUCTIONS: &str = "\
Se você recomendar uma ação sobre um alerta, termine a resposta com uma linha \
por ação neste formato exato:
vigia-acao: {\"alert_id\": 7, \"action\": \"kill_tree\"}
Isso é opcional: use só quando a ação realmente fizer sentido. Só vale para \
ações listadas no próprio alerta (campo de ações) e para alertas abertos. \
Nunca sugira \"allow\" (permitir sempre): criar exceção é decisão só do usuário. \
Escreva uma ação por linha, sem nenhum outro texto na mesma linha. O usuário \
verá um botão e decidirá se executa; você nunca executa nada.";

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SuggestedAction {
    pub alert_id: i64,
    pub action: ActionKind,
    pub label: String,
}

#[derive(Deserialize)]
struct Bruta {
    alert_id: i64,
    action: ActionKind,
}

/// Espelha `actionLabel` de `app/src/api.ts`: manter os dois em sincronia.
pub fn action_label(a: ActionKind) -> &'static str {
    match a {
        ActionKind::KillTree => "Encerrar processo",
        ActionKind::Kill => "Encerrar só este",
        ActionKind::Quarantine => "Quarentena",
        ActionKind::Allow => "Permitir sempre",
        ActionKind::Dismiss => "Ignorar",
        ActionKind::Reveal => "Mostrar no Finder",
        ActionKind::OpenSettings => "Abrir Ajustes",
    }
}

/// A linha, ignorando espaços e crases em volta, é uma linha de ação?
fn corpo_da_acao(linha: &str) -> Option<&str> {
    linha
        .trim()
        .trim_matches('`')
        .trim()
        .strip_prefix(PREFIXO)
        .map(str::trim)
}

fn e_cerca(linha: &str) -> bool {
    linha.trim().starts_with("```")
}

/// Tira as linhas de ação do texto e devolve as sugestões válidas: alerta
/// existente, aberto, com a ação entre as oferecidas. Sem repetidas.
///
/// `allow` nunca vale: o texto do alerta é escrito pelo processo suspeito e
/// poderia convencer a IA a sugerir uma exceção permanente para ele.
pub fn extract_actions(text: &str, alerts: &[Alert]) -> (String, Vec<SuggestedAction>) {
    let linhas: Vec<&str> = text.lines().collect();
    let mut remover = vec![false; linhas.len()];

    // Bloco de código que só tinha linhas de ação: a cerca vai junto.
    let mut i = 0;
    while i < linhas.len() {
        if e_cerca(linhas[i]) {
            if let Some(fim) = (i + 1..linhas.len()).find(|&j| e_cerca(linhas[j])) {
                let miolo = &linhas[i + 1..fim];
                let so_acoes = miolo.iter().any(|l| corpo_da_acao(l).is_some())
                    && miolo
                        .iter()
                        .all(|l| l.trim().is_empty() || corpo_da_acao(l).is_some());
                if so_acoes {
                    remover[i] = true;
                    remover[fim] = true;
                }
                i = fim;
            }
        }
        i += 1;
    }

    let mut texto: Vec<&str> = Vec::new();
    let mut sugestoes: Vec<SuggestedAction> = Vec::new();
    for (i, linha) in linhas.iter().enumerate() {
        if remover[i] {
            continue;
        }
        let Some(corpo) = corpo_da_acao(linha) else {
            texto.push(linha);
            continue;
        };
        let Ok(b) = serde_json::from_str::<Bruta>(corpo) else {
            continue;
        };
        let valida = b.action != ActionKind::Allow
            && alerts.iter().any(|a| {
                a.id == b.alert_id && a.status == AlertStatus::Open && a.actions.contains(&b.action)
            });
        let repetida = sugestoes
            .iter()
            .any(|s| s.alert_id == b.alert_id && s.action == b.action);
        if valida && !repetida {
            sugestoes.push(SuggestedAction {
                alert_id: b.alert_id,
                action: b.action,
                label: action_label(b.action).to_string(),
            });
        }
    }
    (texto.join("\n").trim_end().to_string(), sugestoes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vigia_proto::{ProcInfo, Severity};

    fn alert(id: i64, status: AlertStatus, actions: Vec<ActionKind>) -> Alert {
        Alert {
            id,
            ts: "2026-10-19T11:00:00Z".into(),
            first_ts: "2026-10-19T11:00:00Z".into(),
            rule: "secret_read".into(),
            severity: Severity::High,
            title: "t".into(),
            detail: "d".into(),
            target: None,
            process: ProcInfo {
                pid: 10,
                path: "/usr/bin/node".into(),
                ..Default::default()
            },
            chain: vec![],
            actions,
            count: 1,
            status,
            status_message: None,
            simulated: false,
        }
    }

    fn abertos() -> Vec<Alert> {
        vec![alert(
            7,
            AlertStatus::Open,
            vec![ActionKind::KillTree, ActionKind::Dismiss],
        )]
    }

    const LINHA: &str = r#"vigia-acao: {"alert_id": 7, "action": "kill_tree"}"#;

    #[test]
    fn extracts_and_strips() {
        let (texto, s) = extract_actions(&format!("Encerre o processo.\n{LINHA}"), &abertos());
        assert_eq!(texto, "Encerre o processo.");
        assert_eq!(
            s,
            vec![SuggestedAction {
                alert_id: 7,
                action: ActionKind::KillTree,
                label: "Encerrar processo".into()
            }]
        );
    }

    #[test]
    fn ignores_unknown_alert() {
        let l = r#"vigia-acao: {"alert_id": 999, "action": "kill_tree"}"#;
        let (texto, s) = extract_actions(&format!("oi\n{l}"), &abertos());
        assert!(s.is_empty());
        assert_eq!(texto, "oi");
    }

    #[test]
    fn ignores_action_not_offered() {
        let a = vec![alert(7, AlertStatus::Open, vec![ActionKind::Dismiss])];
        let l = r#"vigia-acao: {"alert_id": 7, "action": "quarantine"}"#;
        assert!(extract_actions(l, &a).1.is_empty());
    }

    #[test]
    fn never_suggests_allow() {
        let a = vec![alert(
            7,
            AlertStatus::Open,
            vec![ActionKind::Allow, ActionKind::Dismiss],
        )];
        let l = r#"vigia-acao: {"alert_id": 7, "action": "allow"}"#;
        let (texto, s) = extract_actions(&format!("Pode permitir.\n{l}"), &a);
        assert!(s.is_empty(), "{s:?}");
        // A linha sai do texto do mesmo jeito.
        assert_eq!(texto, "Pode permitir.");
        assert!(ACTION_INSTRUCTIONS.contains("Nunca sugira"));
    }

    #[test]
    fn ignores_closed_alert() {
        let a = vec![alert(7, AlertStatus::Killed, vec![ActionKind::KillTree])];
        assert!(extract_actions(LINHA, &a).1.is_empty());
    }

    #[test]
    fn dedups() {
        let (_, s) = extract_actions(&format!("{LINHA}\ntexto\n{LINHA}"), &abertos());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn bad_json_is_removed_and_ignored() {
        let (texto, s) = extract_actions("antes\nvigia-acao: {nao e json}\ndepois", &abertos());
        assert_eq!(texto, "antes\ndepois");
        assert!(s.is_empty());
    }

    #[test]
    fn tolerates_spaces_and_inline_backticks() {
        let (texto, s) = extract_actions(&format!("oi\n   `{LINHA}`  "), &abertos());
        assert_eq!(texto, "oi");
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn strips_fence_around_only_actions() {
        let t = format!("Recomendo:\n```\n{LINHA}\n```");
        let (texto, s) = extract_actions(&t, &abertos());
        assert_eq!(texto, "Recomendo:");
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn keeps_fence_with_other_content() {
        let t = format!("```sh\nls\n{LINHA}\n```");
        let (texto, s) = extract_actions(&t, &abertos());
        assert_eq!(texto, "```sh\nls\n```");
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn labels_match_api_ts() {
        assert_eq!(action_label(ActionKind::Allow), "Permitir sempre");
        assert_eq!(action_label(ActionKind::Reveal), "Mostrar no Finder");
    }
}
