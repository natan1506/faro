#!/usr/bin/env bash
# Instala o Vigia: compila o daemon e o app, instala o LaunchDaemon e abre o app.
#
#   ./scripts/instalar.sh              # tudo
#   ./scripts/instalar.sh --sem-app    # só o daemon (dá para usar `faro-daemon tail`)
#
# Para quem compila pelo código. Quem baixa a Release não precisa disto: o
# Faro.app instala o serviço sozinho (botão "Instalar" da tela Início).
#
# Pede a senha de administrador (sudo) para copiar o daemon e o LaunchDaemon.
set -euo pipefail

LABEL="dev.natan.vigia"
BIN_DIR="/usr/local/faro"
BIN="$BIN_DIR/faro-daemon"
DATA_DIR="/Library/Application Support/Vigia"
LOG_DIR="/Library/Logs/Vigia"
PLIST="/Library/LaunchDaemons/$LABEL.plist"
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
COM_APP=1
[[ "${1:-}" == "--sem-app" ]] && COM_APP=0

passo() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
falha() { printf '\033[31merro:\033[0m %s\n' "$*" >&2; exit 1; }

[[ "$(uname)" == "Darwin" ]] || falha "o Faro só roda no macOS"
[[ $EUID -ne 0 ]] || falha "rode como seu usuário (sem sudo); o script pede a senha quando precisar"

VERSAO="$(sw_vers -productVersion)"
MAIOR="${VERSAO%%.*}"
(( MAIOR >= 13 )) || falha "precisa do macOS 13 (Ventura) ou mais novo; o eslogger não existe no $VERSAO"
[[ -x /usr/bin/eslogger ]] || falha "/usr/bin/eslogger não encontrado"

cd "$RAIZ"
command -v cargo >/dev/null || falha "Rust não encontrado. Instale com: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
if (( COM_APP )); then
  command -v npm >/dev/null || falha "Node/npm não encontrado (necessário para o app)"
fi

passo "Compilando o daemon"
cargo build --release -p vigia-daemon
DAEMON_SRC="$RAIZ/target/release/vigia-daemon"

if (( COM_APP )); then
  passo "Compilando o app (Tauri)"
  # O daemon vai dentro do app, como na Release (o botão Instalar do Início usa este).
  cp "$DAEMON_SRC" app/src-tauri/resources/faro-daemon
  codesign --force --sign - --identifier dev.natan.faro.daemon app/src-tauri/resources/faro-daemon
  (cd app && npm ci --no-audit --no-fund && npx tauri build --bundles app)
  APP_SRC="$RAIZ/target/release/bundle/macos/Faro.app"
  [[ -d "$APP_SRC" ]] || falha "o build do app não gerou $APP_SRC"
fi

# Pasta de repositórios de terceiros: onde o código não confiável mora.
PADRAO_TERCEIROS="~/Documents/www/terceiros"
if [[ ! -f "$DATA_DIR/config.yaml" ]]; then
  passo "Configuração"
  echo "Em qual pasta você clona repositórios de terceiros (código em que você não confia)?"
  read -r -p "  [$PADRAO_TERCEIROS] " TERCEIROS
  TERCEIROS="${TERCEIROS:-$PADRAO_TERCEIROS}"
  mkdir -p "${TERCEIROS/#\~/$HOME}"
  TOPICO="vigia-$(LC_ALL=C tr -dc 'a-z0-9' </dev/urandom | head -c 24)"
fi

passo "Instalando o daemon (pede a senha de administrador)"
sudo install -d -o root -g wheel -m 755 "$BIN_DIR"
# Se o binário mudar, o macOS pode pedir o Acesso Total ao Disco de novo.
sudo install -o root -g wheel -m 755 "$DAEMON_SRC" "$BIN"
sudo install -d -o root -g wheel -m 700 "$DATA_DIR"
sudo install -d -o root -g wheel -m 755 "$LOG_DIR"

if [[ ! -f "$DATA_DIR/config.yaml" ]]; then
  sed -e "s|__HOME__|$HOME|" -e "s|__UID__|$(id -u)|" -e "s|__TERCEIROS__|$TERCEIROS|" -e "s|__TOPICO__|$TOPICO|" \
    "$RAIZ/config/config.exemplo.yaml" | sudo tee "$DATA_DIR/config.yaml" >/dev/null
  sudo chmod 600 "$DATA_DIR/config.yaml"
  echo "config criada em $DATA_DIR/config.yaml (tópico do ntfy: $TOPICO)"
else
  echo "config existente mantida: $DATA_DIR/config.yaml"
fi
sudo "$BIN" check --config "$DATA_DIR/config.yaml"

sudo install -o root -g wheel -m 644 "$RAIZ/launchd/$LABEL.plist" "$PLIST"

passo "Acesso Total ao Disco"
cat <<EOF
O daemon precisa de Acesso Total ao Disco para usar o Endpoint Security (eslogger)
e para ler as permissões de privacidade (TCC). Vou abrir os Ajustes:

  1. Clique em + (ou arraste o arquivo para a lista).
  2. Aperte Cmd+Shift+G, cole $BIN e confirme.
  3. Deixe a chave ligada.
EOF
open "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles" || true
open -R "$BIN" || true
read -r -p "Aperte Enter quando tiver liberado... "

passo "Iniciando o daemon"
sudo launchctl bootout "system/$LABEL" 2>/dev/null || true
# Até a 0.1.0 o binário se chamava /usr/local/vigia/vigia-daemon.
sudo rm -rf /usr/local/vigia
sudo launchctl bootstrap system "$PLIST"
sleep 3
if sudo launchctl print "system/$LABEL" | grep -q "state = running"; then
  echo "daemon rodando"
else
  echo "o daemon não está rodando; veja: sudo tail -50 $LOG_DIR/daemon.log"
fi
if sudo grep -q "Sensor não consegue rodar\|eslogger terminou" "$LOG_DIR/daemon.log" 2>/dev/null; then
  echo "atenção: o eslogger não subiu. Confira o Acesso Total ao Disco e reinicie com:"
  echo "  sudo launchctl kickstart -k system/$LABEL"
fi

if (( COM_APP )); then
  passo "Instalando o app em /Applications"
  # O app se chamava Vigia: tira a versão antiga com o nome velho, se houver.
  rm -rf /Applications/Vigia.app /Applications/Faro.app
  # mv, não cp: a cópia do build no projeto aparecia como um segundo Faro no Spotlight.
  mv "$APP_SRC" /Applications/Faro.app
  open /Applications/Faro.app
  echo "Para abrir o app no login: Ajustes > Geral > Itens de Início > + > Faro."
  echo "(O próprio Faro vai alertar esse item novo de inicialização: é esperado, clique em Permitir sempre.)"
fi

passo "Pronto"
cat <<EOF
Teste com a simulação segura de um ataque:   ./scripts/simular.sh
Acompanhe pelo terminal:                     $BIN tail
Log do daemon:                               sudo tail -f $LOG_DIR/daemon.log
EOF
