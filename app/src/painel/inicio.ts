// Início: card de destaque (só texto), a configuração inicial (5 ou 6 passos, conforme o celular) e as proteções.
import { api, events, h, ms, type Alert, type AppConfig, type Settings, type Status } from "../api";
import { CELULAR } from "../recursos";
import { headline, steps, type StepId } from "../checklist";
import type { SectionId } from "../painel";

const COFRE_TEXTO =
  "Hoje cada dev guarda os .env no disco, e foi de lá que o incidente de 16/10 tirou as credenciais. A saída é um cofre de secrets (a proposta que foi para o time): as chaves ficam fora do disco e entram só na hora de rodar. Quando terminar, marque como feito.";

let root: HTMLElement;
let ir: (id: SectionId) => void = () => {};
let montado = false;
let connected = false;
let status: Status | null = null;
let settings: Settings | null = null;
let config: AppConfig | null = null;
let alerts: Alert[] = [];
let aberto: StepId | null = null; // passo com o texto de ajuda expandido
let forcarLista = false; // "ver configuração inicial" com os 6 feitos
let aviso: { text: string; error: boolean } | null = null;
let copiado = false;

let pedido = 0; // contador para ignorar respostas fora de ordem
let ultimo = ""; // o que foi desenhado por último, para não refazer o DOM à toa

async function carregar() {
  const meu = ++pedido;
  const con = await api.connected().catch(() => false);
  const [s, st, c, a] = await Promise.all([
    api.status().catch(() => null),
    api.getSettings().catch(() => null),
    api.getAppConfig().catch(() => null),
    api.history(200).catch(() => [] as Alert[]),
  ]);
  if (meu !== pedido) return; // chegou outro pedido depois deste
  connected = con;
  status = connected ? s : null;
  settings = connected ? st : null;
  config = c ?? config;
  alerts = a;
  desenhar();
}

async function salvarChecklist(chave: "reviewed_normal" | "cofre_done") {
  if (!config) return;
  const novo: AppConfig = JSON.parse(JSON.stringify(config));
  novo.checklist[chave] = true;
  try {
    config = await api.saveAppConfig(novo);
    aviso = null;
  } catch (e) {
    aviso = { text: `Não foi possível salvar: ${e}`, error: true };
  }
  desenhar();
}

async function escolherPasta() {
  try {
    const p = await api.pickFolder();
    if (!p) return;
    const atual = await api.getSettings();
    if (!atual.third_party_dirs.includes(p)) atual.third_party_dirs.push(p);
    settings = await api.saveSettings(atual);
    aviso = null;
  } catch (e) {
    aviso = { text: String(e), error: true };
  }
  desenhar();
}

async function abrirUrl(url: string) {
  try {
    await api.openUrl(url);
    aviso = null;
  } catch (e) {
    aviso = { text: String(e), error: true };
  }
  desenhar();
}

async function copiar(texto: string) {
  try {
    await navigator.clipboard.writeText(texto);
    copiado = true;
    aviso = null;
    window.setTimeout(() => ((copiado = false), desenhar()), 2000);
  } catch {
    aviso = { text: "Não consegui copiar. Selecione o comando e copie com Cmd+C.", error: true };
  }
  desenhar();
}

function ajuda(id: StepId): HTMLElement | null {
  if (aberto !== id) return null;
  if (id === "daemon") {
    return h(
      "div",
      { class: "passo-ajuda" },
      h("p", {}, "Na pasta do projeto, rode no terminal (pede a senha do Mac):"),
      h("div", { class: "cmd" }, h("code", {}, "./scripts/instalar.sh"), h("button", { type: "button", class: "add", onclick: () => void copiar("./scripts/instalar.sh") }, copiado ? "Copiado" : "Copiar")),
    );
  }
  if (id === "cofre") {
    const url = config?.cofre_url ?? "";
    return h(
      "div",
      { class: "passo-ajuda" },
      h("p", {}, COFRE_TEXTO),
      url ? h("button", { type: "button", class: "add", onclick: () => void abrirUrl(url) }, "Abrir a proposta") : null,
    );
  }
  return null;
}

function botao(id: StepId, texto: string): HTMLElement {
  const acao: Record<StepId, () => void> = {
    daemon: () => ((aberto = aberto === "daemon" ? null : "daemon"), desenhar()),
    fda: () => void abrirUrl("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"),
    terceiros: () => void escolherPasta(),
    celular: () => ir("celular"),
    normal: () => ir("atividade"),
    cofre: () => ((aberto = aberto === "cofre" ? null : "cofre"), desenhar()),
  };
  return h("button", { type: "button", class: "add", onclick: acao[id] }, texto);
}

function destaque(): HTMLElement {
  const t = headline({ connected, status, alerts });
  const sim = status?.sensor.state === "replay" && connected;
  return h(
    "div",
    { class: `destaque${!connected ? " parado" : sim ? " sim" : ""}` },
    h("h1", {}, t.title),
    h("p", {}, t.line),
    t.button ? h("button", { type: "button", class: "destaque-btn", onclick: () => ir("alertas") }, t.button) : null,
  );
}

