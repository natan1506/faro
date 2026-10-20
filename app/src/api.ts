// Tipos espelhados de crates/proto/src/lib.rs e chamadas ao backend do app.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Severity = "low" | "medium" | "high" | "critical";
export type AlertStatus = "open" | "killed" | "quarantined" | "allowed" | "dismissed" | "failed";
export type ActionKind = "kill" | "kill_tree" | "quarantine" | "allow" | "dismiss" | "reveal" | "open_settings";

export interface ProcInfo {
  pid: number;
  ppid: number;
  path: string;
  args?: string[];
  cwd?: string;
  signing_id?: string;
  team_id?: string;
  signed?: boolean;
}

export interface Alert {
  id: number;
  ts: string;
  first_ts: string;
  rule: string;
  severity: Severity;
  title: string;
  detail: string;
  target?: string;
  process: ProcInfo;
  chain?: ProcInfo[];
  actions: ActionKind[];
  count: number;
  status: AlertStatus;
  status_message?: string;
  /** Veio do replay (gravação de teste): os botões não executam nada no Mac. */
  simulated?: boolean;
}

export interface ExecEntry {
  ts: string;
  pid: number;
  ppid: number;
  path: string;
  args: string[];
  cwd?: string;
  apple: boolean;
}

export type SensorState =
  | { state: "starting" }
  | { state: "running" }
  | { state: "replay" }
  | { state: "restarting"; reason: string }
  | { state: "down"; reason: string };

/** Exceção em vigor no daemon (só leitura). */
export interface AllowView {
  rule: string;
  signing_id: string | null;
  team_id: string | null;
  path_prefix: string | null;
  target: string | null;
  comment: string | null;
}

export interface Settings {
  third_party_dirs: string[];
  /** Vazio = o Mac inteiro. */
  watch_dirs: string[];
  rules: Record<string, boolean>;
  ntfy_enabled: boolean;
  ntfy_topic: string;
}

export interface Status {
  version: string;
  started_at: string;
  sensor: SensorState;
  events_total: number;
  events_dropped: number;
  events_per_sec: number;
  alerts_open: number;
  learning_until?: string;
}

export interface AppConfig {
  sound: { enabled: boolean; volume: number; high: string; medium: string };
  /** Segundos até a pílula recolher sozinha: 10, 15 ou 30. */
  auto_close_secs: number;
  ai: {
    provider: "claude" | "codex";
    /** Vazio = o modelo padrão do CLI. */
    model: string;
    codex_ack_at: string | null;
    claude_models: string[];
    codex_models: string[];
  };
  checklist: { reviewed_normal: boolean; cofre_done: boolean };
  cofre_url: string;
}

/** Sugestão de ação que a IA deixou na resposta; vira botão (quem executa é você). */
export interface SuggestedAction {
  alert_id: number;
  action: ActionKind;
  label: string;
}

export interface ChatMessage {
  role: "voce" | "ia" | "erro";
  text: string;
  provider: "claude" | "codex" | null;
  model: string | null;
  actions: SuggestedAction[];
}

