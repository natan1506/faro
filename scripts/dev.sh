#!/usr/bin/env bash
# Desenvolvimento sem root e sem instalar nada: sobe o daemon em modo replay com
# uma gravação fictícia de um ataque e abre o app em modo dev apontando para ele.
# Os alertas descem da pílula do topo uns 25 s depois de o daemon subir.
set -euo pipefail

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
DEV="${TMPDIR:-/tmp}/vigia-dev"
SOCK="$DEV/vigia.sock"
mkdir -p "$DEV"
rm -f "$DEV/vigia.db"* "$DEV/ajustes.yaml"

sed -e "s|__HOME__|/Users/dev|" -e "s|__UID__|$(id -u)|" -e "s|__TERCEIROS__|~/repos-terceiros|" -e "s|__TOPICO__||" \
  -e "s|^learning_hours:.*|learning_hours: 0|" "$RAIZ/config/config.exemplo.yaml" >"$DEV/config.yaml"
cat >>"$DEV/config.yaml" <<EOF
paths:
  socket: $SOCK
  db: $DEV/vigia.db
  allow_file: $DEV/allow.yaml
  quarantine_dir: $DEV/quarentena
  settings_file: $DEV/ajustes.yaml
EOF

cd "$RAIZ"
cargo build -p vigia-daemon
# O replay começa depois de 25 s, para dar tempo de o app compilar e conectar.
./target/debug/vigia-daemon run --config "$DEV/config.yaml" --no-pollers --replay-delay 25 \
  --replay "$RAIZ/daemon/tests/fixtures/ataque.jsonl" &
DAEMON=$!
trap 'kill $DAEMON 2>/dev/null || true' EXIT

cd app
npm install --no-audit --no-fund
VIGIA_SOCKET="$SOCK" npx tauri dev
