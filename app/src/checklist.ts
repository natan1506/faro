// Regras da tela Início: quais passos da configuração inicial estão feitos e o que o card
// de destaque diz. Sem DOM e sem importar o api.ts (que puxa o Tauri), para testar com node.
import type { Alert, AppConfig, Settings, Severity, Status } from "./api.ts";
import { CELULAR } from "./recursos.ts";

export type StepId = "daemon" | "fda" | "terceiros" | "celular" | "normal" | "cofre";
export interface Step {
  id: StepId;
  title: string;
  done: boolean;
  /** Texto do botão do passo pendente. */
  action: string;
}

/** `celular`: inclui o passo do celular (padrão: o recurso CELULAR de recursos.ts). */
export function steps(i: { connected: boolean; status: Status | null; settings: Settings | null; config: AppConfig; celular?: boolean }): Step[] {
  const c = i.connected;
  const comCelular = i.celular ?? CELULAR;
  const todos: Step[] = [
    { id: "daemon", title: "Instalar o Faro", done: c, action: "Como instalar" },
    { id: "fda", title: "Dar Acesso Total ao Disco", done: c && i.status?.sensor.state === "running", action: "Abrir Ajustes do macOS" },
    { id: "terceiros", title: "Marcar a pasta de repositórios de terceiros", done: c && (i.settings?.third_party_dirs.length ?? 0) > 0, action: "Escolher pasta" },
    { id: "celular", title: "Receber alertas no celular", done: c && i.settings?.ntfy_enabled === true, action: "Configurar" },
    { id: "normal", title: "Revisar o que é normal no seu dia", done: i.config.checklist.reviewed_normal, action: "Começar" },
    { id: "cofre", title: "Tirar os .env dos projetos do disco", done: i.config.checklist.cofre_done, action: "Ver como" },
  ];
  return comCelular ? todos : todos.filter((s) => s.id !== "celular");
}

const ordem: Record<Severity, number> = { critical: 0, high: 1, medium: 2, low: 3 };

/** "5 min", "1 h", "2 dias": há quanto tempo, em português curto. */
export function haQuanto(iso: string, now = Date.now()): string {
  const min = Math.max(0, Math.round((now - Date.parse(iso)) / 60_000));
  if (min < 1) return "menos de 1 min";
  if (min < 60) return `${min} min`;
  const horas = Math.floor(min / 60);
  if (horas < 24) return `${horas} h`;
  const dias = Math.floor(horas / 24);
  return dias === 1 ? "1 dia" : `${dias} dias`;
}

export function headline(
  i: { connected: boolean; status: Status | null; alerts: Alert[] },
  now = Date.now(),
): { title: string; line: string; button: string | null } {
  if (!i.connected) {
    return {
      title: "O Faro parou de vigiar",
      line: "Nenhum evento está sendo analisado. Verifique com: sudo launchctl print system/dev.natan.vigia",
      button: null,
    };
  }
  if (i.status?.sensor.state === "replay") {
    return { title: "Simulação", line: "Os alertas abaixo são de uma gravação de teste; nada disso aconteceu no seu Mac.", button: "Ver alertas" };
  }
  const abertos = i.alerts.filter((a) => a.status === "open");
  if (abertos.length > 0) {
    const pior = [...abertos].sort((a, b) => ordem[a.severity] - ordem[b.severity] || Date.parse(b.ts) - Date.parse(a.ts))[0];
    return {
      title: abertos.length === 1 ? "1 alerta pede sua decisão" : `${abertos.length} alertas pedem sua decisão`,
      line: `O mais grave: ${pior.title}, há ${haQuanto(pior.ts, now)}.`,
      button: "Ver alertas",
    };
  }
  const v = i.status?.events_per_sec ?? 0;
  const eps = v.toLocaleString("pt-BR", { maximumFractionDigits: v < 10 ? 1 : 0 });
  return { title: "Tudo calmo por aqui", line: `${eps} eventos por segundo analisados agora.`, button: null };
}
