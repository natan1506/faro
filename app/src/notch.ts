// A pílula do topo da tela: a raposinha mora aqui o tempo todo.
//
// Fechada é o normal. Um clique na faixa abre, com abas: Início (estado e "Ver
// alertas"), Conversar (perguntas à IA) e os ajustes rápidos (engrenagem);
// outro clique fecha. Aberta e sem o mouse por perto, ela se fecha sozinha. Um alerta novo desce aberto e, se ninguém mexer, recolhe
// também: o contador e a raposinha assustada continuam na faixa.
import "./base.css";
import "./notch.css";
import {
  actionButton,
  api,
  events,
  h,
  ms,
  sevClass,
  severityLabel,
  severityOrder,
  shortPath,
  tilde,
  type ActionKind,
  type Alert,
  type AppConfig,
  type ChatMessage,
  type Status,
  type SuggestedAction,
} from "./api";
import { codexWarning, messageView, providerPicker, streamingView } from "./conversa-ui";
import { fox, lookAt, setMood, startLife, type Mood } from "./mascote";

type View = "pill" | "home" | "chat" | "quick" | "alert";
/** As vistas das abas. "alert" só existe enquanto houver alerta na fila. */
type Tab = "alert" | "home" | "chat" | "quick";

const root = document.getElementById("notch") as HTMLElement;
const EAR = 9;

let expanded = false;
let hovering = false;
let idleTimer: number | undefined;
/** Depois que o mouse sai da pílula aberta. */
const IDLE_MS = 2500;
/** Alerta novo que ninguém tocou: vem dos Ajustes (10, 15 ou 30 s). */
const alertIdleMs = () => (config?.auto_close_secs ?? 15) * 1000;
let config: AppConfig | null = null;
let openTab: Tab = "home";
let connected = false;
let status: Status | null = null;
let hasNotch = false;
let barHeight = 24;

let queue: Alert[] = [];
let current: Alert | null = null;

const DISCONNECT_GRACE_MS = 30_000;
let disconnectTimer: number | undefined;

// ---------------------------------------------------------------------------
// Estrutura fixa: a raposinha da faixa nunca é recriada, para a animação não pular.

const barFox = fox("sleep", 20);
const spacer = h("span", { class: "spacer" });
const chipCount = h("span", { class: "n" });
const chip = h("span", { class: "chip" }, h("span", { class: "dot" }), chipCount);
const bar = h("div", { class: "bar", title: "Faro" }, barFox, spacer, chip);
const body = h("div", { class: "body" });
root.append(bar, body);

const openCount = () => status?.alerts_open ?? 0;

function mood(): Mood {
  if (!connected) return "sleep";
  if (current || openCount() > 0) return "alert";
  return "calm";
}

function widthFor(v: View): number {
  if (v === "alert") return 480;
  if (v === "chat") return 480;
  if (v === "home" || v === "quick") return hasNotch ? 380 : 360;
  // Fechada: só a raposinha e o contador. Com notch físico precisa ser mais
  // larga que a câmera, para os dois aparecerem um de cada lado.
  return hasNotch ? 300 : 100;
}

// ---------------------------------------------------------------------------
// Desenho

function renderBar() {
  const m = mood();
  setMood(barFox, m);
  const n = openCount();
  // Fechada, a pílula não tem texto no meio: o estado vai só para a dica do
  // mouse e para o leitor de tela. Texto aparece quando chega um alerta (ela abre).
  let label: string;
  if (!connected) {
    chip.dataset.state = "sleep";
    chipCount.textContent = "off";
    label = "Faro parado";
  } else if (n > 0) {
    chip.dataset.state = "alert";
    chipCount.textContent = String(n);
    label = isReplay()
      ? n === 1 ? "1 alerta simulado" : `${n} alertas simulados`
      : n === 1 ? "1 alerta aberto" : `${n} alertas abertos`;
  } else {
    chip.dataset.state = "calm";
    chipCount.textContent = "";
    label = isReplay() ? "Simulação" : "Vigiando";
  }
  bar.title = label;
  bar.setAttribute("aria-label", `Faro: ${label}`);
  drawChatCtx();
}

/** O daemon está reproduzindo uma gravação (scripts/dev.sh), não olhando o Mac. */
function isReplay(): boolean {
  return connected && status?.sensor.state === "replay";
}

function currentView(): View {
  if (!expanded) return "pill";
  if (openTab === "alert") return current ? "alert" : "home";
  return openTab;
}

// ---------------------------------------------------------------------------
// Zoom ao abrir e fechar. A janela tem tamanho fixo (encolher uma janela
// transparente fazia a pílula piscar); só a área clicável acompanha a pílula.

const OPEN_MS = 380;
const CLOSE_MS = 240;
const SWAP_MS = 160;
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
/** Vista desenhada agora (durante o fechamento ainda é a aberta). */
let shownView: View = "pill";
let closeTimer: number | undefined;
let morphTimer: number | undefined;

