// Painel: barra lateral com as seções (alertas, atividade, ajustes…). Aqui ficam
// o histórico de alertas com a linhagem de processos e a lista de processos recentes.
import "./base.css";
import "./painel.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  actionButton,
  api,
  basename,
  dayLabel,
  events,
  h,
  ms,
  ruleLabel,
  sevClass,
  severityLabel,
  shortPath,
  statusLabel,
  tilde,
  time,
  type ActionKind,
  type Alert,
  type ExecEntry,
  type ProcInfo,
  type Status,
} from "./api";

import { loadAjustes, renderCelular, renderPastas, renderRegras } from "./ajustes";
import { estadoTodos, podar, podeMarcar } from "./selecao";
import { CELULAR } from "./recursos";
import { renderInicio } from "./painel/inicio";
import { askAbout, renderConversa, setConversaCtx } from "./painel/conversa";
import { renderExcecoes } from "./painel/excecoes";
import { renderSons } from "./painel/sons";
import { fox, setMood } from "./mascote";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const headerFox = fox("sleep", 22);

let alerts: Alert[] = [];
let selected: number | null = null;
let connected = false;
let lastStatus: Status | null = null;
let statusError: string | null = null;

// ---------------------------------------------------------------------------
// Estado do daemon

function renderState() {
  const lamp = $("state-lamp");
  const text = $("state-text");
  const metrics = $("metrics");
  const s = lastStatus;
  let cls = "lamp";
  let label = "Conectando…";
  if (!connected || statusError) {
    cls += " critica";
    label = "Daemon desconectado";
  } else if (s?.sensor.state === "down" || s?.sensor.state === "restarting") {
    cls += " critica";
    label = "Sensor parado";
  } else if (s?.sensor.state === "running" || s?.sensor.state === "replay") {
    cls += " calm";
    label = s.sensor.state === "replay" ? "Simulação (replay de teste)" : "Vigiando";
  }
  lamp.className = cls;
  text.textContent = label;
  setMood(headerFox, !connected || statusError || cls.includes("critica") ? "sleep" : (s?.alerts_open ?? 0) > 0 ? "alert" : "calm");

  metrics.replaceChildren();
  if (s && connected) {
    metrics.append(
      h("span", {}, `${Math.round(s.events_per_sec * 60).toLocaleString("pt-BR")} eventos/min`),
      h("span", {}, s.alerts_open === 1 ? "1 alerta aberto" : `${s.alerts_open} alertas abertos`),
    );
    if (s.events_dropped > 0) {
      metrics.append(h("span", { title: "O motor de regras não deu conta do volume e descartou eventos." }, `${s.events_dropped.toLocaleString("pt-BR")} descartados`));
    }
    if (s.learning_until && ms(s.learning_until) > Date.now()) {
      const until = new Date(ms(s.learning_until)).toLocaleString("pt-BR", { weekday: "short", hour: "2-digit", minute: "2-digit" });
      metrics.append(h("span", { title: "Até lá, a regra de rede só aprende quais programas usam a internet." }, `Aprendendo a rede até ${until}`));
    }
  }
  renderBanners();
}

function banner(title: string, body: string): HTMLElement {
  return h(
    "div",
    { class: "banner", role: "alert" },
    h("span", { class: "lamp critica", "aria-hidden": "true", style: "margin-top:4px" }),
    h("div", {}, h("strong", {}, title), h("p", {}, body)),
  );
}

function renderBanners() {
  const box = $("banners");
  box.replaceChildren();
  if (!connected || statusError) {
    box.append(
      banner(
        "O app não consegue falar com o daemon",
        `Sem o daemon nada é analisado. Confira se ele está rodando com "sudo launchctl print system/dev.natan.vigia" e veja o log em /Library/Logs/Vigia/daemon.log.${statusError ? ` Erro: ${statusError}` : ""}`,
      ),
    );
    return;
  }
  const s = lastStatus;
  if (!s) return;
  if (s.sensor.state === "down" || s.sensor.state === "restarting") {
    box.append(
      banner(
        "O sensor parou",
        `${s.sensor.reason}. Em geral é falta de Acesso Total ao Disco: em Ajustes > Privacidade e Segurança > Acesso Total ao Disco, adicione /usr/local/vigia/vigia-daemon.`,
      ),
    );
  }
}

