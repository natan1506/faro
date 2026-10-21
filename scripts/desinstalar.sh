#!/usr/bin/env bash
# Remove o Vigia. Alertas, exceções e quarentena ficam, a não ser com --tudo.
set -euo pipefail

LABEL="dev.natan.vigia"
DATA_DIR="/Library/Application Support/Vigia"

sudo launchctl bootout "system/$LABEL" 2>/dev/null || true
sudo rm -f "/Library/LaunchDaemons/$LABEL.plist" /var/run/vigia.sock
sudo rm -rf /usr/local/faro /usr/local/vigia
osascript -e 'quit app "Faro"' 2>/dev/null || true
osascript -e 'quit app "Vigia"' 2>/dev/null || true  # nome antigo do app
rm -rf /Applications/Faro.app /Applications/Vigia.app

if [[ "${1:-}" == "--tudo" ]]; then
  sudo rm -rf "$DATA_DIR" /Library/Logs/Vigia
  echo "Faro removido, inclusive dados, logs e quarentena."
else
  echo "Faro removido. Dados e quarentena continuam em $DATA_DIR (use --tudo para apagar)."
fi
echo "Lembre de tirar /usr/local/faro/faro-daemon do Acesso Total ao Disco em Ajustes."