function render() {
  const view = currentView();
  const animate = !reducedMotion.matches;
  if (view === "pill" && shownView !== "pill" && animate) {
    if (closeTimer === undefined) collapse();
    else renderBar();
    return;
  }
  // Reabriu no meio do fechamento: cancela e faz o zoom de abertura de novo.
  if (closeTimer !== undefined) {
    clearTimeout(closeTimer);
    closeTimer = undefined;
    body.classList.remove("zoom-out");
    endMorph();
    shownView = "pill";
  }
  const opening = view !== "pill" && shownView === "pill" && animate;
  // Chat e ajustes rápidos têm campos: não os refaz (perderia o foco e o que foi digitado).
  if (view === shownView && (view === "chat" || view === "quick")) {
    renderBar();
    // O topo continua valendo: o sino (e o número dele) acompanha a fila.
    updateTopRow(view);
    void fit();
    return;
  }
  const switching = view !== "pill" && shownView !== "pill" && view !== shownView && animate;
  // A classe entra antes da troca de largura, senão a transição não anima.
  if (switching) {
    root.classList.add("switching");
    void root.offsetWidth;
  }
  draw(view);
  shownView = view;
  if (opening) void expand();
  else {
    if (switching) swapAnimation();
    void fit();
  }
}

/** Troca de vista com a pílula aberta: fade + deslize de 8 px, sem o zoom. */
let swapTimer: number | undefined;
function swapAnimation() {
  clearTimeout(swapTimer);
  body.classList.remove("swap");
  void body.offsetWidth;
  body.classList.add("swap");
  root.classList.add("switching");
  swapTimer = window.setTimeout(() => {
    body.classList.remove("swap");
    root.classList.remove("switching");
  }, SWAP_MS);
}

/** Pílula → aberta: mede o tamanho final e cresce até ele. */
async function expand() {
  const from = { w: widthFor("pill"), h: barHeight };
  endMorph();
  const to = { w: widthFor(shownView), h: Math.ceil(root.getBoundingClientRect().height) };
  root.style.width = `${from.w}px`;
  root.style.height = `${from.h}px`;
  body.classList.add("zoom-in");
  void api.notchHitArea(to.w + EAR * 2, Math.max(to.h, barHeight)).catch(() => {});
  void root.offsetWidth; // fixa o tamanho inicial antes da transição
  root.classList.add("morph", "opening");
  root.style.width = `${to.w}px`;
  root.style.height = `${to.h}px`;
  clearTimeout(morphTimer);
  morphTimer = window.setTimeout(() => {
    endMorph();
    void fit(); // o conteúdo pode ter mudado durante a animação
    repedirCursor();
  }, OPEN_MS);
}

/** Aberta → pílula: o conteúdo encolhe e some, a pílula volta ao tamanho da faixa. */
function collapse() {
  const h = Math.ceil(root.getBoundingClientRect().height);
  endMorph();
  root.style.height = `${h}px`;
  void root.offsetWidth;
  root.classList.add("morph", "closing");
  body.classList.add("zoom-out");
  root.style.width = `${widthFor("pill")}px`;
  root.style.height = `${barHeight}px`;
  renderBar();
  closeTimer = window.setTimeout(() => {
    closeTimer = undefined;
    body.classList.remove("zoom-out");
    draw("pill");
    shownView = "pill";
    endMorph();
    void fit();
  }, CLOSE_MS);
}

function endMorph() {
  clearTimeout(morphTimer);
  root.classList.remove("morph", "opening", "closing");
  root.style.height = "";
  body.classList.remove("zoom-in");
}

function draw(view: View) {
  // Saindo do chat o campo some sem disparar blur: devolve o teclado ao app em uso.
  if (view !== "chat") void setFocusable(false);
  root.dataset.view = view;
  root.setAttribute("aria-expanded", String(expanded));
  root.style.width = `${widthFor(view)}px`;
  delete root.dataset.severity;
  delete root.dataset.sim;
  renderBar();

  if (view === "pill") {
    body.hidden = true;
    body.replaceChildren();
  } else if (view === "alert") {
    if (current) {
      root.dataset.severity = sevClass[current.severity];
      if (current.simulated) root.dataset.sim = "1";
      body.hidden = false;
      updateTopRow(view);
      body.replaceChildren(topRow, ...alertContent(current));
    }
  } else {
    body.hidden = false;
    updateTopRow(view);
    const content = view === "home" ? homeContent() : view === "chat" ? chatContent() : quickContent();
    body.replaceChildren(topRow, content);
    if (view === "chat") void enterChat();
  }
  repedirCursor();
}

/** A área clicável acompanha a pílula: nada de área invisível bloqueando cliques. */
async function fit() {
  await new Promise(requestAnimationFrame);
  // No meio do zoom a altura é a da animação; quem termina a animação chama fit de novo.
  if (root.classList.contains("morph")) return;
  const height = Math.ceil(root.getBoundingClientRect().height);
  await api.notchHitArea(widthFor(currentView()) + EAR * 2, Math.max(height, barHeight)).catch(() => {});
}

function homeContent(): Node {
  const n = openCount();
  const s = status;
  const line1 = !connected ? "O daemon não está respondendo" : n > 0 ? (n === 1 ? "1 alerta aberto" : `${n} alertas abertos`) : "Tudo calmo por aqui";
  const parts: string[] = [];
  if (connected && s) {
    parts.push(`${Math.round(s.events_per_sec * 60).toLocaleString("pt-BR")} eventos/min`);
    if (s.sensor.state === "down" || s.sensor.state === "restarting") parts.push("sensor parado");
    if (s.learning_until && ms(s.learning_until) > Date.now()) parts.push("aprendendo a rede");
  } else if (!connected) {
    parts.push("nenhum evento está sendo analisado");
  }
  return h(
    "div",
    { class: "home" },
    h("span", { class: "big-fox" }, fox(mood(), 44)),
    h("span", { class: "line1" }, line1),
    h("span", { class: "line2" }, parts.join(" · ")),
    h(
      "span",
      { class: "links" },
      h("button", { type: "button", onclick: () => void openAndClose(() => api.openPainel(null)) }, n > 0 ? "Ver alertas" : "Abrir painel"),
    ),
  );
}

