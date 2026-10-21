// Simulação SEGURA de um ataque de cadeia de suprimentos, para testar o Faro de ponta a ponta.
//
// Não há nada malicioso aqui: ela repete os passos de um malware com alvos falsos e
// desfaz tudo no fim. Cada passo deveria gerar um alerta:
//   1. lê um .env de "outro projeto"            -> Leitura de segredo
//   2. node executa curl                         -> Comando suspeito
//   3. roda um binário sem assinatura do /tmp     -> Binário sem assinatura
//   4. registra um LaunchAgent                   -> Inicialização automática
// Depois fica viva por um tempo para você testar o botão "Encerrar processo".
//
// Uso: node simulacao.mjs <caminho-do-.env-falso> [segundos-de-espera]
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync, chmodSync } from "node:fs";
import { homedir, tmpdir, userInfo } from "node:os";
import { join } from "node:path";

const envPath = process.argv[2];
const wait = Number(process.argv[3] ?? 90);
if (!envPath) {
  console.error("uso: node simulacao.mjs <caminho-do-.env-falso> [segundos]");
  process.exit(2);
}

const LABEL = "dev.natan.vigia-simulacao";
const plist = join(homedir(), "Library", "LaunchAgents", `${LABEL}.plist`);
const scratch = mkdtempSync(join(tmpdir(), "vigia-sim-"));
const uid = userInfo().uid;
const pause = (s) => execFileSync("/bin/sleep", [String(s)]);

function cleanup() {
  try {
    execFileSync("/bin/launchctl", ["bootout", `gui/${uid}`, plist], { stdio: "ignore" });
  } catch {}
  if (existsSync(plist)) rmSync(plist, { force: true });
  rmSync(scratch, { recursive: true, force: true });
}
process.on("exit", cleanup);
for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"]) process.on(sig, () => process.exit(130));

function step(n, text, fn) {
  process.stdout.write(`${n}. ${text}... `);
  try {
    fn();
    console.log("feito");
  } catch (e) {
    console.log(`pulado (${e.message.split("\n")[0]})`);
  }
  pause(2);
}

console.log(`Simulação de ataque, pid ${process.pid}, rodando em ${process.cwd()}\n`);

step(1, `lendo ${envPath}`, () => {
  readFileSync(envPath, "utf8");
});

step(2, "executando curl para example.com", () => {
  execFileSync("/usr/bin/curl", ["-s", "-o", "/dev/null", "-m", "5", "https://example.com"]);
});

step(3, "rodando um binário sem assinatura em uma pasta temporária", () => {
  const bin = join(scratch, "agente-falso");
  copyFileSync("/usr/bin/true", bin);
  chmodSync(bin, 0o755);
  try {
    execFileSync("/usr/bin/codesign", ["--remove-signature", bin], { stdio: "ignore" });
    execFileSync(bin);
  } catch {
    // Apple Silicon não roda binário sem assinatura nenhuma; ad-hoc também conta como "sem assinatura de desenvolvedor".
    execFileSync("/usr/bin/codesign", ["-f", "-s", "-", bin], { stdio: "ignore" });
    execFileSync(bin);
  }
});

step(4, `registrando o LaunchAgent ${LABEL} (inofensivo: roda /usr/bin/true)`, () => {
  writeFileSync(
    plist,
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>${LABEL}</string>
  <key>ProgramArguments</key><array><string>/usr/bin/true</string></array>
  <key>RunAtLoad</key><false/>
</dict></plist>
`,
  );
  execFileSync("/bin/launchctl", ["bootstrap", `gui/${uid}`, plist], { stdio: "ignore" });
});

console.log(`\nPronto. Ficando viva por ${wait} s: use "Encerrar processo" no alerta para testar (pid ${process.pid}).`);
for (let i = 0; i < wait; i += 5) pause(5);
console.log("Tempo esgotado; desfazendo tudo.");
