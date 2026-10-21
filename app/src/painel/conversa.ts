// Conversar: perguntas à IA (Claude Code ou Codex) sobre o que o Vigia vê.
// Monta uma vez, escuta os eventos e redesenha só o que mudou.
import { api, events, h, type Alert, type AppConfig, type ChatMessage, type SuggestedAction } from "../api";
import { codexWarning, contextNote, messageView, providerPicker, streamingView } from "../conversa-ui";

const PERGUNTA_ALERTA = "O que aconteceu aqui e o que devo fazer?";

export interface ConversaCtx {
  /** Alerta pelo id (da lista do painel); undefined se sumiu. */
  alertById: (id: number) => Alert | undefined;
  /** Mostra a seção Conversar. */
  mostrar: () => void;
}

let ctx: ConversaCtx;

/** Define o contexto no início do painel, para askAbout funcionar sem ter aberto Conversar. */
export function setConversaCtx(c: ConversaCtx) {
  ctx = c;
}
let montado = false;
let root: HTMLElement;
let cfg: AppConfig | null = null;
let providers: string[] = [];
let mensagens: ChatMessage[] = [];
let ocupado = false;
let parcial = "";
let foco: number | null = null;
let aviso: string | null = null;

// Pedaços da tela
let topo: HTMLElement;
let lista: HTMLElement;
let chipFoco: HTMLElement;
let areaAviso: HTMLElement;
let entrada: HTMLTextAreaElement;
let enviar: HTMLButtonElement;
let parar: HTMLButtonElement;
let etiquetaCodex: HTMLElement;
let msgAviso: HTMLElement;
let stream: ReturnType<typeof streamingView> | null = null;

const bloqueado = () => !!cfg && cfg.ai.provider === "codex" && !cfg.ai.codex_ack_at;

async function salvarCfg(novo: AppConfig) {
  try {
    cfg = await api.saveAppConfig(novo);
    aviso = null;
  } catch (e) {
    aviso = `Não foi possível salvar: ${e}`;
  }
  desenharTudo();
}

/** Botão de sugestão da IA: acha o alerta (lista do painel ou histórico) e executa. */
async function aoAgir(s: SuggestedAction) {
  let a = ctx.alertById(s.alert_id);
  // A lista do painel pode não ter o alerta (filtro, limite): procura no histórico, como a pílula.
  if (!a) a = (await api.history(500).catch(() => [] as Alert[])).find((x) => x.id === s.alert_id);
  if (!a || a.status !== "open" || !a.actions.includes(s.action)) {
    aviso = "Esse alerta já foi resolvido.";
    desenharAviso();
    return;
  }
  try {
    const m = await api.act(a, s.action);
    aviso = m ? m.charAt(0).toUpperCase() + m.slice(1) + "." : null;
  } catch (e) {
    aviso = String(e);
  }
  desenharAviso();
}

// ---------------------------------------------------------------------------
// Desenho

function desenharTopo() {
  if (!cfg) return;
  // Não refaz o seletor enquanto o usuário digita o nome de um modelo.
  if (document.activeElement instanceof HTMLInputElement && topo.contains(document.activeElement)) return;
  topo.replaceChildren(
    providers.length > 0 ? providerPicker(cfg, providers, (n) => void salvarCfg(n)) : h("span", {}),
    h(
      "button",
      { type: "button", class: "link", disabled: mensagens.length === 0 && !ocupado, onclick: () => void api.aiClear() },
      "Limpar conversa",
    ),
  );
}

