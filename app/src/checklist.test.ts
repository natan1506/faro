import test from "node:test";
import assert from "node:assert/strict";
import { headline, steps } from "./checklist.ts";
import type { Alert, AppConfig, Settings, Status } from "./api.ts";

const status = (state: string, extra: Partial<Status> = {}): Status =>
  ({ version: "0", started_at: "", sensor: { state } as Status["sensor"], events_total: 0, events_dropped: 0, events_per_sec: 12.4, alerts_open: 0, ...extra });
const settings = (extra: Partial<Settings> = {}): Settings =>
  ({ third_party_dirs: [], watch_dirs: [], rules: {}, ntfy_enabled: false, ntfy_topic: "", ...extra });
const config = (c: Partial<AppConfig["checklist"]> = {}) =>
  ({ checklist: { reviewed_normal: false, ...c } }) as AppConfig;
const alerta = (id: number, severity: Alert["severity"], ts: string, title = `a${id}`, st: Alert["status"] = "open"): Alert =>
  ({ id, severity, ts, title, status: st }) as Alert;

test("sem daemon, só os passos manuais podem estar feitos", () => {
  const s = steps({ connected: false, status: status("running"), settings: settings({ third_party_dirs: ["~/x"], ntfy_enabled: true }), config: config({ reviewed_normal: true }), celular: true });
  assert.deepEqual(s.map((x) => x.id), ["daemon", "fda", "terceiros", "celular", "normal"]);
  assert.deepEqual(s.map((x) => x.done), [false, false, false, false, true]);
});

test("sensor running marca o acesso ao disco", () => {
  const i = { connected: true, settings: settings(), config: config(), celular: true };
  assert.equal(steps({ ...i, status: status("running") })[1].done, true);
  assert.equal(steps({ ...i, status: status("down") })[1].done, false);
  assert.equal(steps({ ...i, status: null })[1].done, false);
});

test("pasta de terceiros e celular vêm dos ajustes", () => {
  const s = steps({ connected: true, status: status("running"), settings: settings({ third_party_dirs: ["~/t"], ntfy_enabled: true }), config: config(), celular: true });
  assert.equal(s[2].done, true);
  assert.equal(s[3].done, true);
  const v = steps({ connected: true, status: status("running"), settings: settings(), config: config(), celular: true });
  assert.equal(v[2].done, false);
  assert.equal(v[3].done, false);
});

test("o passo manual vem do app.json", () => {
  const i = { connected: true, status: null, settings: null, celular: true };
  assert.equal(steps({ ...i, config: config({ reviewed_normal: true }) })[4].done, true);
  assert.equal(steps({ ...i, config: config() })[4].done, false);
  assert.equal(steps({ ...i, config: config() })[0].title, "Instalar o Faro");
});

test("headline com 1 alerta usa o singular", () => {
  const now = Date.parse("2026-10-19T12:00:00Z");
  const h = headline({ connected: true, status: status("running"), alerts: [alerta(1, "critical", "2026-10-19T11:55:00Z", "node leu .env")] }, now);
  assert.equal(h.title, "1 alerta pede sua decisão");
  assert.equal(h.line, "O mais grave: node leu .env, há 5 min.");
  assert.equal(h.button, "Ver alertas");
});

test("headline em replay diz Simulação", () => {
  const h = headline({ connected: true, status: status("replay"), alerts: [] });
  assert.equal(h.title, "Simulação");
  assert.equal(h.button, "Ver alertas");
});

test("headline sem alertas diz Tudo calmo por aqui", () => {
  const h = headline({ connected: true, status: status("running"), alerts: [alerta(1, "high", "2026-10-19T11:55:00Z", "x", "dismissed")] });
  assert.equal(h.title, "Tudo calmo por aqui");
  assert.match(h.line, /12 eventos por segundo analisados agora\./);
  assert.equal(h.button, null);
  const baixo = headline({ connected: true, status: status("running", { events_per_sec: 3.46 }), alerts: [] });
  assert.match(baixo.line, /^3,5 eventos por segundo/);
  const off = headline({ connected: false, status: null, alerts: [] });
  assert.equal(off.title, "O Faro parou de vigiar");
  assert.match(off.line, /sudo launchctl print system\/dev\.natan\.vigia/);
  assert.equal(off.button, null);
});

test("o mais grave é o de maior severidade, depois o mais recente", () => {
  const now = Date.parse("2026-10-19T12:00:00Z");
  const h = headline({ connected: true, status: status("running"), alerts: [
    alerta(1, "high", "2026-10-19T11:59:00Z", "alta recente"),
    alerta(2, "critical", "2026-10-19T10:00:00Z", "crítica velha"),
    alerta(3, "critical", "2026-10-19T11:00:00Z", "crítica nova"),
  ] }, now);
  assert.equal(h.title, "3 alertas pedem sua decisão");
  assert.equal(h.line, "O mais grave: crítica nova, há 1 h.");
});

test("com o celular desligado, o passo some e sobram 4", () => {
  const s = steps({ connected: true, status: status("running"), settings: settings({ ntfy_enabled: true }), config: config(), celular: false });
  assert.deepEqual(s.map((x) => x.id), ["daemon", "fda", "terceiros", "normal"]);
});