/** Abre o painel e recolhe a pílula, para ela não ficar aberta por cima. */
async function openAndClose(fn: () => Promise<void>) {
  await fn();
  setExpanded(false);
}

// ---------------------------------------------------------------------------
// Linha de cima das vistas abertas: abas à esquerda, ajustes rápidos à direita.

const NS = "http://www.w3.org/2000/svg";
function svgIcon(paths: string[], stroke = false): SVGSVGElement {
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  if (stroke) svg.classList.add("stroke");
  for (const d of paths) {
    const path = document.createElementNS(NS, "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

const ICON_HOME = ["M12 3 2 12h3v8h5v-6h4v6h5v-8h3z"];
const ICON_BELL = [
  "M12 22a2.5 2.5 0 0 0 2.45-2h-4.9A2.5 2.5 0 0 0 12 22zm7-6v-5a7 7 0 0 0-5.5-6.84V3.5a1.5 1.5 0 0 0-3 0v.66A7 7 0 0 0 5 11v5l-2 2v1h18v-1z",
];
const ICON_SEND = ["M12 19V5M5 12l7-7 7 7"];
const ICON_STOP = ["M7 7h10v10H7z"];
const ICON_CHAT = ["M5 3h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2h-8l-5 4v-4H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"];
const ICON_CONE = "M3 9v6h4l5 5V4L7 9z";
const ICON_WAVES = "M15.5 8.5a5 5 0 0 1 0 7M18 6a8.5 8.5 0 0 1 0 12";
const ICON_CLOSE = ["M6 6l12 12M18 6 6 18"];

function iconButton(cls: string, label: string, icon: SVGSVGElement, onclick: () => void): HTMLButtonElement {
  return h("button", { class: `ib ${cls}`, type: "button", title: label, "aria-label": label, onclick }, icon) as HTMLButtonElement;
}

const tabAlertCount = h("span", { class: "tab-n", "aria-hidden": "true" });
const tabAlert = iconButton("tab-alert", "Alertas", svgIcon(ICON_BELL), () => setTab("alert"));
tabAlert.append(tabAlertCount);
const tabHome = iconButton("tab-home", "Início", svgIcon(ICON_HOME), () => setTab("home"));
const tabChat = iconButton("tab-chat", "Conversar", svgIcon(ICON_CHAT), () => setTab("chat"));
const btnGear = iconButton("gear", "Ajustes rápidos", gearIcon(), () => setTab(openTab === "quick" ? "home" : "quick"));
const btnSound = iconButton("sound", "Som dos alertas", speakerIcon(true), () => void toggleSound());
const btnClose = iconButton("x", "Fechar", svgIcon(ICON_CLOSE, true), () => setExpanded(false));
const topRow = h(
  "div",
  { class: "toprow" },
  h("span", { class: "tabs", role: "group", "aria-label": "Vistas" }, tabAlert, tabHome, tabChat),
  h("span", { class: "tools" }, btnGear, btnSound, btnClose),
);

function speakerIcon(on: boolean): SVGSVGElement {
  const svg = svgIcon([ICON_CONE]);
  if (on) {
    const w = document.createElementNS(NS, "path");
    w.setAttribute("d", ICON_WAVES);
    w.setAttribute("class", "waves");
    svg.append(w);
  }
  return svg;
}

function updateTopRow(view: View) {
  // A aba de alertas só aparece com alerta na fila; o número é quantos esperam.
  const pendentes = (current ? 1 : 0) + queue.length;
  tabAlert.hidden = !current;
  tabAlertCount.textContent = pendentes > 1 ? String(pendentes) : "";
  tabAlert.setAttribute("aria-pressed", String(view === "alert"));
  const rotulo = pendentes === 1 ? "Alertas: 1 esperando decisão" : `Alertas: ${pendentes} esperando decisão`;
  tabAlert.title = rotulo;
  tabAlert.setAttribute("aria-label", rotulo);
  tabHome.setAttribute("aria-pressed", String(view === "home"));
  tabChat.setAttribute("aria-pressed", String(view === "chat"));
  btnGear.setAttribute("aria-pressed", String(view === "quick"));
  const on = config?.sound.enabled ?? true;
  btnSound.setAttribute("aria-pressed", String(on));
  const label = on ? "Som ligado: clique para silenciar" : "Som desligado: clique para ligar";
  btnSound.title = label;
  btnSound.setAttribute("aria-label", label);
  btnSound.replaceChildren(speakerIcon(on));
  btnSound.classList.toggle("off", !on);
}

function setTab(t: Tab) {
  if (openTab === t) return;
  if (t === "alert" && !current) return;
  openTab = t;
  render();
  armIdle();
}

// ---------------------------------------------------------------------------
// Configuração (som e tempo de fechamento) compartilhada com o painel.

let cfgVersion = 0;
async function saveConfig(change: (c: AppConfig) => void) {
  if (!config) return;
  const next: AppConfig = JSON.parse(JSON.stringify(config));
  change(next);
  const before = config;
  const mine = ++cfgVersion;
  config = next;
  refreshConfigViews();
  try {
    const saved = await api.saveAppConfig(next);
    if (mine === cfgVersion) config = saved;
    if (chatMsg?.startsWith("Não foi possível salvar o ajuste")) chatMsg = null;
  } catch (e) {
    chatMsg = `Não foi possível salvar o ajuste: ${e}`;
    // Só volta atrás se nada mais novo chegou (por exemplo, do painel).
    if (mine === cfgVersion) config = before;
  }
  refreshConfigViews();
}

async function toggleSound() {
  await saveConfig((c) => (c.sound.enabled = !c.sound.enabled));
}

/** A configuração mudou (aqui ou no painel): atualiza o que está na tela. */
function refreshConfigViews() {
  if (!expanded) return;
  const view = currentView();
  // Na aba Alertas o corpo não é refeito, mas o alto-falante do topo sim.
  updateTopRow(view);
  if (view === "alert") return;
  if (view === "quick") {
    // Não refaz enquanto o usuário arrasta o volume.
    if (document.activeElement instanceof HTMLInputElement && quickEl?.contains(document.activeElement)) return;
    quickEl = null;
    body.replaceChildren(topRow, quickContent());
  } else if (view === "chat") {
    drawChatTop();
    drawChatAnswer();
  }
}

let quickEl: HTMLElement | null = null;
function quickContent(): Node {
  const c = config;
  if (!c) {
    quickEl = h("div", { class: "quick" }, h("p", { class: "muted" }, "Carregando…"));
    return quickEl;
  }
  const on = c.sound.enabled;
  const sw = h("input", { type: "checkbox", role: "switch", class: "switch", "aria-label": "Som dos alertas" }) as HTMLInputElement;
  sw.checked = on;
  sw.addEventListener("change", () => void saveConfig((x) => (x.sound.enabled = sw.checked)));
  const vol = h("input", { type: "range", min: "0", max: "1", step: "0.05", class: "volume", "aria-label": "Volume" }) as HTMLInputElement;
  vol.value = String(c.sound.volume);
  vol.disabled = !on;
  vol.addEventListener("change", () => void saveConfig((x) => (x.sound.volume = Number(vol.value))));
  const segs = [10, 15, 30].map((n) => {
    const b = h("button", { type: "button", class: "seg", "aria-pressed": String(c.auto_close_secs === n), onclick: () => void saveConfig((x) => (x.auto_close_secs = n)) }, `${n} s`);
    return b;
  });
  quickEl = h(
    "div",
    { class: "quick" },
    h("div", { class: "qrow" }, h("span", { class: "qlabel" }, "Som"), sw),
    h("div", { class: "qrow" }, h("span", { class: "qlabel" }, "Volume"), vol),
    h("div", { class: "qrow" }, h("span", { class: "qlabel" }, "Fechar automático"), h("span", { class: "segs", role: "group", "aria-label": "Fechar automático" }, ...segs)),
    chatMsg ? h("p", { class: "chat-note", role: "alert" }, chatMsg) : null,
    h("button", { type: "button", class: "more", onclick: () => void openAndClose(() => api.openSettings()) }, "Configurações…"),
  );
  return quickEl;
}

// ---------------------------------------------------------------------------
// Conversar: só a última resposta; a conversa inteira fica no painel.

let chatEl: HTMLElement | null = null;
let chatTop: HTMLElement;
let chatAnswer: HTMLElement;
let chatNote: HTMLElement;
let chatInput: HTMLInputElement;
let chatSend: HTMLButtonElement;
let chatStop: HTMLButtonElement;
let chatCodexTag: HTMLElement;
let chatCtx: HTMLElement;
let chatFox: HTMLElement;
let providers: string[] = [];
let lastAnswer: ChatMessage | null = null;
let chatBusy = false;
let chatPartial = "";
let chatStream: ReturnType<typeof streamingView> | null = null;
let chatMsg: string | null = null;

const codexBlocked = () => !!config && config.ai.provider === "codex" && !config.ai.codex_ack_at;

/**
 * Chat: um card escuro com brilho, a raposinha grande à
 * esquerda (olhos vivos) e, à direita, o chip de contexto, a última resposta,
 * o badge da IA e o campo com o botão redondo de enviar.
 */
function chatContent(): Node {
  if (chatEl) return chatEl;
  chatTop = h("div", { class: "chat-top" });
  // Saiu do seletor ou do campo "Outro…": redesenha com o config atual.
  chatTop.addEventListener("focusout", () => window.setTimeout(drawChatTop, 0));
  chatAnswer = h("div", { class: "chat-answer", role: "log" });
  chatNote = h("p", { class: "chat-note", hidden: true, "aria-live": "polite" });
  chatCtx = h("span", { class: "chat-ctx-chip" });
  chatFox = fox(mood(), 64);
  chatInput = h("input", { type: "text", class: "chat-input", "aria-label": "Pergunta", placeholder: "Pergunte sobre o seu Mac…", spellcheck: "false" }) as HTMLInputElement;
  chatInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing) {
      e.preventDefault();
      void ask(chatInput.value);
    }
  });
  chatInput.addEventListener("focus", () => clearTimeout(idleTimer));
  chatSend = h(
    "button",
    { type: "button", class: "send", title: "Enviar (Enter)", "aria-label": "Enviar", onclick: () => void ask(chatInput.value) },
    svgIcon(ICON_SEND, true),
  ) as HTMLButtonElement;
  chatStop = h(
    "button",
    { type: "button", class: "send stop", title: "Parar a resposta", "aria-label": "Parar", onclick: () => void api.aiStop() },
    svgIcon(ICON_STOP),
  ) as HTMLButtonElement;
  chatCodexTag = h("span", { class: "tag-codex", title: "O Codex pode abrir arquivos do Mac" }, "Codex: pode ler arquivos");
  chatEl = h(
    "div",
    { class: "chat" },
    h(
      "div",
      { class: "chat-card" },
      h("div", { class: "chat-side" }, chatFox),
      h(
        "div",
        { class: "chat-main" },
        h(
          "div",
          { class: "chat-ctx" },
          chatCtx,
          h("button", { type: "button", class: "more", onclick: () => void openAndClose(() => api.openConversa()) }, "Abrir no painel"),
        ),
        chatAnswer,
        chatNote,
        h("div", { class: "chat-meta" }, chatCodexTag, chatTop),
        h("div", { class: "chat-compose" }, chatInput, chatSend, chatStop),
      ),
    ),
  );
  return chatEl;
}