function desenharLista() {
  const perto = lista.scrollHeight - lista.scrollTop - lista.clientHeight < 80;
  const itens: Node[] = [];
  if (providers.length === 0) {
    itens.push(
      h(
        "div",
        { class: "empty" },
        h("strong", {}, "Nenhuma IA encontrada"),
        "Para conversar, instale o Claude Code (claude) ou o Codex (codex) e clique em Procurar de novo. O Faro usa o que já está instalado no seu Mac, sem chave nova.",
        h("button", { type: "button", class: "link", onclick: () => void carregar(true) }, "Procurar de novo"),
      ),
    );
  } else if (mensagens.length === 0 && !ocupado) {
    itens.push(
      h(
        "div",
        { class: "empty" },
        h("strong", {}, "Pergunte sobre o que o Faro está vendo"),
        'Por exemplo: "o que é esse processo?", "esse alerta é perigoso?" ou "o que eu faço agora?". A IA explica e recomenda; quem age é você.',
      ),
    );
  }
  for (const m of mensagens) itens.push(messageView(m, (x) => void aoAgir(x)));
  if (ocupado) {
    stream = streamingView();
    stream.set(parcial);
    itens.push(stream.el);
  } else stream = null;
  lista.replaceChildren(...itens);
  if (perto || ocupado) lista.scrollTop = lista.scrollHeight;
}

function desenharAviso() {
  msgAviso.textContent = aviso ?? "";
  msgAviso.hidden = !aviso;
}

function desenharCompositor() {
  const sem = providers.length === 0;
  const bloq = bloqueado();
  entrada.disabled = sem || bloq;
  entrada.placeholder = sem ? "Nenhuma IA instalada" : bloq ? "Aceite o aviso do Codex para perguntar" : "Pergunte algo…";
  enviar.disabled = sem || bloq || ocupado;
  parar.hidden = !ocupado;
  etiquetaCodex.hidden = !(cfg?.ai.provider === "codex" && !!cfg.ai.codex_ack_at);

  areaAviso.replaceChildren();
  if (bloq && !sem) {
    areaAviso.append(
      codexWarning(
        async () => {
          try {
            cfg = await api.aiAcceptCodex();
            aviso = null;
          } catch (e) {
            aviso = `Não foi possível salvar: ${e}`;
          }
          desenharTudo();
        },
        () => {
          if (cfg) {
            const novo: AppConfig = JSON.parse(JSON.stringify(cfg));
            novo.ai.provider = "claude";
            novo.ai.model = "";
            void salvarCfg(novo);
          }
        },
        providers.includes("claude"),
      ),
    );
  }

  chipFoco.replaceChildren();
  chipFoco.hidden = foco === null;
  if (foco !== null) {
    const a = ctx.alertById(foco);
    chipFoco.append(
      h("span", {}, a ? `Sobre o alerta: ${a.title}` : `Sobre o alerta #${foco}`),
      h("button", { type: "button", class: "link", "aria-label": "Tirar o alerta da conversa", onclick: () => { foco = null; desenharCompositor(); } }, "×"),
    );
  }
}

function desenharTudo() {
  desenharTopo();
  desenharLista();
  desenharAviso();
  desenharCompositor();
}

// ---------------------------------------------------------------------------
// Envio

async function perguntar(texto: string) {
  const q = texto.trim();
  if (!q || ocupado || bloqueado() || providers.length === 0) return;
  entrada.value = "";
  mensagens.push({ role: "voce", text: q, provider: null, model: null, actions: [] });
  ocupado = true;
  parcial = "";
  aviso = null;
  desenharTudo();
  try {
    await api.aiAsk(q, foco);
  } catch (e) {
    const texto = String(e);
    if (/espere/i.test(texto)) {
      // Já há uma resposta em andamento (por exemplo, iniciada pela pílula): continua ocupado.
      await recarregarConversa();
      aviso = texto;
    } else {
      ocupado = false;
      mensagens.push({ role: "erro", text: texto, provider: null, model: null, actions: [] });
    }
    desenharTudo();
  }
}

/** Relê a conversa; false se não conseguiu. */
async function recarregarConversa(): Promise<boolean> {
  try {
    mensagens = await api.aiConversation();
    return true;
  } catch {
    return false; // mantém o que já tem na tela
  }
}

/** Abre Conversar com o alerta no contexto e a primeira pergunta pronta no
 * campo, sem enviar: o Natan revisa, edita e manda (desenho §9). */