async function refreshStatus() {
  try {
    lastStatus = await api.status();
    statusError = null;
    connected = true;
  } catch (e) {
    statusError = String(e);
  }
  renderState();
}

// ---------------------------------------------------------------------------
// Lista de alertas

function visibleAlerts(): Alert[] {
  const onlyNew = $<HTMLInputElement>("only-new").checked;
  return onlyNew ? alerts.filter((a) => a.status === "open") : alerts;
}

// ---------------------------------------------------------------------------
// Seleção em massa: só "Ignorar". Encerrar, quarentena e "Permitir sempre"
// continuam um a um no detalhe: em lote, um clique escondia o alerta certo.

let marcados = new Set<number>();
let emMassa: { feitos: number; total: number } | null = null;
let confirmando = false;
let confirmTimer: number | undefined;
let avisoMassa: { texto: string; erro: boolean } | null = null;

function marcar(id: number, on: boolean) {
  if (on) marcados.add(id);
  else marcados.delete(id);
  confirmando = false;
  renderList();
}

function limparSelecao() {
  marcados = new Set();
  confirmando = false;
  renderList();
}

function renderBulk(items: Alert[]) {
  const bar = $("bulk");
  const elegiveis = items.filter(podeMarcar);
  if (elegiveis.length === 0 && !avisoMassa) {
    bar.replaceChildren();
    bar.hidden = true;
    return;
  }
  bar.hidden = false;
  const estado = estadoTodos(items, marcados);
  const todos = h("input", {
    type: "checkbox",
    "aria-label": "Selecionar todos os abertos",
    checked: estado === "todos",
    disabled: !!emMassa || elegiveis.length === 0,
    onchange: (e) => {
      const on = (e.target as HTMLInputElement).checked;
      marcados = on ? new Set(elegiveis.map((a) => a.id)) : new Set();
      confirmando = false;
      renderList();
    },
  }) as HTMLInputElement;
  todos.indeterminate = estado === "alguns";
  const n = marcados.size;
  const itens: (Node | null)[] = [h("label", { class: "bulk-todos" }, todos, n > 0 ? `${n} selecionado${n === 1 ? "" : "s"}` : "Selecionar todos os abertos")];
  if (emMassa) {
    itens.push(h("span", { class: "bulk-prog", role: "status" }, `Ignorando ${emMassa.feitos} de ${emMassa.total}…`));
  } else if (n > 0) {
    itens.push(
      h(
        "button",
        {
          type: "button",
          class: confirmando ? "bulk-ignorar confirm" : "bulk-ignorar",
          onclick: () => void ignorarMarcados(),
        },
        confirmando ? `Confirmar: ignorar ${n}` : `Ignorar ${n}`,
      ),
      h("button", { type: "button", class: "link", onclick: limparSelecao }, "Limpar"),
    );
  }
  if (avisoMassa) itens.push(h("span", { class: avisoMassa.erro ? "bulk-msg err" : "bulk-msg", role: "status" }, avisoMassa.texto));
  bar.replaceChildren(...itens.filter((x): x is Node => x !== null));
}

/** Ignora os marcados um a um (o daemon trata cada alerta como no detalhe). Pede um segundo clique. */
async function ignorarMarcados() {
  if (emMassa || marcados.size === 0) return;
  if (!confirmando) {
    confirmando = true;
    clearTimeout(confirmTimer);
    confirmTimer = window.setTimeout(() => {
      confirmando = false;
      renderList();
    }, 4000);
    renderList();
    return;
  }
  clearTimeout(confirmTimer);
  confirmando = false;
  const alvo = alerts.filter((a) => marcados.has(a.id) && podeMarcar(a));
  emMassa = { feitos: 0, total: alvo.length };
  avisoMassa = null;
  renderList();
  const falhas: string[] = [];
  for (const a of alvo) {
    try {
      await api.act(a, "dismiss");
    } catch (e) {
      falhas.push(`${a.title}: ${e}`);
    }
    emMassa.feitos += 1;
    renderList();
  }
  const ok = alvo.length - falhas.length;
  emMassa = null;
  marcados = new Set();
  avisoMassa =
    falhas.length === 0
      ? { texto: `${ok} ${ok === 1 ? "alerta ignorado" : "alertas ignorados"}.`, erro: false }
      : { texto: `${ok} ignorados; ${falhas.length} não: ${falhas[0]}`, erro: true };
  window.setTimeout(() => {
    avisoMassa = null;
    renderList();
  }, 5000);
  renderList();
}