export const api = {
  history: (limit = 300) => invoke<Alert[]>("history", { limit }),
  timeline: (limit = 300) => invoke<ExecEntry[]>("timeline", { limit }),
  status: () => invoke<Status>("status"),
  /** Executa a ação. Ações locais (Finder, Ajustes) o backend faz sem falar com o daemon. */
  act: (alert: Alert, action: ActionKind) =>
    invoke<string>("act", { alertId: alert.id, action, target: alert.target ?? null, rule: alert.rule }),
  markSeen: (ids: number[]) => invoke<void>("mark_seen", { ids }),
  connected: () => invoke<boolean>("daemon_connected"),
  openPainel: (alertId: number | null) => invoke<void>("open_painel", { alertId }),
  /** Abre o painel na aba Ajustes. */
  openSettings: () => invoke<void>("open_settings"),
  /** Abre o painel na seção Conversar. */
  openConversa: () => invoke<void>("open_conversa"),
  /** Abre um link https:// ou dos Ajustes do macOS (x-apple.systempreferences:). */
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  allowlist: () => invoke<AllowView[]>("allowlist"),
  getSettings: () => invoke<Settings>("get_settings"),
  /** Devolve o que ficou valendo; rejeita com a mensagem do daemon se algo for recusado. */
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  getAppConfig: () => invoke<AppConfig>("get_app_config"),
  /** Devolve o que ficou valendo; rejeita com a mensagem de validação. */
  saveAppConfig: (config: AppConfig) => invoke<AppConfig>("save_app_config", { config }),
  /** Sons que vêm com o macOS (caminhos completos). */
  listSystemSounds: () => invoke<string[]>("list_system_sounds"),
  /** Toca o som uma vez para ouvir; rejeita se o arquivo não for áudio válido. */
  previewSound: (path: string, volume: number) => invoke<void>("preview_sound", { path, volume }),
  /** Seletor de arquivo de áudio; `null` se cancelar. */
  pickSoundFile: () => invoke<string | null>("pick_sound_file"),
  /** Seletor de pasta do macOS; `null` se cancelar. Vem com `~` no lugar da home. */
  pickFolder: () => invoke<string | null>("pick_folder"),
  /** Altura da barra de menus (pontos) e se a tela tem notch físico. */
  notchMetrics: () => invoke<{ menubar: number; has_notch: boolean }>("notch_metrics"),
  /** Deixa a pílula receber o teclado (para digitar na conversa) ou volta a ignorá-lo. */
  notchFocusable: (on: boolean) => invoke<void>("notch_focusable", { on }),
  /** Cursor do sistema sobre a pílula (o WebKit não troca em janela que não é a ativa). */
  notchCursor: (kind: "pointer" | "text" | "default") => invoke<void>("notch_cursor", { kind }),
  /** Área que a pílula ocupa (pontos). A janela é fixa; fora dessa área os cliques atravessam. */
  notchHitArea: (width: number, height: number) => invoke<void>("notch_hit_area", { width, height }),
  /** CLIs de IA instalados ("claude", "codex"). O app guarda a procura; `refresh` procura de novo no shell de login. */
  aiProviders: (refresh = false) => invoke<("claude" | "codex")[]>("ai_providers", { refresh }),
  /** A conversa até agora (só em memória, some quando o app fecha). */
  aiConversation: () => invoke<ChatMessage[]>("ai_conversation"),
  /** Manda a pergunta; volta logo. A resposta chega por `events.onAiChunk` e `events.onAiDone`. */
  aiAsk: (question: string, focusAlert: number | null) => invoke<void>("ai_ask", { question, focusAlert }),
  /** Para a resposta em andamento (vira uma mensagem "Resposta interrompida."). */
  aiStop: () => invoke<void>("ai_stop"),
  /** Apaga a conversa; as duas janelas recebem `events.onAiCleared`. */
  aiClear: () => invoke<void>("ai_clear"),
  /** Grava o aceite do aviso do Codex; devolve as preferências atualizadas. */
  aiAcceptCodex: () => invoke<AppConfig>("ai_accept_codex"),
};

export const events = {
  onAlert: (fn: (a: Alert, isNew: boolean) => void): Promise<UnlistenFn> =>
    listen<{ alert: Alert; is_new: boolean }>("vigia:alert", (e) => fn(e.payload.alert, e.payload.is_new)),
  onDaemon: (fn: (connected: boolean) => void): Promise<UnlistenFn> =>
    listen<boolean>("vigia:daemon", (e) => fn(e.payload)),
  onStatus: (fn: (s: Status) => void): Promise<UnlistenFn> =>
    listen<Status>("vigia:status", (e) => fn(e.payload)),
  onFocusAlert: (fn: (id: number) => void): Promise<UnlistenFn> =>
    listen<number>("vigia:focus-alert", (e) => fn(e.payload)),
  onAppConfig: (fn: (c: AppConfig) => void): Promise<UnlistenFn> =>
    listen<AppConfig>("vigia:app-config", (e) => fn(e.payload)),
  /** Pedaço novo da resposta da IA. */
  onAiChunk: (fn: (text: string) => void): Promise<UnlistenFn> =>
    listen<{ text: string }>("vigia:ai-chunk", (e) => fn(e.payload.text)),
  /** Resposta terminada (ou erro/interrupção): a mensagem final, já na conversa. */
  onAiDone: (fn: (m: ChatMessage) => void): Promise<UnlistenFn> =>
    listen<ChatMessage>("vigia:ai-done", (e) => fn(e.payload)),
  /** A conversa foi apagada (e a resposta em andamento, se havia, parada). */
  onAiCleared: (fn: () => void): Promise<UnlistenFn> => listen("vigia:ai-cleared", () => fn()),
  onOpenSettings: (fn: () => void): Promise<UnlistenFn> => listen("vigia:open-settings", () => fn()),
  onOpenConversa: (fn: () => void): Promise<UnlistenFn> => listen("vigia:open-conversa", () => fn()),
  /** Mouse entrou ou saiu da pílula (o Rust olha o cursor; o mouseleave do WebKit não é confiável aqui). */
  onNotchHover: (fn: (inside: boolean) => void): Promise<UnlistenFn> =>
    listen<boolean>("vigia:notch-hover", (e) => fn(e.payload)),
  /** Posição do mouse em pontos, relativa ao centro do topo da janela da pílula. */
  onCursor: (fn: (x: number, y: number) => void): Promise<UnlistenFn> =>
    listen<[number, number]>("vigia:cursor", (e) => fn(e.payload[0], e.payload[1])),
};

