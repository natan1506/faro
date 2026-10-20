// Exceções em vigor (só leitura): o que o allow.yaml do daemon deixa passar.
import { api, h, ruleLabel, shortPath, type AllowView } from "../api";
import { section } from "../ajustes";

const programa = (x: AllowView): string =>
  x.signing_id ? `${x.signing_id}${x.team_id ? ` · ${x.team_id}` : ""}` : x.path_prefix ? shortPath(x.path_prefix) : "";

export async function renderExcecoes(el: HTMLElement) {
  let lista: AllowView[] = [];
  let erro: string | null = null;
  try {
    lista = await api.allowlist();
  } catch (e) {
    erro = String(e);
  }
  const corpo = erro
    ? h("p", { class: "msg err", role: "alert" }, `Não foi possível ler as exceções: ${erro}`)
    : lista.length === 0
      ? h("p", { class: "empty-note" }, "Nenhuma exceção. O Faro não deixa passar nada por conta própria.")
      : h(
          "table",
          { class: "excecoes" },
          h("thead", {}, h("tr", {}, h("th", {}, "Regra"), h("th", {}, "Programa"), h("th", {}, "Destino"), h("th", {}, "Comentário"))),
          h(
            "tbody",
            {},
            ...lista.map((x) =>
              h(
                "tr",
                {},
                h("td", {}, ruleLabel[x.rule] ?? x.rule),
                h("td", { class: "mono" }, programa(x)),
                h("td", { class: "mono" }, x.target ?? ""),
                h("td", {}, x.comment ?? ""),
              ),
            ),
          ),
        );
  el.replaceChildren(
    h(
      "div",
      { class: "ajustes-scroll" },
      h(
        "div",
        { class: "ajustes wide" },
        section(
          "Exceções",
          "Para criar uma exceção, use Permitir no alerta. Para remover, edite /Library/Application Support/Vigia/allow.yaml.",
          corpo,
        ),
      ),
    ),
  );
}