function renderList() {
  const list = $("alert-list");
  const items = visibleAlerts();
  // Alerta resolvido em outro lugar (pílula, celular) sai da seleção.
  if (!emMassa) marcados = podar(marcados, alerts);
  renderBulk(items);
  list.replaceChildren();
  if (items.length === 0) {
    list.append(
      h(
        "div",
        { class: "empty" },
        h("strong", {}, alerts.length === 0 ? "Nenhum alerta ainda" : "Nenhum alerta aberto"),
        alerts.length === 0
          ? "Quando algo suspeito acontecer, o alerta aparece no topo da tela e fica guardado aqui por 30 dias."
          : 'Desmarque "Só os abertos" para ver o histórico.',
      ),
    );
    return;
  }
  let day = "";
  for (const a of items) {
    const d = dayLabel(a.ts);
    if (d !== day) {
      day = d;
      list.append(h("div", { class: "day" }, d));
    }
    const linha = h(
      "button",
      {
        class: `row${a.status === "open" ? "" : " handled"}`,
        type: "button",
        "aria-current": a.id === selected ? "true" : "false",
        onclick: () => select(a.id),
      },
      h("span", { class: `lamp ${sevClass[a.severity]}`, "aria-hidden": "true" }),
      h("span", { class: "t", title: a.title }, a.count > 1 ? `${a.title} ×${a.count}` : a.title),
      a.simulated ? h("span", { class: "sim-tag" }, "Simulação") : null,
      h("span", { class: "when" }, time(a.ts).slice(0, 5)),
    );
    // A caixa fica fora do botão da linha (controle dentro de botão não é válido).
    const caixa = podeMarcar(a)
      ? h("input", {
          type: "checkbox",
          class: "sel-alerta",
          "aria-label": `Selecionar: ${a.title}`,
          checked: marcados.has(a.id),
          disabled: !!emMassa,
          onchange: (e) => marcar(a.id, (e.target as HTMLInputElement).checked),
        })
      : h("span", { class: "sel-alerta-vazia", "aria-hidden": "true" });
    list.append(h("div", { class: `row-wrap${marcados.has(a.id) ? " marcado" : ""}` }, caixa, linha));
  }
}

function select(id: number | null) {
  selected = id;
  renderList();
  renderDetail();
  if (id !== null) void api.markSeen([id]);
}

function upsert(a: Alert) {
  const i = alerts.findIndex((x) => x.id === a.id);
  if (i >= 0) alerts[i] = a;
  else alerts.unshift(a);
}

// ---------------------------------------------------------------------------
// Detalhe

function lineage(a: Alert): HTMLElement {
  // A cadeia vem do pai para cima; desenhamos do mais antigo até o processo do alerta.
  const chain: ProcInfo[] = [...(a.chain ?? [])].reverse();
  chain.push(a.process);
  const ol = h("ol", { class: "lineage", "aria-label": "Cadeia de processos, do mais antigo ao mais recente" });
  for (const p of chain) {
    const isCulprit = p === a.process;
    ol.append(
      h(
        "li",
        { class: isCulprit ? `culprit ${sevClass[a.severity]}` : "" },
        h("span", { class: "name" }, basename(p.path)),
        p.pid > 0 ? h("span", { class: "pid" }, `pid ${p.pid}`) : null,
        isCulprit ? h("span", { class: "role" }, "responsável") : null,
        h("span", { class: "path mono" }, shortPath(p.path)),
        p.cwd ? h("span", { class: "path mono" }, `rodando em ${shortPath(p.cwd)}`) : null,
      ),
    );
  }
  return ol;
}

function signature(p: ProcInfo): string | null {
  if (p.team_id) return `Assinado pelo desenvolvedor ${p.team_id}${p.signing_id ? ` (${p.signing_id})` : ""}`;
  if (p.signed === false) return "Sem assinatura de desenvolvedor";
  if (p.signing_id) return `Identificador ${p.signing_id}`;
  return null;
}