export async function askAbout(alertId: number) {
  ctx.mostrar();
  await carregar();
  foco = alertId;
  entrada.value = PERGUNTA_ALERTA;
  desenharTudo();
  entrada.focus();
  entrada.setSelectionRange(entrada.value.length, entrada.value.length);
}

// ---------------------------------------------------------------------------

/** `procurar`: pede ao app para procurar os CLIs de novo (senão vale a procura guardada). */
async function carregar(procurar = false) {
  const [c, p, conv] = await Promise.all([
    api.getAppConfig().catch(() => cfg),
    api.aiProviders(procurar).catch(() => [] as string[]),
    api.aiConversation().catch(() => mensagens),
  ]);
  cfg = c;
  providers = p;
  if (!ocupado) mensagens = conv;
  // A IA escolhida pode ter sido desinstalada: cai para a primeira que existir.
  if (cfg && providers.length > 0 && !providers.includes(cfg.ai.provider)) {
    const novo: AppConfig = JSON.parse(JSON.stringify(cfg));
    novo.ai.provider = providers[0] as "claude" | "codex";
    novo.ai.model = "";
    novo.ai.codex_ack_at = null;
    await salvarCfg(novo);
    return;
  }
  desenharTudo();
}

export async function renderConversa(el: HTMLElement, c?: ConversaCtx) {
  if (c) ctx = c;
  if (!montado) {
    montado = true;
    root = el;
    topo = h("div", { class: "conversa-topo" });
    // Saiu do seletor (ou do campo "Outro…"): redesenha com o config atual, que
    // pode ter mudado enquanto o campo estava focado.
    topo.addEventListener("focusout", () => window.setTimeout(desenharTopo, 0));
    lista = h("div", { class: "conversa-lista", role: "log" });
    chipFoco = h("div", { class: "chip-foco" });
    areaAviso = h("div", {});
    msgAviso = h("p", { class: "note", hidden: true, "aria-live": "polite" });
    entrada = h("textarea", { rows: "2", "aria-label": "Pergunta", class: "conversa-entrada" }) as HTMLTextAreaElement;
    entrada.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
        e.preventDefault();
        void perguntar(entrada.value);
      }
    });
    enviar = h("button", { type: "button", class: "primario", onclick: () => void perguntar(entrada.value) }, "Enviar") as HTMLButtonElement;
    parar = h("button", { type: "button", onclick: () => void api.aiStop() }, "Parar") as HTMLButtonElement;
    etiquetaCodex = h("span", { class: "tag-codex", title: "O Codex pode abrir arquivos do Mac" }, "Codex: pode ler arquivos");
    root.replaceChildren(
      h(
        "div",
        { class: "conversa" },
        topo,
        contextNote(),
        lista,
        areaAviso,
        msgAviso,
        chipFoco,
        h("div", { class: "conversa-compor" }, entrada, h("div", { class: "compor-lado" }, etiquetaCodex, parar, enviar)),
      ),
    );

    await events.onAiChunk((t) => {
      if (!ocupado) {
        ocupado = true; // pergunta feita pela pílula
        parcial = "";
        void recarregarConversa().then(desenharTudo);
      }
      parcial += t;
      if (stream) {
        stream.set(parcial);
        lista.scrollTop = lista.scrollHeight;
      }
    });
    await events.onAiDone(async (m) => {
      ocupado = false;
      parcial = "";
      const ok = await recarregarConversa();
      // Se a releitura falhou ou não trouxe a mensagem final, usa a do evento (sem duplicar).
      const ultima = mensagens[mensagens.length - 1];
      if (!ok || !ultima || ultima.role !== m.role || ultima.text !== m.text) mensagens.push(m);
      desenharTudo();
    });
    await events.onAiCleared(() => {
      ocupado = false;
      parcial = "";
      mensagens = [];
      foco = null;
      aviso = null;
      desenharTudo();
    });
    await events.onAppConfig((n) => {
      cfg = n;
      desenharTudo();
    });
  }
  await carregar();
}