/** Chip de contexto do chat: o Vigia e quantos alertas estão abertos. */
function drawChatCtx() {
  if (!chatEl) return;
  setMood(chatFox, mood());
  const n = openCount();
  chatCtx.dataset.state = !connected ? "sleep" : n > 0 ? "alert" : "calm";
  chatCtx.replaceChildren(
    h("span", { class: "dot", "aria-hidden": "true" }),
    !connected ? "faro · parado" : n > 0 ? `faro · ${n === 1 ? "1 alerta" : `${n} alertas`}` : "faro",
  );
}

let focusable = false;
/** Estado pedido; um único executor manda o IPC até o confirmado igualar o pedido. */
let desired = false;
let focusRunner: Promise<void> | null = null;
function setFocusable(on: boolean): Promise<void> {
  desired = on;
  if (!focusRunner) {
    const rodar = async () => {
      while (focusable !== desired) {
        const alvo = desired;
        try {
          await api.notchFocusable(alvo);
          focusable = alvo;
        } catch {
          break; // sem backend (pré-visualização): segue sem foco
        }
      }
    };
    // O finally roda depois da atribuição (mesmo se rodar() terminar de imediato).
    focusRunner = rodar().finally(() => {
      focusRunner = null;
    });
  }
  return focusRunner;
}
const isTextField = (t: EventTarget | null): t is HTMLInputElement | HTMLTextAreaElement =>
  t instanceof HTMLTextAreaElement || (t instanceof HTMLInputElement && t.type === "text");
