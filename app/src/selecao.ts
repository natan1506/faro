// Seleção em massa na lista de alertas: o que pode ser marcado e o estado do
// "Selecionar todos". Lógica pura (sem DOM), testada em selecao.test.ts.
import type { Alert } from "./api";

/** Só alertas abertos, de verdade (id > 0) e que oferecem "Ignorar" entram na seleção. */
export function podeMarcar(a: Alert): boolean {
  return a.id > 0 && a.status === "open" && a.actions.includes("dismiss");
}

/** Estado da caixa "Selecionar todos" para os alertas visíveis. */
export function estadoTodos(visiveis: Alert[], marcados: ReadonlySet<number>): "todos" | "alguns" | "nenhum" {
  const elegiveis = visiveis.filter(podeMarcar);
  const n = elegiveis.filter((a) => marcados.has(a.id)).length;
  if (n === 0) return "nenhum";
  return n === elegiveis.length ? "todos" : "alguns";
}

/** Tira da seleção o que deixou de existir ou de estar aberto (resolvido em outro lugar). */
export function podar(marcados: ReadonlySet<number>, alertas: Alert[]): Set<number> {
  const ok = new Set(alertas.filter(podeMarcar).map((a) => a.id));
  return new Set([...marcados].filter((id) => ok.has(id)));
}
