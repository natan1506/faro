// Regras do seletor de IA e modelo (badge da conversa). Sem DOM e sem importar o
// api.ts em tempo de execução, para testar com node.
import type { AppConfig } from "./api.ts";

/** Valor da opção "Outro…": o nome do modelo vem de um campo de texto. */
export const OUTRO = "__outro__";

/** Valor de uma opção do <select>: "ia|modelo". */
export const valorOpcao = (provider: string, model: string) => `${provider}|${model}`;

/** Lê o valor de uma opção. Divide só no primeiro "|": o nome do modelo pode ter "|". */
export function lerOpcao(v: string): { provider: string; model: string } {
  const i = v.indexOf("|");
  if (i < 0) return { provider: v, model: "" };
  return { provider: v.slice(0, i), model: v.slice(i + 1) };
}

/** O modelo atual não está na lista da IA (foi digitado em "Outro…"). */
export function modeloEmOutro(cfg: AppConfig): boolean {
  const ai = cfg.ai;
  const lista = ai.provider === "codex" ? ai.codex_models : ai.claude_models;
  return ai.model !== "" && !lista.includes(ai.model);
}

/** Valor do <select> que corresponde ao config atual. */
export function opcaoAtual(cfg: AppConfig): string {
  return valorOpcao(cfg.ai.provider, modeloEmOutro(cfg) ? OUTRO : cfg.ai.model);
}

/**
 * Config novo com a IA e o modelo escolhidos, ou null se nada muda.
 * Trocar para o Codex vindo de outra IA zera o aceite do aviso: ele aparece de novo.
 */
export function aplicarEscolha(cfg: AppConfig, provider: string, model: string): AppConfig | null {
  const ai = cfg.ai;
  if (provider === ai.provider && model === ai.model) return null;
  const novo: AppConfig = JSON.parse(JSON.stringify(cfg));
  if (provider !== ai.provider) {
    novo.ai.provider = provider as AppConfig["ai"]["provider"];
    if (provider === "codex") novo.ai.codex_ack_at = null;
  }
  novo.ai.model = model;
  return novo;
}
