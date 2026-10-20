#!/usr/bin/env bash
# Fase 0: grava os eventos do Endpoint Security durante o seu uso normal e
# mostra o volume e o que viraria alerta, para ajustar as exceções antes de
# deixar o daemon rodando.
#
#   ./scripts/medir.sh          # 10 minutos
#   ./scripts/medir.sh 3600     # 1 hora
#
# O Terminal (ou o Warp) precisa de Acesso Total ao Disco para o eslogger rodar.
set -euo pipefail

SEGUNDOS="${1:-600}"
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
SAIDA="${TMPDIR:-/tmp}/vigia-medida-$(date +%Y%m%d-%H%M%S).jsonl"
CONFIG="/Library/Application Support/Vigia/config.yaml"

if [[ ! -x "$RAIZ/target/release/vigia-daemon" ]]; then
  (cd "$RAIZ" && cargo build --release -p vigia-daemon)
fi

echo "Gravando eventos por $SEGUNDOS s em $SAIDA"
echo "Use o Mac normalmente (rode o next dev, npm install, abra o editor...)."
sudo -v
# O eslogger não tem opção de duração; um sleep em segundo plano encerra a gravação.
sudo /usr/bin/eslogger exec open btm_launch_item_add >"$SAIDA" &
PID=$!
trap 'sudo kill $PID 2>/dev/null || true' EXIT
sleep "$SEGUNDOS"
sudo kill "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
trap - EXIT

echo
if sudo test -f "$CONFIG"; then
  sudo "$RAIZ/target/release/vigia-daemon" analisar --config "$CONFIG" "$SAIDA"
else
  TMPCFG="$(mktemp)"
  sed -e "s|__HOME__|$HOME|" -e "s|__UID__|$(id -u)|" -e "s|__TERCEIROS__|~/Documents/www/terceiros|" -e "s|__TOPICO__||" \
    "$RAIZ/config/config.exemplo.yaml" >"$TMPCFG"
  "$RAIZ/target/release/vigia-daemon" analisar --config "$TMPCFG" "$SAIDA"
  rm -f "$TMPCFG"
fi
echo
echo "Gravação mantida em $SAIDA ($(du -h "$SAIDA" | cut -f1)). Apague quando terminar: ela contém caminhos de arquivos que você abriu."
