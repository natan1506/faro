// Peças da conversa com a IA, usadas pelo painel (seção Conversar) e pela pílula.
// Não importa painel.ts: quem usa passa o que precisa.
import "./conversa-ui.css";
import { actionLabel, h, needsConfirm, type AppConfig, type ChatMessage, type SuggestedAction } from "./api";
import { aplicarEscolha, lerOpcao, modeloEmOutro, opcaoAtual, OUTRO, valorOpcao } from "./ia-escolha.ts";

const NOME: Record<string, string> = { claude: "Claude", codex: "Codex" };
const PADRAO: Record<string, string> = { claude: "Padrão do Claude Code", codex: "Padrão do Codex" };

export const nomeIA = (p: string | null | undefined) => (p ? (NOME[p] ?? p) : "IA");

/** Resposta da IA, fala do usuário ou erro. Sugestões viram botões (sempre com segundo clique). */
export function messageView(m: ChatMessage, onAct: (s: SuggestedAction) => void): HTMLElement {
  const box = h("div", { class: `msg ${m.role}` });
  if (m.role === "ia") {
    box.append(h("div", { class: "msg-tag" }, `${nomeIA(m.provider)} · ${m.model || "padrão"}`));
  }
  box.append(h("div", { class: "msg-text" }, m.text));
  if (m.role === "ia" && m.actions.length > 0) {
    box.append(h("div", { class: "sugestoes" }, ...m.actions.map((s) => suggestionButton(s, onAct))));
  }
  return box;
}

/**
 * Botão de uma ação sugerida pela IA. A IA lê texto escrito pelo processo
 * suspeito e pode ter sido induzida: toda sugestão leva a marca "sugerido pela IA"
 * e pede um segundo clique, qualquer que seja a ação (o Rust já descarta `allow`).
 */
function suggestionButton(s: SuggestedAction, onAct: (s: SuggestedAction) => void): HTMLButtonElement {
  const texto = s.label || actionLabel[s.action];
  const rotulo = h("span", { class: "sug-texto" }, texto);
  const btn = h(
    "button",
    {
      type: "button",
      class: needsConfirm(s.action) ? "sugestao perigo" : "sugestao",
      title: "Sugerido pela IA: confira o alerta antes. Pede um segundo clique.",
    },
    rotulo,
    h("span", { class: "sug-marca" }, "sugerido pela IA"),
  );
  let timer: number | undefined;
  const desarmar = () => {
    btn.classList.remove("confirm");
    rotulo.textContent = texto;
  };
  btn.addEventListener("click", () => {
    if (!btn.classList.contains("confirm")) {
      btn.classList.add("confirm");
      rotulo.textContent = `Confirmar: ${texto}`;
      timer = window.setTimeout(desarmar, 4000);
      return;
    }
    clearTimeout(timer);
    desarmar();
    onAct(s);
  });
  return btn;
}

/** Resposta em andamento: o texto parcial cresce com `set`. */
export function streamingView(): { el: HTMLElement; set: (text: string) => void } {
  const texto = h("div", { class: "msg-text" });
  const el = h("div", { class: "msg ia streaming", "aria-live": "polite" }, texto);
  const set = (t: string) => {
    texto.textContent = t;
    el.classList.toggle("vazio", t === "");
  };
  set("");
  return { el, set };
}

/** Cor de cada IA no badge: a predominante da marca (Claude laranja, Codex azul). */
export const COR_IA: Record<string, string> = { claude: "#D97757", codex: "#4C8BF5" };

/** Opções de quem usa o seletor. */
export interface PickerOpts {
  /** Chamado antes de focar o campo "Outro…" (a pílula precisa virar focável). */
  antesDeFocar?: () => Promise<void>;
  /** O campo "Outro…" apareceu ou sumiu (a pílula reajusta a área clicável). */
  aoMudarTamanho?: () => void;
}

/**
 * Escolha de IA e de modelo num badge com a cor da IA (borda sólida, fundo a 50%).
 * Um <select> nativo invisível cobre o badge: o clique abre o menu do macOS,
 * com os modelos agrupados por IA. "Outro…" mostra um campo para o nome; a troca
 * (IA e modelo juntos) só é gravada quando o nome é confirmado.
 * Trocar de Claude para Codex zera o aceite do aviso, por qualquer caminho.
 */