function checklist(): HTMLElement | null {
  if (!config) return null;
  const lista = steps({ connected, status, settings, config });
  const feitos = lista.filter((s) => s.done).length;
  if (feitos === lista.length && !forcarLista) {
    return h("p", { class: "ver-config" }, h("button", { type: "button", class: "link", onclick: () => ((forcarLista = true), desenhar()) }, "ver configuração inicial"));
  }
  return h(
    "section",
    { class: "group checklist" },
    h("h2", {}, "O que falta para ficar protegido"),
    h("div", { class: "progresso" }, h("span", {}, `${feitos} de ${lista.length}`), h("div", { class: "barra", role: "progressbar", "aria-valuemin": "0", "aria-valuemax": String(lista.length), "aria-valuenow": String(feitos) }, h("div", { style: `width:${(feitos / lista.length) * 100}%` }))),
    h(
      "ul",
      { class: "passos" },
      ...lista.map((s) =>
        h(
          "li",
          { class: s.done ? "feito" : "" },
          h(
            "div",
            { class: "passo" },
            h("span", { class: `marca${s.done ? " ok" : ""}`, "aria-hidden": "true" }, s.done ? "✓" : ""),
            h("span", { class: "passo-titulo" }, s.title, h("span", { class: "sr" }, s.done ? " (feito)" : " (pendente)")),
            s.done
              ? null
              : h(
                  "span",
                  { class: "passo-acoes" },
                  botao(s.id, s.action),
                  s.id === "normal" || s.id === "cofre"
                    ? h("button", { type: "button", class: "link", onclick: () => void salvarChecklist(s.id === "normal" ? "reviewed_normal" : "cofre_done") }, "Marcar como feito")
                    : null,
                ),
          ),
          s.done ? null : ajuda(s.id),
        ),
      ),
    ),
    feitos === lista.length ? h("p", { class: "ver-config" }, h("button", { type: "button", class: "link", onclick: () => ((forcarLista = false), desenhar()) }, "ocultar")) : null,
  );
}

type Tom = "calm" | "alta" | "critica";
function protecao(nome: string, tom: Tom, detalhe: string): HTMLElement {
  return h("div", { class: "protecao" }, h("span", { class: `lamp ${tom}`, "aria-hidden": "true" }), h("div", {}, h("b", {}, nome), h("small", {}, detalhe)));
}

function protecoes(): HTMLElement {
  const s = status;
  let sensor: HTMLElement;
  let rede: HTMLElement;
  let celular: HTMLElement;
  if (!connected) {
    sensor = protecao("Sensor do kernel", "critica", "Sem resposta do daemon");
    rede = protecao("Rede", "critica", "Sem resposta do daemon");
    celular = protecao("Celular", "critica", "Sem resposta do daemon");
  } else {
    const estado = s?.sensor.state;
    const porMin = Math.round((s?.events_per_sec ?? 0) * 60).toLocaleString("pt-BR");
    sensor =
      estado === "running" ? protecao("Sensor do kernel", "calm", `${porMin} eventos/min`)
      : estado === "replay" ? protecao("Sensor do kernel", "alta", "Simulação (gravação de teste)")
      : estado === "starting" ? protecao("Sensor do kernel", "alta", "Iniciando…")
      : protecao("Sensor do kernel", "critica", "Parado. Falta o Acesso Total ao Disco?");
    const ligada = settings?.rules.new_connection !== false;
    const aprende = s?.learning_until && ms(s.learning_until) > Date.now();
    rede = !ligada
      ? protecao("Rede", "alta", "Desligada")
      : aprende
        ? protecao("Rede", "alta", `Aprendendo até ${new Date(ms(s!.learning_until!)).toLocaleString("pt-BR", { weekday: "short", hour: "2-digit", minute: "2-digit" })}`)
        : protecao("Rede", "calm", "Ligada");
    celular = settings?.ntfy_enabled ? protecao("Celular", "calm", "Ligado") : protecao("Celular", "alta", "Desligado");
  }
  return h("section", { class: "group" }, h("h2", {}, "Proteções"), h("div", { class: "protecoes" }, sensor, rede, CELULAR ? celular : null));
}

function desenhar() {
  if (root.hidden) return; // Início fechado: refaz quando for mostrado (showSection chama renderInicio)
  const chave = JSON.stringify([connected, status, settings, config, alerts, aberto, forcarLista, aviso, copiado]);
  if (chave === ultimo) return;
  ultimo = chave;
  const rolagem = root.scrollTop;
  root.replaceChildren(
    h(
      "div",
      { class: "inicio" },
      destaque(),
      aviso ? h("p", { class: aviso.error ? "msg err" : "msg ok", role: "alert" }, aviso.text) : null,
      checklist(),
      protecoes(),
    ),
  );
  root.scrollTop = rolagem;
}

/** `irPara` troca de seção no painel (passado por painel.ts para evitar import circular). */
export async function renderInicio(el: HTMLElement, irPara?: (id: SectionId) => void) {
  root = el;
  if (irPara) ir = irPara;
  if (!montado) {
    montado = true;
    const recarrega = () => {
      if (!root.hidden) void carregar();
    };
    try {
      await Promise.all([
        events.onAppConfig((c) => ((config = c), desenhar())),
        events.onAlert(recarrega),
        events.onDaemon(recarrega),
        // O status vem pronto no evento: sem nova consulta.
        events.onStatus((s) => {
          if (connected) status = s;
          desenhar();
        }),
      ]);
    } catch (e) {
      console.error("não consegui ouvir os eventos do Início", e);
    }
  }
  await carregar();
}