// A pílula não é focável (não rouba o foco do app em uso): clicar em qualquer
// campo de texto dela (pergunta, "Outro…" do modelo) a torna focável.
root.addEventListener("mousedown", (e) => {
  const t = e.target;
  if (isTextField(t) && !t.disabled && !desired) {
    e.preventDefault();
    void setFocusable(true).then(() => t.focus());
  }
});
root.addEventListener("focusout", () => {
  // Foco andando dentro da pílula (campo → campo, Enviar, seletores) não desliga nada.
  window.setTimeout(() => {
    if (!root.contains(document.activeElement)) void setFocusable(false);
    armIdle();
  }, 0);
});
// O usuário trocou de app: a janela perdeu o foco de verdade.
window.addEventListener("blur", () => void setFocusable(false));

async function enterChat() {
  const [c, p, conv] = await Promise.all([
    api.getAppConfig().catch(() => config),
    api.aiProviders().catch(() => [] as string[]),
    api.aiConversation().catch(() => null),
  ]);
  config = c;
  providers = p;
  if (conv && !chatBusy) lastAnswer = [...conv].reverse().find((m) => m.role !== "voce") ?? null;
  if (config && providers.length > 0 && !providers.includes(config.ai.provider)) {
    await saveConfig((x) => {
      x.ai.provider = providers[0] as "claude" | "codex";
      x.ai.model = "";
      x.ai.codex_ack_at = null;
    });
  }
  if (currentView() !== "chat") return;
  updateTopRow("chat");
  drawChat();
}

function drawChat() {
  drawChatCtx();
  drawChatTop();
  drawChatAnswer();
  drawChatCompose();
}

function drawChatTop() {
  if (!chatEl || !config) return;
  // Não refaz o seletor enquanto o usuário digita o nome de um modelo.
  if (document.activeElement instanceof HTMLInputElement && chatTop.contains(document.activeElement)) return;
  chatTop.replaceChildren(
    providers.length > 0
      ? providerPicker(config, providers, (n) => void saveConfig((x) => Object.assign(x, n)), {
          // O campo "Outro…" só recebe o teclado com a pílula focável.
          antesDeFocar: () => setFocusable(true),
          aoMudarTamanho: () => void fit(),
        })
      : h("span", {}),
  );
  drawChatCompose();
}