export function providerPicker(
  cfg: AppConfig,
  providers: string[],
  onChange: (cfg: AppConfig) => void,
  opts: PickerOpts = {},
): HTMLElement {
  const ai = cfg.ai;
  const modelosDe = (p: string) => (p === "codex" ? ai.codex_models : ai.claude_models);
  const atual = opcaoAtual(cfg);
  const emOutro = modeloEmOutro(cfg);
  const descricao = `IA: ${nomeIA(ai.provider)}, ${ai.model ? `modelo ${ai.model}` : "modelo padrão"}`;

  const sel = h("select", { class: "ia-select", "aria-label": descricao, title: descricao }) as HTMLSelectElement;
  for (const p of providers) {
    const grupo = h("optgroup", { label: p === "claude" ? "Claude Code" : nomeIA(p) });
    grupo.append(h("option", { value: valorOpcao(p, "") }, PADRAO[p] ?? "Padrão"));
    for (const m of modelosDe(p)) grupo.append(h("option", { value: valorOpcao(p, m) }, m));
    grupo.append(h("option", { value: valorOpcao(p, OUTRO) }, "Outro…"));
    sel.append(grupo);
  }
  sel.value = atual;

  // O texto do badge é só visual: o leitor de tela lê o aria-label do <select>.
  const badge = h(
    "span",
    { class: "ia-badge", "data-ia": ai.provider, style: `--ia: ${COR_IA[ai.provider] ?? "#8e8e93"}` },
    h("span", { class: "ia-dot", "aria-hidden": "true" }),
    h("span", { class: "ia-nome", "aria-hidden": "true" }, nomeIA(ai.provider)),
    h("span", { class: "ia-modelo", "aria-hidden": "true" }, ai.model || "padrão"),
    h("span", { class: "ia-seta", "aria-hidden": "true" }, "⌄"),
    sel,
  );

  const campo = h("input", {
    type: "text",
    class: "modelo-outro",
    placeholder: "nome do modelo",
    "aria-label": "Nome do modelo",
    spellcheck: "false",
  }) as HTMLInputElement;
  campo.value = emOutro ? ai.model : "";
  campo.hidden = !emOutro;
  const mostrarCampo = (v: boolean) => {
    if (campo.hidden === !v) return;
    campo.hidden = !v;
    opts.aoMudarTamanho?.();
  };

  /** IA do "Outro…" em edição: a escolhida no menu (pode não ser a atual). */
  const iaDoCampo = () => {
    const o = lerOpcao(sel.value);
    return o.model === OUTRO ? o.provider : ai.provider;
  };
  let enviado = false;
  const confirmar = () => {
    if (enviado) return;
    const p = iaDoCampo();
    const v = campo.value.trim();
    // Nome vazio numa IA diferente não troca nada: fica como desistência.
    if (v === "" && p !== ai.provider) return;
    const novo = aplicarEscolha(cfg, p, v);
    if (!novo) return;
    enviado = true;
    onChange(novo);
  };
  campo.addEventListener("change", confirmar);
  campo.addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key !== "Enter") return;
    confirmar();
    // Confirmado: solta o campo para quem usa redesenhar o seletor com o valor novo.
    if (enviado) campo.blur();
  });
  // Saiu do campo sem confirmar: o menu volta para o valor atual.
  campo.addEventListener("blur", () => {
    if (enviado) return;
    sel.value = atual;
    campo.value = emOutro ? ai.model : "";
    mostrarCampo(emOutro);
  });

  sel.addEventListener("change", () => {
    const { provider: p, model: m } = lerOpcao(sel.value);
    if (m === OUTRO) {
      // Não grava ainda: a IA e o modelo vão juntos quando o nome for confirmado.
      if (p !== ai.provider) campo.value = "";
      mostrarCampo(true);
      const focar = () => {
        campo.focus();
        campo.select();
      };
      if (opts.antesDeFocar) void opts.antesDeFocar().then(focar);
      else focar();
      return;
    }
    mostrarCampo(false);
    const novo = aplicarEscolha(cfg, p, m);
    if (novo) onChange(novo);
  });

  return h("div", { class: "picker" }, badge, campo);
}

/** Aviso que bloqueia o envio enquanto o Codex não for aceito. */
export function codexWarning(onAccept: () => void, onUseClaude: () => void, claudeDisponivel = true): HTMLElement {
  return h(
    "div",
    { class: "codex-aviso", role: "alert" },
    h(
      "p",
      {},
      "O Codex consegue abrir qualquer arquivo do seu Mac, inclusive ~/.aws, ~/.ssh e os .env, e mandar o conteúdo para a OpenAI para responder. O Claude Code roda sem acesso a arquivos. Usar o Codex mesmo assim?",
    ),
    h(
      "div",
      { class: "linha-botoes" },
      h(
        "button",
        {
          type: "button",
          class: "primario",
          disabled: !claudeDisponivel,
          title: claudeDisponivel ? "" : "O Claude Code não está instalado",
          onclick: onUseClaude,
        },
        "Usar o Claude",
      ),
      h("button", { type: "button", onclick: onAccept }, "Entendi, usar o Codex"),
    ),
  );
}

/** Bloco recolhível com o que entra (e o que nunca entra) no contexto de cada pergunta.
 * Espelha `app/src-tauri/src/ai/context.rs` e `build_prompt` (ai/mod.rs): manter em sincronia. */
export function contextNote(): HTMLElement {
  return h(
    "details",
    { class: "contexto" },
    h("summary", {}, "O que vai junto"),
    h("p", {}, "A cada pergunta, o Faro manda para a IA:"),
    h(
      "ul",
      {},
      h("li", {}, "a versão do Faro, o estado do sensor, quantos alertas estão abertos e a hora atual;"),
      h("li", {}, "as pastas de código de terceiros e as pastas vigiadas para .env;"),
      h(
        "li",
        {},
        "até 40 alertas (os abertos e os das últimas 24 h): número, horário, regra, severidade, título, detalhe, alvo, quantas vezes repetiu, se é simulação, ações disponíveis e situação;",
      ),
      h("li", {}, "de cada alerta, o processo: caminho, pid, assinatura, argumentos, pasta onde rodava e o caminho dos processos pais;"),
      h("li", {}, "o alerta escolhido, se você começou a conversa por um alerta;"),
      h("li", {}, "as últimas mensagens desta conversa e a sua pergunta."),
    ),
    h("p", {}, "Nunca vai:"),
    h(
      "ul",
      {},
      h("li", {}, "conteúdo de arquivo nem valores de variáveis;"),
      h("li", {}, "o tópico do ntfy;"),
      h(
        "li",
        {},
        "o que parecer segredo nos alertas e no que você escreve (tokens, senhas, chaves, cabeçalhos de autenticação): é trocado por [removido] antes de sair do Mac.",
      ),
    ),
  );
}
