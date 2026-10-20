// Pastas, Regras e Celular do painel: o que antes
// só dava para mudar editando o config.yaml com sudo. O app manda para o daemon,
// que confere, grava em ajustes.yaml e passa a valer na hora (sem reiniciar nada).
// Cada tela tem rascunho e Salvar próprios e só grava os seus campos.
import { api, h, ruleLabel, type Settings } from "./api";

const RULES: { id: string; hint: string }[] = [
  { id: "secret_read", hint: ".env, ~/.aws, ~/.ssh, cookies e senhas do navegador lidos fora do próprio projeto." },
  { id: "third_party_shell", hint: "node/npm chamando curl, osascript, security… ou um shell numa pasta de terceiros." },
  { id: "unsigned_exec", hint: "Programa sem assinatura rodando de /tmp, Downloads ou Library." },
  { id: "persistence", hint: "LaunchAgent, LaunchDaemon ou item de login novo (o backdoor do incidente)." },
  { id: "tcc_change", hint: "App novo com Monitoramento de Entrada, Acessibilidade ou gravação de tela." },
  { id: "new_connection", hint: "node, python, curl… falando com uma rede que nunca usaram." },
];

/** Tópico difícil de adivinhar: no ntfy.sh, quem souber o nome lê os alertas. */
function randomTopic(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(12));
  return "vigia-" + Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

const clone = <T>(s: T): T => JSON.parse(JSON.stringify(s));

/**
 * Uma tela de ajustes com rascunho, mensagem e Salvar próprios. Ela só mexe nos
 * campos de `keys`: ao salvar, parte dos ajustes que estão no daemon agora e troca
 * só esses campos, então edições não salvas de outra tela nunca vazam.
 */
class Pane {
  private saved: Settings | null = null;
  private draft: Settings | null = null;
  private message: { text: string; error: boolean } | null = null;
  private loadError: string | null = null;
  private saving = false;

  constructor(
    private el: HTMLElement,
    private bare: boolean,
    private keys: (keyof Settings)[],
    private body: (d: Settings, p: Pane) => (Node | null)[],
  ) {}

  private pick = (s: Settings) => JSON.stringify(this.keys.map((k) => s[k]));
  private dirty = () => !!this.saved && !!this.draft && this.pick(this.saved) !== this.pick(this.draft);

  /** Traz o que está no daemon. Mudanças não salvas desta tela são mantidas. */
  async load() {
    if (this.dirty()) return;
    try {
      this.saved = await api.getSettings();
      this.draft = clone(this.saved);
      this.loadError = null;
    } catch (e) {
      this.loadError = String(e);
    }
    this.render();
  }

  update(fn: (d: Settings) => void) {
    if (!this.draft) return;
    fn(this.draft);
    this.message = null;
    this.render();
  }

  async addFolder(list: "third_party_dirs" | "watch_dirs") {
    let p: string | null = null;
    try {
      p = await api.pickFolder();
    } catch (e) {
      this.message = { text: `Não consegui abrir o seletor de pasta: ${e}`, error: true };
    }
    if (!p) return this.render(); // cancelou: desfaz o rádio marcado
    this.update((d) => {
      if (!d[list].includes(p)) d[list].push(p);
    });
  }

  private async save() {
    if (!this.draft || this.saving) return;
    this.saving = true;
    this.render();
    try {
      const atual = await api.getSettings();
      for (const k of this.keys) (atual as unknown as Record<string, unknown>)[k] = clone(this.draft[k]);
      this.saved = await api.saveSettings(atual);
      this.draft = clone(this.saved);
      this.message = { text: "Salvo. Já está valendo.", error: false };
    } catch (e) {
      this.message = { text: String(e), error: true };
    }
    this.saving = false;
    this.render();
  }

  private discard() {
    if (!this.saved) return;
    this.draft = clone(this.saved);
    this.message = null;
    this.render();
  }

  folderList(list: "third_party_dirs" | "watch_dirs", empty: string): HTMLElement {
    const d = this.draft!;
    return h(
      "div",
      { class: "folders" },
      d[list].length === 0 ? h("p", { class: "empty" }, empty) : null,
      ...d[list].map((dir) =>
        h(
          "div",
          { class: "folder" },
          h("span", { class: "mono", title: dir }, dir),
          h(
            "button",
            { type: "button", class: "link", onclick: () => this.update((x) => (x[list] = x[list].filter((y) => y !== dir))) },
            "Remover",
          ),
        ),
      ),
      h("button", { type: "button", class: "add", onclick: () => void this.addFolder(list) }, "Adicionar pasta…"),
    );
  }

  render() {
    const root = this.el;
    if (!this.draft) {
      root.replaceChildren(
        h(
          "div",
          { class: "ajustes-empty" },
          this.loadError
            ? h("p", {}, `Não foi possível ler os ajustes: ${this.loadError}. Os ajustes ficam no daemon; confira se ele está rodando.`)
            : h("p", {}, "Carregando…"),
        ),
      );
      return;
    }
    const groups = this.body(this.draft, this);
    const content = this.bare
      ? h("div", { class: "ajustes-bare" }, ...groups)
      : h("div", { class: "ajustes-scroll" }, h("div", { class: "ajustes" }, ...groups));
    const m = this.message;
    const footer = h(
      "footer",
      { class: "ajustes-footer" },
      m ? h("p", { class: m.error ? "msg err" : "msg ok", role: m.error ? "alert" : "status" }, m.text) : h("span"),
      this.dirty() ? h("button", { type: "button", class: "link", onclick: () => this.discard() }, "Descartar") : null,
      h("button", { type: "button", class: "primary", disabled: !this.dirty() || this.saving, onclick: () => void this.save() }, this.saving ? "Salvando…" : "Salvar"),
    );
    // Preserva a rolagem ao redesenhar.
    const scroll = this.bare ? 0 : (root.querySelector(".ajustes-scroll")?.scrollTop ?? 0);
    root.replaceChildren(content, footer);
    content.scrollTop = scroll;
  }
}

