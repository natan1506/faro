import test from "node:test";
import assert from "node:assert/strict";
import { aplicarEscolha, lerOpcao, opcaoAtual, OUTRO, valorOpcao } from "./ia-escolha.ts";
import type { AppConfig } from "./api.ts";

const cfg = (ai: Partial<AppConfig["ai"]> = {}): AppConfig =>
  ({
    ai: { provider: "claude", model: "", codex_ack_at: null, claude_models: ["sonnet", "opus"], codex_models: ["gpt-5"], ...ai },
  }) as AppConfig;

test("lerOpcao divide só no primeiro |", () => {
  assert.deepEqual(lerOpcao("claude|meu|modelo"), { provider: "claude", model: "meu|modelo" });
  assert.deepEqual(lerOpcao(valorOpcao("codex", "")), { provider: "codex", model: "" });
  assert.deepEqual(lerOpcao(valorOpcao("codex", OUTRO)), { provider: "codex", model: OUTRO });
});

test("opcaoAtual aponta para Outro quando o modelo não está na lista", () => {
  assert.equal(opcaoAtual(cfg({ model: "sonnet" })), "claude|sonnet");
  assert.equal(opcaoAtual(cfg({ model: "x-1" })), `claude|${OUTRO}`);
  assert.equal(opcaoAtual(cfg({ provider: "codex", model: "" })), "codex|");
});

test("trocar do Claude para o Codex zera o aceite, inclusive via Outro", () => {
  const antes = cfg({ codex_ack_at: "2026-10-01T00:00:00Z" });
  const n = aplicarEscolha(antes, "codex", "modelo-digitado");
  assert.ok(n);
  assert.equal(n.ai.provider, "codex");
  assert.equal(n.ai.model, "modelo-digitado");
  assert.equal(n.ai.codex_ack_at, null);
  // o original não muda
  assert.equal(antes.ai.codex_ack_at, "2026-10-01T00:00:00Z");
});

test("Codex para Claude via Outro grava o Claude com o modelo digitado", () => {
  const n = aplicarEscolha(cfg({ provider: "codex", model: "gpt-5", codex_ack_at: "2026-10-01T00:00:00Z" }), "claude", "x-2");
  assert.ok(n);
  assert.equal(n.ai.provider, "claude");
  assert.equal(n.ai.model, "x-2");
});

test("ficar no Codex mantém o aceite; nada muda devolve null", () => {
  const c = cfg({ provider: "codex", model: "", codex_ack_at: "2026-10-01T00:00:00Z" });
  assert.equal(aplicarEscolha(c, "codex", "gpt-5")?.ai.codex_ack_at, "2026-10-01T00:00:00Z");
  assert.equal(aplicarEscolha(c, "codex", ""), null);
});