function drawChatAnswer() {
  const itens: Node[] = [];
  if (providers.length === 0) {
    itens.push(h("p", { class: "chat-empty" }, "Nenhuma IA encontrada. Instale o Claude Code ou o Codex para conversar."));
  } else if (lastAnswer && !chatBusy) {
    itens.push(messageView(lastAnswer, (s) => void onSuggestion(s)));
  } else if (!chatBusy) {
    itens.push(h("p", { class: "chat-empty" }, "Pergunte sobre o que o Faro está vendo."));
  }
  if (chatBusy) {
    chatStream = streamingView();
    chatStream.set(chatPartial);
    itens.push(chatStream.el);
  } else chatStream = null;
  chatAnswer.replaceChildren(...itens);
  chatAnswer.scrollTop = chatAnswer.scrollHeight;
  chatNote.textContent = chatMsg ?? "";
  chatNote.hidden = !chatMsg;
  void fit();
}

function drawChatCompose() {
  if (!chatEl) return;
  const sem = providers.length === 0;
  const bloq = codexBlocked();
  chatInput.disabled = sem || bloq;
  chatSend.disabled = sem || bloq || chatBusy;
  chatSend.hidden = chatBusy;
  chatStop.hidden = !chatBusy;
  // Como no painel: com o Codex escolhido (e aceito), a etiqueta fica ao lado do campo.
  chatCodexTag.hidden = !(config?.ai.provider === "codex" && !!config.ai.codex_ack_at);
  chatEl.querySelector(".codex-aviso")?.remove();
  if (bloq && !sem) {
    chatAnswer.after(
      codexWarning(
        async () => {
          try {
            config = await api.aiAcceptCodex();
          } catch (e) {
            chatMsg = `Não foi possível salvar: ${e}`;
          }
          drawChat();
        },
        () => void saveConfig((x) => ((x.ai.provider = "claude"), (x.ai.model = ""))).then(drawChat),
        providers.includes("claude"),
      ),
    );
  }
  void fit();
}

async function ask(texto: string) {
  const q = texto.trim();
  if (!q || chatBusy || codexBlocked() || providers.length === 0) return;
  chatInput.value = "";
  chatMsg = null;
  chatBusy = true;
  chatPartial = "";
  drawChat();
  try {
    await api.aiAsk(q, null);
  } catch (e) {
    const t = String(e);
    if (/espere/i.test(t)) {
      chatMsg = t; // já há uma resposta em andamento (iniciada no painel)
    } else {
      chatBusy = false;
      lastAnswer = { role: "erro", text: t, provider: null, model: null, actions: [] };
    }
    drawChat();
  }
}

/** Botão de sugestão da IA: acha o alerta (fila da pílula ou histórico) e executa. */
async function onSuggestion(s: SuggestedAction) {
  let a: Alert | undefined = [current, ...queue].find((x): x is Alert => !!x && x.id === s.alert_id);
  if (!a) a = (await api.history(500).catch(() => [] as Alert[])).find((x) => x.id === s.alert_id);
  if (!a || a.status !== "open" || !a.actions.includes(s.action)) {
    chatMsg = "Esse alerta já foi resolvido.";
    drawChatAnswer();
    return;
  }
  try {
    const m = await api.act(a, s.action);
    chatMsg = m ? m.charAt(0).toUpperCase() + m.slice(1) + "." : null;
  } catch (e) {
    chatMsg = String(e);
  }
  drawChatAnswer();
  void refreshStatus();
}

function onAiChunk(t: string) {
  if (!chatBusy) {
    chatBusy = true; // pergunta feita pelo painel
    chatPartial = "";
    if (chatEl) drawChat();
  }
  chatPartial += t;
  if (chatStream) {
    chatStream.set(chatPartial);
    chatAnswer.scrollTop = chatAnswer.scrollHeight;
    void fit();
  }
}

function onAiDone(m: ChatMessage) {
  chatBusy = false;
  chatPartial = "";
  lastAnswer = m.role === "voce" ? lastAnswer : m;
  if (chatEl) drawChat();
  armIdle();
}

function onAiCleared() {
  chatBusy = false;
  chatPartial = "";
  lastAnswer = null;
  chatMsg = null;
  if (chatEl) drawChat();
}

function gearIcon(): SVGSVGElement {
  return svgIcon([
    "M19.14 12.94a7.07 7.07 0 0 0 .06-.94 7.07 7.07 0 0 0-.06-.94l2.03-1.58a.5.5 0 0 0 .12-.64l-1.92-3.32a.5.5 0 0 0-.6-.22l-2.39.96a7.03 7.03 0 0 0-1.63-.94l-.36-2.54A.5.5 0 0 0 13.9 2.5h-3.84a.5.5 0 0 0-.49.42l-.36 2.54c-.59.24-1.14.55-1.63.94l-2.39-.96a.5.5 0 0 0-.6.22L2.67 8.98a.5.5 0 0 0 .12.64l2.03 1.58a7.07 7.07 0 0 0 0 1.88l-2.03 1.58a.5.5 0 0 0-.12.64l1.92 3.32c.13.22.39.31.6.22l2.39-.96c.49.39 1.04.7 1.63.94l.36 2.54c.04.24.25.42.49.42h3.84c.24 0 .45-.18.49-.42l.36-2.54c.59-.24 1.14-.55 1.63-.94l2.39.96c.22.09.47 0 .6-.22l1.92-3.32a.5.5 0 0 0-.12-.64zM12 15.5A3.5 3.5 0 1 1 12 8.5a3.5 3.5 0 0 1 0 7z",
  ]);
}

function islandActions(a: Alert): ActionKind[] {
  const prefer: ActionKind[] = ["kill_tree", "quarantine", "open_settings", "kill"];
  const first = prefer.find((x) => a.actions.includes(x));
  const out: ActionKind[] = [];
  if (first) out.push(first);
  if (a.actions.includes("allow")) out.push("allow");
  return out;
}