const panes: Pane[] = [];

function mount(el: HTMLElement, bare: boolean, keys: (keyof Settings)[], body: (d: Settings, p: Pane) => (Node | null)[]) {
  const p = new Pane(el, bare, keys, body);
  panes.push(p);
  p.render();
}

/** Busca os ajustes no daemon para todas as telas montadas. */
export async function loadAjustes() {
  await Promise.all(panes.map((p) => p.load()));
}

export function section(title: string, hint: string, ...body: (Node | null)[]): HTMLElement {
  return h("section", { class: "group" }, h("h2", {}, title), h("p", { class: "hint" }, hint), ...body);
}

/** Onde o Vigia olha: pastas de terceiros e pastas monitoradas. */
export function renderPastas(el: HTMLElement) {
  mount(el, false, ["third_party_dirs", "watch_dirs"], (d, p) => {
    const watchAll = d.watch_dirs.length === 0;
    const watchOnly = h("input", {
      type: "radio",
      name: "watch",
      checked: !watchAll,
      onchange: () => void p.addFolder("watch_dirs"),
    });
    return [
      section(
        "Pastas de terceiros",
        "Onde você clona código em que não confia (projeto baixado, exemplo de tutorial, repositório de outra pessoa). Lá dentro, node/npm abrindo um shell já vira alerta, e nenhum .env conta como \"do próprio projeto\".",
        p.folderList("third_party_dirs", "Nenhuma pasta. Recomendo ter pelo menos uma e clonar código de terceiros sempre nela."),
      ),
      section(
        "Pastas monitoradas",
        "Onde a leitura de arquivos .env é vigiada.",
        h(
          "div",
          { class: "choices" },
          h(
            "label",
            {},
            h("input", { type: "radio", name: "watch", checked: watchAll, onchange: () => p.update((x) => (x.watch_dirs = [])) }),
            h("span", {}, h("b", {}, "O Mac inteiro"), " (recomendado)"),
          ),
          h("label", {}, watchOnly, h("span", {}, h("b", {}, "Só estas pastas"))),
        ),
        watchAll ? null : p.folderList("watch_dirs", "Nenhuma pasta: escolha pelo menos uma, ou volte para o Mac inteiro."),
        h(
          "p",
          { class: "hint small" },
          "Mesmo restringindo, continuam valendo no Mac inteiro: credenciais (~/.aws, ~/.ssh, cookies), qualquer leitura feita por código rodando numa pasta de terceiros, backdoors de inicialização, binários sem assinatura e permissões de privacidade.",
        ),
      ),
    ];
  });
}

/** Liga e desliga cada regra. */
export function renderRegras(el: HTMLElement) {
  mount(el, false, ["rules"], (d, p) => [
    section(
      "Regras",
      "Desligar uma regra para de gerar alertas dela imediatamente.",
      h(
        "div",
        { class: "rules" },
        ...RULES.map((r) =>
          h(
            "label",
            { class: "rule" },
            h("input", {
              type: "checkbox",
              checked: d.rules[r.id] !== false,
              onchange: (e) => p.update((x) => (x.rules[r.id] = (e.target as HTMLInputElement).checked)),
            }),
            h("span", {}, h("b", {}, ruleLabel[r.id] ?? r.id), h("small", {}, r.hint)),
          ),
        ),
      ),
    ),
  ]);
}

/** Alertas críticos no celular (ntfy): tela própria, Celular, na barra lateral. */
export function renderCelular(el: HTMLElement) {
  mount(el, false, ["ntfy_enabled", "ntfy_topic"], (d, p) => [
    section(
      "Alertas no celular",
      "Alertas críticos chegam no celular pelo app gratuito ntfy. Instale, toque em + e assine o tópico abaixo.",
      h(
        "label",
        { class: "rule" },
        h("input", {
          type: "checkbox",
          checked: d.ntfy_enabled,
          onchange: (e) =>
            p.update((x) => {
              x.ntfy_enabled = (e.target as HTMLInputElement).checked;
              if (x.ntfy_enabled && x.ntfy_topic.length < 16) x.ntfy_topic = randomTopic();
            }),
        }),
        h("span", {}, h("b", {}, "Mandar alertas críticos para o celular")),
      ),
      h(
        "div",
        { class: "topic" },
        h("input", {
          type: "text",
          class: "mono",
          value: d.ntfy_topic,
          placeholder: "tópico do ntfy",
          "aria-label": "Tópico do ntfy",
          spellcheck: "false",
          onchange: (e) => p.update((x) => (x.ntfy_topic = (e.target as HTMLInputElement).value.trim())),
        }),
        h("button", { type: "button", class: "link", onclick: () => p.update((x) => (x.ntfy_topic = randomTopic())) }, "Gerar outro"),
      ),
    ),
  ]);
}