function renderDetail() {
  const box = $("detail");
  const a = alerts.find((x) => x.id === selected);
  box.replaceChildren();
  if (!a) {
    box.append(
      h(
        "div",
        { class: "empty" },
        h("strong", {}, "Escolha um alerta"),
        "O detalhe mostra o que aconteceu, a cadeia de processos que levou até ali e o que dá para fazer.",
      ),
    );
    return;
  }
  const date = new Date(ms(a.ts)).toLocaleString("pt-BR", { dateStyle: "long", timeStyle: "medium" });
  const note = h("p", { class: "note" });
  if (a.status_message) note.textContent = cap(a.status_message) + ".";
  if (a.status === "failed") note.className = "note err";

  // Alerta aberto: todas as ações. Resolvido: só as locais (Finder, Ajustes).
  // "Encerrar só este" só aparece quando não há como encerrar a árvore inteira.
  const available: ActionKind[] = (
    a.status === "open" ? a.actions : a.actions.filter((x) => x === "reveal" || x === "open_settings")
  ).filter((x) => !(x === "kill" && a.actions.includes("kill_tree")));
  const primary = available.find((x) => x === "kill_tree" || x === "quarantine" || x === "open_settings");
  const sig = signature(a.process);

  const parts: (Node | null)[] = [
    a.simulated
      ? h("p", { class: "sim-banner" }, "Simulação: este alerta vem da gravação de teste do scripts/dev.sh. Nada disso aconteceu no seu Mac, e os botões não executam nada.")
      : null,
    h("h1", {}, a.title),
    h(
      "div",
      { class: "facts" },
      h("span", { class: "sev" }, h("span", { class: `lamp ${sevClass[a.severity]}`, "aria-hidden": "true" }), `Severidade ${severityLabel[a.severity].toLowerCase()}`),
      h("span", {}, ruleLabel[a.rule] ?? a.rule),
      h("span", {}, date),
      a.count > 1 ? h("span", {}, `${a.count} vezes desde ${time(a.first_ts)}`) : null,
      a.status !== "open" ? h("span", {}, statusLabel[a.status]) : null,
    ),
    h("p", { class: "text" }, tilde(a.detail)),
    a.target ? h("h2", {}, a.rule === "new_connection" ? "Destino" : "Alvo") : null,
    a.target ? h("div", { class: "target mono" }, a.target) : null,
    h("h2", {}, "Linhagem"),
    lineage(a),
    sig ? h("p", { class: "note" }, sig) : null,
    h(
      "div",
      { class: "actions" },
      ...available.map((x) => actionButton(a, x, (act) => run(a, act, note), x === primary)),
      h("button", { type: "button", onclick: () => void askAbout(a.id) }, "Perguntar à IA"),
    ),
    available.includes("allow")
      ? h(
          "p",
          { class: "note" },
          "Permitir sempre cria uma exceção só para esta regra e este programa. As exceções ficam em /Library/Application Support/Vigia/allow.yaml.",
        )
      : null,
    note,
  ];
  box.append(...parts.filter((p): p is Node => p !== null));
}

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

async function run(a: Alert, action: ActionKind, note: HTMLElement) {
  try {
    const msg = await api.act(a, action);
    // O daemon devolve o alerta atualizado pelo evento; a mensagem fica de reforço.
    if (action !== "reveal" && action !== "open_settings") {
      note.className = "note";
      note.textContent = cap(msg) + ".";
    }
  } catch (e) {
    note.className = "note err";
    note.textContent = String(e);
  }
}

// ---------------------------------------------------------------------------
// Processos recentes

let timeline: ExecEntry[] = [];

function renderProcs() {
  const q = $<HTMLInputElement>("proc-filter").value.trim().toLowerCase();
  const hideApple = $<HTMLInputElement>("hide-apple").checked;
  const rows = timeline.filter((t) => {
    if (hideApple && t.apple) return false;
    if (!q) return true;
    return [t.path, t.args.join(" "), t.cwd ?? ""].some((s) => s.toLowerCase().includes(q));
  });
  const body = $("proc-rows");
  body.replaceChildren(
    ...rows.map((t) =>
      h(
        "tr",
        {},
        h("td", { class: "when" }, time(t.ts)),
        h("td", {}, h("span", { class: "name" }, basename(t.path)), h("span", { class: "args mono" }, t.args.slice(1).join(" ") || shortPath(t.path))),
        h("td", { class: "cwd mono" }, t.cwd ? shortPath(t.cwd) : ""),
        h("td", { class: "pid" }, String(t.pid)),
      ),
    ),
  );
  if (rows.length === 0) {
    body.append(
      h("tr", {}, h("td", { colspan: "4", class: "cwd" }, timeline.length ? "Nada corresponde ao filtro." : "Os processos iniciados aparecem aqui conforme o sensor os vê.")),
    );
  }
}