// ---------------------------------------------------------------------------
// Texto

/** Classe CSS da "lâmpada" de cada severidade. */
export const sevClass: Record<Severity, string> = {
  critical: "critica",
  high: "alta",
  medium: "media",
  low: "media",
};

export const severityLabel: Record<Severity, string> = {
  critical: "Crítica",
  high: "Alta",
  medium: "Média",
  low: "Baixa",
};

export const severityOrder: Record<Severity, number> = { critical: 0, high: 1, medium: 2, low: 3 };

export const ruleLabel: Record<string, string> = {
  secret_read: "Leitura de segredo",
  third_party_shell: "Comando suspeito",
  unsigned_exec: "Binário sem assinatura",
  persistence: "Inicialização automática",
  new_connection: "Rede",
  tcc_change: "Permissão de privacidade",
  keyboard_tap: "Leitura do teclado",
  sensor: "Funcionamento do Faro",
};

export const statusLabel: Record<AlertStatus, string> = {
  open: "Novo",
  killed: "Processo encerrado",
  quarantined: "Em quarentena",
  allowed: "Permitido",
  dismissed: "Visto",
  failed: "Ação falhou",
};

export const actionLabel: Record<ActionKind, string> = {
  kill_tree: "Encerrar processo",
  kill: "Encerrar só este",
  quarantine: "Quarentena",
  allow: "Permitir sempre",
  dismiss: "Ignorar",
  reveal: "Mostrar no Finder",
  open_settings: "Abrir Ajustes",
};

/** Ações que destroem algo pedem um segundo clique. */
export const needsConfirm = (a: ActionKind) => a === "kill_tree" || a === "kill" || a === "quarantine";

export function basename(p: string): string {
  const i = p.lastIndexOf("/");
  return i >= 0 ? p.slice(i + 1) : p;
}

export function shortPath(p: string, home = guessHome(p)): string {
  return home && p.startsWith(home + "/") ? "~" + p.slice(home.length) : p;
}

/** Troca /Users/<nome>/ por ~/ dentro de um texto corrido. */
export function tilde(text: string): string {
  return text.replace(/\/Users\/[^/\s]+\//g, "~/");
}

function guessHome(p: string): string | null {
  const m = /^\/Users\/[^/]+/.exec(p);
  return m ? m[0] : null;
}

export const ms = (iso: string) => Date.parse(iso);

export function time(iso: string): string {
  return new Date(ms(iso)).toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function dayLabel(iso: string): string {
  const d = new Date(ms(iso));
  const today = new Date();
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((start(today) - start(d)) / 86_400_000);
  if (diff === 0) return "Hoje";
  if (diff === 1) return "Ontem";
  return d.toLocaleDateString("pt-BR", { weekday: "long", day: "numeric", month: "long" });
}

// ---------------------------------------------------------------------------
// DOM sem innerHTML: todo texto vem do daemon e pode ter sido escrito por um
// processo malicioso (nomes de arquivo, argumentos).

type Child = Node | string | null | undefined | false;

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | boolean | ((e: Event) => void)> = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (typeof v === "function") el.addEventListener(k.replace(/^on/, ""), v);
    else if (v === true) el.setAttribute(k, "");
    else if (v !== false) el.setAttribute(k, v);
  }
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    el.append(typeof c === "string" ? document.createTextNode(c) : c);
  }
  return el;
}

/** Botão de ação com confirmação em dois cliques para as destrutivas. */
export function actionButton(a: Alert, action: ActionKind, run: (action: ActionKind) => void, primary = false): HTMLButtonElement {
  // Só a ação principal é vermelha; as outras destrutivas ficam vermelhas ao serem armadas.
  const red = primary && needsConfirm(action);
  const cls = [red ? "kill" : "", action === "dismiss" ? "link" : ""].filter(Boolean).join(" ");
  const btn = h("button", { class: cls, type: "button" }, actionLabel[action]);
  let timer: number | undefined;
  btn.addEventListener("click", () => {
    if (needsConfirm(action) && !btn.classList.contains("confirm")) {
      btn.classList.add("kill", "confirm");
      btn.textContent =
        action === "quarantine" ? "Sim, pôr em quarentena" : `Sim, encerrar ${basename(a.process.path)}`;
      btn.title =
        action === "kill_tree"
          ? `Encerra o pid ${a.process.pid} e todos os processos filhos`
          : action === "kill"
            ? `Encerra só o pid ${a.process.pid}`
            : "Encerra o processo e move o arquivo para a quarentena do Faro";
      timer = window.setTimeout(() => {
        btn.classList.remove("confirm");
        if (!red) btn.classList.remove("kill");
        btn.textContent = actionLabel[action];
      }, 4000);
      return;
    }
    clearTimeout(timer);
    run(action);
  });
  return btn;
}
