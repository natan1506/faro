#!/usr/bin/env bash
# Roda a simulação segura do incidente (lab/simulacao.mjs) de dentro da pasta de
# repositórios de terceiros, como o malware rodou. Desfaz tudo no fim, mesmo
# se você encerrar o processo pelo alerta.
#
#   ./scripts/simular.sh          # espera 90 s no fim para testar "Encerrar processo"
#   ./scripts/simular.sh 0        # sem espera
set -euo pipefail

ESPERA="${1:-90}"
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="/Library/Application Support/Vigia/config.yaml"
LABEL="dev.natan.vigia-simulacao"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

command -v node >/dev/null || { echo "precisa do node"; exit 1; }

# Primeira pasta de terceiros da config (o arquivo é só do root).
TERCEIROS="$(sudo awk '/^third_party_dirs:/{f=1;next} f&&/^ *- /{sub(/^ *- */,"");gsub(/"/,"");print;exit} f&&/^[^ ]/{exit}' "$CONFIG" 2>/dev/null || true)"
TERCEIROS="${TERCEIROS:-~/Documents/www/terceiros}"
TERCEIROS="${TERCEIROS/#\~/$HOME}"

LAB="$TERCEIROS/vigia-simulacao"
ALVO="$(mktemp -d)/projeto-falso"

limpar() {
  launchctl bootout "gui/$(id -u)" "$PLIST" 2>/dev/null || true
  rm -f "$PLIST"
  rm -rf "$LAB" "$(dirname "$ALVO")"
}
trap limpar EXIT

mkdir -p "$LAB" "$ALVO"
git -C "$LAB" init -q 2>/dev/null || true
cp "$RAIZ/lab/simulacao.mjs" "$LAB/"
printf 'CHAVE_FALSA=isto-nao-e-um-segredo\n' >"$ALVO/.env"

echo "Repositório de terceiros falso: $LAB"
echo "Alvo falso:                     $ALVO/.env"
echo
(cd "$LAB" && node simulacao.mjs "$ALVO/.env" "$ESPERA") || echo "(a simulação foi encerrada: se foi pelo alerta, o botão funcionou)"
echo
echo "Esperado no Faro: leitura de segredo, comando suspeito (curl), binário sem assinatura e item de inicialização."