async function refreshProcs() {
  try {
    timeline = await api.timeline(500);
  } catch {
    timeline = [];
  }
  renderProcs();
}

// ---------------------------------------------------------------------------
// Barra lateral

export type SectionId = "inicio" | "alertas" | "atividade" | "conversa" | "pastas" | "excecoes" | "sons" | "celular" | "regras";

const NAV: { id: SectionId; label: string }[] = [
  { id: "inicio", label: "Início" },
  { id: "alertas", label: "Alertas" },
  { id: "atividade", label: "Atividade" },
  { id: "conversa", label: "Conversar" },
  { id: "pastas", label: "Pastas" },
  { id: "excecoes", label: "Exceções" },
  { id: "sons", label: "Sons e avisos" },
  // Celular desligado por ora (recursos.ts): fica fora da barra lateral.
  ...(CELULAR ? [{ id: "celular" as const, label: "Celular" }] : []),
  { id: "regras", label: "Regras" },
];
// Os itens depois deste ficam sob o rótulo CONFIGURAR.
const FIRST_CONFIG: SectionId = "pastas";

let current: SectionId = "inicio";

function renderNav() {
  const open = alerts.filter((a) => a.status === "open").length;
  const items: Node[] = [];
  for (const n of NAV) {
    if (n.id === FIRST_CONFIG) items.push(h("div", { class: "nav-label", role: "heading", "aria-level": "2" }, "Configurar"));
    items.push(
      h(
        "button",
        { type: "button", class: "nav-item", "aria-current": String(n.id === current), onclick: () => showSection(n.id) },
        h("span", {}, n.label),
        n.id === "alertas" && open > 0 ? h("span", { class: "count", "aria-label": `${open} abertos` }, String(open)) : null,
      ),
    );
  }
  $("nav").replaceChildren(...items);
}

/** Troca a seção visível e carrega o que ela precisa. */
export function showSection(id: SectionId) {
  current = id;
  for (const n of NAV) $(n.id).hidden = n.id !== id;
  renderNav();
  loadSection(id);
}

function loadSection(id: SectionId) {
  if (id === "inicio") void renderInicio($("inicio"), showSection);
  if (id === "atividade") void refreshProcs();
  if (id === "conversa")
    void renderConversa($("conversa"), { alertById: (n) => alerts.find((x) => x.id === n), mostrar: () => showSection("conversa") });
  if (id === "pastas" || id === "regras" || id === "celular") void loadAjustes();
  if (id === "excecoes") void renderExcecoes($("excecoes"));
  if (id === "sons") void renderSons($("sons"));
}

async function loadAlerts() {
  try {
    alerts = await api.history(500);
  } catch {
    alerts = [];
  }
  if (selected === null && alerts.length > 0) selected = alerts[0].id;
  renderList();
  renderDetail();
  renderNav();
}

async function main() {
  $("fox-slot").append(headerFox);
  setConversaCtx({ alertById: (n) => alerts.find((x) => x.id === n), mostrar: () => showSection("conversa") });
  renderPastas($("pastas"));
  renderRegras($("regras"));
  if (CELULAR) renderCelular($("celular"));
  showSection("inicio");
  await events.onOpenSettings(() => showSection("sons"));
  await events.onOpenConversa(() => showSection("conversa"));
  $("only-new").addEventListener("change", renderList);
  $("proc-filter").addEventListener("input", renderProcs);
  $("hide-apple").addEventListener("change", renderProcs);

  await events.onAlert((a) => {
    upsert(a);
    renderList();
    renderNav();
    if (a.id === selected) renderDetail();
    void refreshStatus();
  });
  await events.onDaemon((c) => {
    connected = c;
    if (c) {
      void loadAlerts();
      void refreshStatus();
    } else renderState();
  });
  await events.onFocusAlert(async (id) => {
    showSection("alertas");
    if (!alerts.some((a) => a.id === id)) await loadAlerts();
    select(id);
  });

  // Ao reabrir o painel, recarrega tudo.
  await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
    if (focused) {
      void loadAlerts();
      void refreshStatus();
      loadSection(current);
    }
  });

  connected = await api.connected();
  await Promise.all([loadAlerts(), refreshStatus()]);
  window.setInterval(() => {
    if (document.visibilityState === "visible") void refreshStatus();
  }, 5000);
}

void main();
