// Sons e avisos: o som que toca quando chega alerta e o volume. (O celular tem tela própria.)
import { api, basename, h, events, type AppConfig } from "../api";
import { section } from "../ajustes";

const ARQUIVO = "__arquivo__";

let cfg: AppConfig | null = null;
let sons: string[] = [];
let aviso: { text: string; error: boolean } | null = null;
let root: HTMLElement;
let montado = false;

/** Nome para mostrar: o arquivo sem a extensão. */
const nome = (p: string) => basename(p).replace(/\.[^.]+$/, "");

async function salvar(fn: (c: AppConfig) => void) {
  if (!cfg) return;
  const novo: AppConfig = JSON.parse(JSON.stringify(cfg));
  fn(novo);
  try {
    cfg = await api.saveAppConfig(novo);
    aviso = null;
  } catch (e) {
    aviso = { text: `Não foi possível salvar: ${e}`, error: true };
  }
  desenhar();
}

async function ouvir(path: string) {
  if (!cfg) return;
  try {
    await api.previewSound(path, cfg.sound.volume);
    aviso = null;
  } catch (e) {
    aviso = { text: `Não consegui tocar ${nome(path)}: ${e}`, error: true };
  }
  desenhar();
}

function seletor(rotulo: string, chave: "high" | "medium"): HTMLElement {
  const atual = cfg!.sound[chave];
  const opcoes = sons.includes(atual) ? sons : [atual, ...sons];
  const select = h(
    "select",
    {
      "aria-label": rotulo,
      disabled: !cfg!.sound.enabled,
      onchange: async (e) => {
        const el = e.target as HTMLSelectElement;
        if (el.value !== ARQUIVO) return void salvar((c) => (c.sound[chave] = el.value));
        let p: string | null = null;
        try {
          p = await api.pickSoundFile();
        } catch (err) {
          aviso = { text: `Não consegui abrir o seletor de arquivo: ${err}`, error: true };
        }
        if (p) void salvar((c) => (c.sound[chave] = p));
        else desenhar(); // cancelou ou falhou: volta para o som anterior
      },
    },
    ...opcoes.map((p) => h("option", { value: p, selected: p === atual }, nome(p))),
    h("option", { value: ARQUIVO }, "Arquivo…"),
  );
  return h(
    "div",
    { class: "som-linha" },
    h("span", {}, rotulo),
    select,
    h("button", { type: "button", class: "add", disabled: !cfg!.sound.enabled, onclick: () => void ouvir(atual) }, "Ouvir"),
  );
}

function desenhar() {
  if (!cfg) {
    root.replaceChildren(
      h(
        "div",
        { class: "ajustes-scroll" },
        h(
          "div",
          { class: "ajustes" },
          section("Som dos alertas", "", h("p", { class: aviso?.error ? "msg err" : "hint" }, aviso ? aviso.text : "Carregando…")),
        ),
      ),
    );
    return;
  }
  const c = cfg;
  const rolagem = root.querySelector(".ajustes-scroll")?.scrollTop ?? 0;
  const volume = h("input", {
    type: "range",
    min: "0",
    max: "1",
    step: "0.05",
    value: String(c.sound.volume),
    disabled: !c.sound.enabled,
    "aria-label": "Volume",
    // Grava só ao soltar; enquanto arrasta, mostra o valor.
    oninput: (e) => (porcento.textContent = `${Math.round(Number((e.target as HTMLInputElement).value) * 100)}%`),
    onchange: (e) => void salvar((x) => (x.sound.volume = Number((e.target as HTMLInputElement).value))),
  });
  const porcento = h("span", { class: "pct" }, `${Math.round(c.sound.volume * 100)}%`);
  const conteudo = h(
    "div",
    { class: "ajustes-scroll" },
    h(
      "div",
      { class: "ajustes" },
      section(
        "Som dos alertas",
        "Toca quando chega um alerta novo, mesmo com o painel fechado.",
        h(
          "label",
          { class: "rule" },
          h("input", {
            type: "checkbox",
            checked: c.sound.enabled,
            onchange: (e) => void salvar((x) => (x.sound.enabled = (e.target as HTMLInputElement).checked)),
          }),
          h("span", {}, h("b", {}, "Tocar um som quando chegar alerta")),
        ),
        h("div", { class: "som-linha" }, h("span", {}, "Volume"), volume, porcento),
        seletor("Crítica e alta", "high"),
        seletor("Média", "medium"),
        aviso ? h("p", { class: aviso.error ? "msg err" : "msg ok", role: "alert" }, aviso.text) : null,
      ),
    ),
  );
  root.replaceChildren(conteudo);
  conteudo.scrollTop = rolagem;
}

export async function renderSons(el: HTMLElement) {
  root = el;
  if (!montado) {
    montado = true;
    // O Rust avisa quando o app.json muda (por exemplo, por outra janela).
    try {
      await events.onAppConfig((c) => {
        cfg = c;
        desenhar();
      });
    } catch (e) {
      console.error("não consegui ouvir vigia:app-config", e);
    }
  }
  try {
    [cfg, sons] = await Promise.all([api.getAppConfig(), api.listSystemSounds()]);
    aviso = null;
  } catch (e) {
    aviso = { text: `Não foi possível ler as preferências: ${e}`, error: true };
  }
  desenhar();
}