function alertContent(a: Alert): Node[] {
  const repeat = a.count > 1 ? ` ×${a.count}` : "";
  const parts: (Node | null)[] = [
    h(
      "div",
      { class: "head" },
      fox("alert", 30),
      h("span", { class: "title", title: a.title }, a.title + repeat),
      a.simulated ? h("span", { class: "sim-tag" }, "Simulação") : h("span", { class: "sev" }, severityLabel[a.severity]),
      h(
        "button",
        {
          class: "ignorar",
          type: "button",
          "aria-label": "Ignorar este alerta (Esc)",
          title: "Ignorar este alerta (Esc)",
          onclick: () => act(a, "dismiss"),
        },
        "Ignorar",
      ),
    ),
    a.simulated ? h("p", { class: "sim-note" }, "Evento de teste gravado. Nada disso aconteceu no seu Mac, e os botões não executam nada.") : null,
    h("p", { class: "detail" }, tilde(a.detail)),
    a.target && a.target !== "teclado" ? h("div", { class: "target mono", title: a.target }, shortPath(a.target)) : null,
    h(
      "div",
      { class: "actions" },
      ...islandActions(a).map((x, i) => actionButton(a, x, (act_) => act(a, act_), i === 0)),
      h("button", { class: "link", type: "button", onclick: () => void api.openPainel(a.id > 0 ? a.id : null) }, "Detalhes"),
      queue.length > 0 ? h("button", { class: "link queue", type: "button", onclick: () => skip() }, `Mais ${queue.length}`) : null,
    ),
  ];
  return parts.filter((p): p is Node => p !== null);
}

// ---------------------------------------------------------------------------
// Fila de alertas

const byPriority = (x: Alert, y: Alert) => severityOrder[x.severity] - severityOrder[y.severity] || ms(x.ts) - ms(y.ts);

function enqueue(a: Alert) {
  if (current && current.id !== 0 && current.id === a.id) return;
  if (queue.some((q) => q.id !== 0 && q.id === a.id)) return;
  queue.push(a);
  queue.sort(byPriority);
  if (!current || severityOrder[a.severity] < severityOrder[current.severity]) {
    if (current) queue.unshift(current);
    current = queue.shift() ?? null;
  }
  // Alerta novo desce aberto na aba de alertas; sem interação, recolhe e fica
  // no contador. Quem está digitando no chat (ou esperando resposta) não é
  // arrancado de lá: a aba de alertas aparece com o número.
  const conversando = expanded && openTab === "chat" && typingInChat();
  if (!conversando) openTab = "alert";
  expanded = true;
  render();
  armIdle(alertIdleMs());
}

function update(a: Alert) {
  if (a.status !== "open") {
    queue = queue.filter((q) => q.id !== a.id);
    if (current?.id === a.id) showNext();
    else render();
    return;
  }
  if (current?.id === a.id) {
    current = a;
    render();
  }
}

/** Passa para o próximo alerta da fila. Se acabou, a pílula fecha. */
function showNext() {
  current = queue.shift() ?? null;
  if (!current) {
    // Acabaram os alertas: na aba deles, fecha (caminho único: solta o foco e
    // volta ao Início); em outra aba (chat, ajustes), fica onde está.
    if (expanded && openTab === "alert") setExpanded(false);
    else render();
    return;
  }
  render();
  armIdle();
}

function skip() {
  if (current) queue.push(current);
  showNext();
}

const doneText: Partial<Record<ActionKind, string>> = {
  kill: "Processo encerrado",
  kill_tree: "Processo e filhos encerrados",
  quarantine: "Em quarentena",
  allow: "Exceção criada",
  dismiss: "Marcado como visto",
};

async function act(a: Alert, action: ActionKind) {
  clearTimeout(idleTimer);
  if (a.id === 0) {
    showNext();
    return;
  }
  try {
    await api.act(a, action);
    if (action === "open_settings" || action === "reveal") return;
    setMood(barFox, "calm");
    body.replaceChildren(h("div", { class: "done" }, fox("calm", 22), doneText[action] ?? "Feito"));
    void fit();
    void refreshStatus();
    window.setTimeout(showNext, 1100);
  } catch (e) {
    body.querySelector(".error")?.remove();
    body.append(h("p", { class: "error", role: "alert" }, String(e)));
    void fit();
  }
}

// ---------------------------------------------------------------------------
// Mouse: clique na faixa abre e fecha; sem o mouse por perto, fecha sozinha.

function setExpanded(v: boolean) {
  if (expanded === v) return;
  expanded = v;
  if (v && current) openTab = "alert";
  if (!v) {
    openTab = "home";
    chatMsg = null;
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    void setFocusable(false);
  }
  render();
  armIdle();
}

/** Agenda o recolhimento. Enquanto o mouse estiver em cima, nada acontece. */
function armIdle(ms = IDLE_MS) {
  clearTimeout(idleTimer);
  if (!expanded || hovering || typingInChat()) return;
  idleTimer = window.setTimeout(() => {
    if (!hovering && !typingInChat()) setExpanded(false);
  }, ms);
}

/** Digitando na conversa ou esperando a resposta: a pílula não fecha sozinha. */
function typingInChat(): boolean {
  if (currentView() !== "chat") return false;
  return chatBusy || (!!chatEl && isTextField(document.activeElement) && chatEl.contains(document.activeElement));
}

