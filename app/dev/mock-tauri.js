// Tauri de mentira para conferir o painel no navegador (http://localhost:1420/?mock=1).
// Só é carregado com ?mock=1 (veja o script de index.html); não vai para o build.
const agora = Date.now();
const iso = (minAtras) => new Date(agora - minAtras * 60_000).toISOString();
const proc = (pid, ppid, path, extra = {}) => ({ pid, ppid, path, args: [path], signed: false, ...extra });

const alertas = [
  {
    id: 3, ts: iso(2), first_ts: iso(2), rule: "secret_read", severity: "critical", count: 1, status: "open",
    title: "node leu /Users/dev/empresa/.env", detail: "Um processo numa pasta de terceiros leu um segredo.",
    target: "/Users/dev/empresa/.env",
    process: proc(9120, 9100, "/usr/local/bin/node", { cwd: "/Users/dev/terceiros/teste", signing_id: "node", team_id: "ABCDE12345", signed: true }),
    chain: [proc(9100, 1, "/bin/zsh")], actions: ["kill_tree", "quarantine", "allow", "dismiss"],
  },
  {
    id: 2, ts: iso(30), first_ts: iso(40), rule: "persistence", severity: "critical", count: 2, status: "open", simulated: true,
    title: "Novo item de inicialização: com.tailwind.helper.plist", detail: "Um LaunchAgent novo foi registrado.",
    target: "/Users/dev/Library/LaunchAgents/com.tailwind.helper.plist",
    process: proc(80005, 1, "/private/tmp/.tw-cache/twhelper"), chain: [], actions: ["kill_tree", "quarantine", "allow", "dismiss"],
  },
  {
    id: 1, ts: iso(60 * 26), first_ts: iso(60 * 26), rule: "new_connection", severity: "medium", count: 1, status: "dismissed",
    title: "curl falou com 203.0.113.0/24", detail: "Rede nova.", target: "203.0.113.7:443",
    process: proc(7000, 1, "/usr/bin/curl", { signed: true }), chain: [], actions: ["allow", "dismiss"],
  },
];

const dados = {
  history: alertas,
  timeline: [
    { ts: iso(1), pid: 9120, ppid: 9100, path: "/usr/local/bin/node", args: ["node", "server.js"], cwd: "/Users/dev/terceiros/teste", apple: false },
    { ts: iso(3), pid: 9101, ppid: 1, path: "/usr/bin/git", args: ["git", "status"], cwd: "/Users/dev/www", apple: true },
  ],
  status: {
    version: "0.1.0", started_at: iso(600), sensor: { state: "running" }, events_total: 123456,
    events_dropped: 0, events_per_sec: 42.4, alerts_open: 2,
  },
  daemon_connected: true,
  get_settings: {
    third_party_dirs: ["/Users/dev/terceiros"], watch_dirs: [],
    rules: { new_connection: false }, ntfy_enabled: true, ntfy_topic: "vigia-0123456789abcdef0123",
  },
  get_app_config: {
    sound: { enabled: true, volume: 0.6, high: "/System/Library/Sounds/Sosumi.aiff", medium: "/System/Library/Sounds/Tink.aiff" },
    auto_close_secs: 15,
    ai: { provider: "claude", model: "", codex_ack_at: null, claude_models: [], codex_models: [] },
    checklist: { reviewed_normal: false, cofre_done: false }, cofre_url: "",
  },
  list_system_sounds: ["Basso", "Blow", "Glass", "Sosumi", "Tink"].map((n) => `/System/Library/Sounds/${n}.aiff`),
  allowlist: [
    { rule: "unsigned_exec", signing_id: "com.docker.docker", team_id: "9BNSXJN65R", path_prefix: null, target: null, comment: "Docker Desktop" },
    { rule: "secret_read", signing_id: null, team_id: null, path_prefix: "/Users/dev/www/vigia/target/debug/app", target: "~/.aws/credentials", comment: null },
  ],
};

const comandos = {
  history: () => dados.history,
  timeline: () => dados.timeline,
  status: () => { if (comandosFora) throw "sem daemon"; return dados.status; },
  daemon_connected: () => dados.daemon_connected,
  open_url: ({ url }) => console.log("[mock] open_url", url),
  daemon_embutido: () => true,
  instalar_daemon: () => new Promise((ok) => setTimeout(ok, 800)),
  revelar_daemon: () => null,
  servico_desatualizado: () => new URLSearchParams(location.search).get("servico") === "velho",
  get_settings: () => { if (comandosFora) throw "sem daemon"; return dados.get_settings; },
  save_settings: ({ settings }) => ((dados.get_settings = settings), settings),
  get_app_config: () => dados.get_app_config,
  save_app_config: ({ config }) => ((dados.get_app_config = config), config),
  list_system_sounds: () => dados.list_system_sounds,
  preview_sound: () => null,
  pick_sound_file: () => "/Users/dev/Music/meu-som.mp3",
  pick_folder: () => "/Users/dev/outra-pasta",
  allowlist: () => dados.allowlist,
  mark_seen: () => null,
  notch_metrics: () => ({ menubar: 24, has_notch: new URLSearchParams(location.search).has("notch") }),
  notch_hit_area: ({ width, height }) => { window.__hit = { width, height }; return null; },
  notch_focusable: async ({ on }) => { if (new URLSearchParams(location.search).has("lento")) await new Promise((r) => setTimeout(r, 300)); window.__focusable = on; (window.__focusLog ??= []).push(on); return null; },
  open_painel: () => console.log("[mock] open_painel"),
  open_settings: () => console.log("[mock] open_settings"),
  open_conversa: () => console.log("[mock] open_conversa"),
  act: (a) => { console.log("[mock] act", JSON.stringify(a)); return "simulação: nada foi executado"; },
};

