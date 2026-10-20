import { test } from "node:test";
import assert from "node:assert/strict";
import type { Alert } from "./api.ts";
import { estadoTodos, podar, podeMarcar } from "./selecao.ts";

const alerta = (id: number, extra: Partial<Alert> = {}): Alert =>
  ({
    id,
    ts: "2026-10-20T12:00:00Z",
    first_ts: "2026-10-20T12:00:00Z",
    rule: "secret_read",
    severity: "critical",
    title: `alerta ${id}`,
    detail: "",
    process: { pid: 1, ppid: 0, path: "/bin/x" },
    actions: ["kill_tree", "dismiss"],
    count: 1,
    status: "open",
    ...extra,
  }) as Alert;

test("só abertos, reais e com Ignorar podem ser marcados", () => {
  assert.equal(podeMarcar(alerta(1)), true);
  assert.equal(podeMarcar(alerta(0)), false); // aviso local do app (daemon fora)
  assert.equal(podeMarcar(alerta(2, { status: "dismissed" })), false);
  assert.equal(podeMarcar(alerta(3, { actions: ["kill"] })), false);
});

test("estado de Selecionar todos", () => {
  const vis = [alerta(1), alerta(2), alerta(3, { status: "killed" })];
  assert.equal(estadoTodos(vis, new Set()), "nenhum");
  assert.equal(estadoTodos(vis, new Set([1])), "alguns");
  assert.equal(estadoTodos(vis, new Set([1, 2])), "todos"); // o 3 não é elegível
  assert.equal(estadoTodos([], new Set([9])), "nenhum");
});

test("podar tira o que foi resolvido ou sumiu", () => {
  const alertas = [alerta(1), alerta(2, { status: "dismissed" })];
  assert.deepEqual([...podar(new Set([1, 2, 7]), alertas)], [1]);
});