// ---------------------------------------------------------------------------
// Cursor. A pílula não é a janela ativa, e o WebKit só troca o cursor na
// janela ativa: o CSS (mão nos botões) não chegava à tela. Com a posição do
// mouse que o Rust manda, a pílula descobre o elemento embaixo e pede o cursor.

let cursorAtual: "pointer" | "text" | "default" = "default";
/** Última posição do mouse (relativa à janela), para pedir o cursor de novo após redesenhar. */
let ultimoMouse: { x: number; y: number } | null = null;
function cursorPara(el: Element): "pointer" | "text" | "default" {
  if (isTextField(el)) return el.disabled ? "default" : "text";
  return getComputedStyle(el).cursor === "pointer" ? "pointer" : "default";
}
/**
 * Só mexe no cursor com o mouse sobre a pílula. Fora dela os cliques atravessam
 * e o macOS já mostra o cursor do app de baixo; pedir "seta" ali sobrescrevia
 * esse cursor (piscava mão → barra de texto → seta). Ao sair, só esquece o atual.
 */
function pedirCursor(el: Element | null) {
  if (!el || !root.contains(el)) {
    cursorAtual = "default";
    return;
  }
  const c = cursorPara(el);
  if (c === cursorAtual) return;
  cursorAtual = c;
  void api.notchCursor(c).catch(() => {});
}

/** O conteúdo embaixo do mouse mudou (redesenho, troca de aba) sem o mouse mexer. */
function repedirCursor() {
  requestAnimationFrame(() => {
    if (ultimoMouse && hovering) pedirCursor(document.elementFromPoint(ultimoMouse.x, ultimoMouse.y));
  });
}

function setHovering(v: boolean) {
  hovering = v;
  // Saiu da pílula: não pede cursor nenhum, o app de baixo cuida do dele.
  if (!v) pedirCursor(null);
  if (v) clearTimeout(idleTimer);
  else armIdle();
}
// O Rust avisa entrada e saída (events.onNotchHover, em main); os eventos do
// DOM ficam para a pré-visualização no navegador.
root.addEventListener("mouseenter", () => setHovering(true));
root.addEventListener("mouseleave", () => setHovering(false));
bar.addEventListener("click", () => setExpanded(!expanded));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") setExpanded(false);
});

// ---------------------------------------------------------------------------

function daemonDown(): Alert {
  const now = new Date().toISOString();
  return {
    id: 0,
    ts: now,
    first_ts: now,
    rule: "sensor",
    severity: "critical",
    title: "O Faro parou de vigiar",
    detail:
      "O daemon não responde há mais de 30 segundos, então nenhum evento está sendo analisado. Verifique com: sudo launchctl print system/dev.natan.vigia",
    process: { pid: 0, ppid: 0, path: "faro-daemon" },
    actions: ["dismiss"],
    count: 1,
    status: "open",
  };
}

async function refreshStatus() {
  try {
    status = await api.status();
  } catch {
    // sem daemon: o estado de conexão já cuida do desenho
  }
  if (currentView() !== "alert") render();
  else renderBar();
}

async function main() {
  try {
    const m = await api.notchMetrics();
    hasNotch = m.has_notch;
    barHeight = Math.round(m.menubar);
  } catch {
    // fora do app (pré-visualização): padrão sem notch
  }
  root.dataset.hardware = hasNotch ? "notch" : "flat";
  document.documentElement.style.setProperty("--bar", `${barHeight}px`);
  barFox.style.width = barFox.style.height = `${Math.max(18, barHeight - 4)}px`;

  config = await api.getAppConfig().catch(() => null);
  await events.onAppConfig((c) => {
    cfgVersion++;
    config = c;
    refreshConfigViews();
  });
  await events.onAiChunk(onAiChunk);
  await events.onAiDone(onAiDone);
  await events.onAiCleared(onAiCleared);
  await events.onNotchHover(setHovering);
  // O cursor vem relativo ao centro do topo da janela.
  await events.onCursor((x, y) => {
    ultimoMouse = { x: window.innerWidth / 2 + x, y };
    lookAt(ultimoMouse.x, y);
    pedirCursor(document.elementFromPoint(ultimoMouse.x, y));
  });
  startLife();
  await events.onAlert((a, isNew) => {
    if (isNew && a.status === "open") enqueue(a);
    else update(a);
    void refreshStatus();
  });
  await events.onStatus((s) => {
    status = s;
    if (currentView() !== "alert") render();
    else renderBar();
  });
  await events.onDaemon((c) => {
    connected = c;
    clearTimeout(disconnectTimer);
    if (!c) {
      disconnectTimer = window.setTimeout(() => enqueue(daemonDown()), DISCONNECT_GRACE_MS);
    } else {
      queue = queue.filter((q) => !(q.id === 0 && q.rule === "sensor"));
      if (current && current.id === 0 && current.rule === "sensor") showNext();
      void refreshStatus();
    }
    render();
  });

  connected = await api.connected();
  if (!connected) {
    disconnectTimer = window.setTimeout(async () => {
      if (!(await api.connected())) enqueue(daemonDown());
    }, DISCONNECT_GRACE_MS);
  }
  render();
  await refreshStatus();
  window.setInterval(() => void refreshStatus(), 5000);
  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && current) void act(current, "dismiss");
  });
}

void main();