// ?estado=0|3|6|replay|fora (sem parâmetro: 4 de 6) escolhe o cenário do Início.
const estado = new URLSearchParams(location.search).get("estado");
if (estado === "0") {
  dados.get_settings.third_party_dirs = []; dados.get_settings.ntfy_enabled = false; dados.status.sensor = { state: "down", reason: "sem Acesso Total ao Disco" };
} else if (estado === "3") {
  dados.get_settings.ntfy_enabled = false;
} else if (estado === "6") {
  dados.history.forEach((a) => (a.status = "dismissed")); dados.status.alerts_open = 0;
  dados.get_app_config.checklist = { reviewed_normal: true, cofre_done: true };
} else if (estado === "replay") {
  dados.status.sensor = { state: "replay" };
} else if (estado === "fora") {
  dados.daemon_connected = false;
}
const comandosFora = estado === "fora";

let proximo = 1;
// Eventos: guarda os callbacks de listen() e deixa emitir (window.__emitir) para simular o backend.
const callbacks = new Map();
const ouvintes = new Map(); // evento -> [handler]
function emitir(evento, payload) {
  for (const id of ouvintes.get(evento) ?? []) callbacks.get(id)?.({ event: evento, id, payload });
}
window.__emitir = emitir;

// IA de mentira. ?ia=claude (padrão) | ambos | codex | nenhum | erro | acked
const ia = new URLSearchParams(location.search).get("ia") ?? "claude";
const provedores = { claude: ["claude"], ambos: ["claude", "codex"], codex: ["codex", "claude"], acked: ["claude", "codex"], nenhum: [], erro: ["claude"] }[ia] ?? ["claude"];
if (ia === "ambos" || ia === "codex" || ia === "acked") {
  dados.get_app_config.ai.claude_models = ["sonnet", "opus"];
  dados.get_app_config.ai.codex_models = ["gpt-5-codex"];
}
if (ia === "codex") dados.get_app_config.ai.provider = "codex";
if (ia === "acked") { dados.get_app_config.ai.provider = "codex"; dados.get_app_config.ai.codex_ack_at = new Date().toISOString(); }
let conversa = [];
let respondendo = null;
const dormir = (ms) => new Promise((r) => setTimeout(r, ms));
async function responder(pergunta, foco) {
  const prov = dados.get_app_config.ai.provider;
  const modelo = dados.get_app_config.ai.model || null;
  const alvo = foco ?? 3;
  const texto = foco !== null && alertas.find((a) => a.id === foco)?.simulated
    ? "Isso é uma simulação: nada disso aconteceu no seu Mac. Em um caso real, eu encerraria o processo."
    : "Sim, é perigoso. Um `node` rodando em ~/terceiros/teste leu o .env de outro projeto (~/empresa/.env): código de terceiros não tem por que ler segredos fora do próprio repositório. É o mesmo padrão de um ataque de cadeia de suprimentos.\n\nO que fazer agora:\n1. Encerre o processo e os filhos (botão abaixo).\n2. Troque as chaves que estavam nesse .env.\n3. Confira os outros alertas desse pid antes de apagar o repositório.";
  const palavras = texto.split(" ");
  const meu = (respondendo = {});
  for (let i = 0; i < palavras.length; i += 3) {
    await dormir(250);
    if (respondendo !== meu) return;
    emitir("vigia:ai-chunk", { text: palavras.slice(i, i + 3).join(" ") + " " });
  }
  await dormir(200);
  if (respondendo !== meu) return;
  respondendo = null;
  const msg = ia === "erro"
    ? { role: "erro", text: "O claude não respondeu: sessão expirada. Rode `claude` no terminal e entre de novo.", provider: null, model: null, actions: [] }
    : { role: "ia", text: texto, provider: prov, model: modelo, actions: [{ alert_id: alvo, action: "kill_tree", label: "Encerrar processo e filhos" }] };
  conversa.push(msg);
  emitir("vigia:ai-done", msg);
}
Object.assign(comandos, {
  ai_providers: () => provedores,
  ai_conversation: () => conversa,
  ai_ask: ({ question, focusAlert }) => {
    conversa.push({ role: "voce", text: question, provider: null, model: null, actions: [] });
    void responder(question, focusAlert);
  },
  ai_stop: () => {
    if (!respondendo) return;
    respondendo = null;
    const msg = { role: "erro", text: "Resposta interrompida.", provider: null, model: null, actions: [] };
    conversa.push(msg);
    emitir("vigia:ai-done", msg);
  },
  ai_clear: () => { respondendo = null; conversa = []; emitir("vigia:ai-cleared", null); },
  ai_accept_codex: () => ((dados.get_app_config.ai.codex_ack_at = new Date().toISOString()), dados.get_app_config),
});

window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: "painel" }, currentWebview: { windowLabel: "painel", label: "painel" } },
  transformCallback: (cb) => { const id = proximo++; callbacks.set(id, cb); return id; },
  unregisterCallback() {},
  convertFileSrc: (p) => p,
  invoke: async (cmd, args) => {
    if (cmd === "plugin:event|listen") {
      const lista = ouvintes.get(args.event) ?? [];
      lista.push(args.handler);
      ouvintes.set(args.event, lista);
      return args.handler;
    }
    if (cmd.startsWith("plugin:event|")) return null;
    if (cmd.startsWith("plugin:")) return null;
    const fn = comandos[cmd];
    if (!fn) {
      console.warn(`[mock] comando sem resposta: ${cmd}`);
      return null;
    }
    return structuredClone(await fn(args));
  },
};
