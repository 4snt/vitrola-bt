#!/usr/bin/env bash
# Instala o pacote de dist/ dentro de um contêiner da distro e confere que o
# vitrola-bt e todas as ferramentas que ele chama ficaram disponíveis.
# Uso (no contêiner, na raiz do repo): packaging/test-install.sh
set -euo pipefail
# shellcheck source=/dev/null
. /etc/os-release
echo "== $PRETTY_NAME"

case "$ID ${ID_LIKE:-}" in
  *debian*|*ubuntu*)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq ./dist/*.deb >/dev/null ;;
  *fedora*|*rhel*)
    dnf install -y -q ./dist/*.rpm ;;
  *arch*)
    pacman -Syu --noconfirm --needed -q >/dev/null
    pacman -U --noconfirm dist/*.pkg.tar.zst >/dev/null ;;
  *) echo "distro não suportada: $ID"; exit 1 ;;
esac

falta=0
for c in vitrola vitrola-gui bluetoothctl btmgmt pw-dump pw-link pw-record pw-metadata jq pkexec rfkill python3; do
  if command -v "$c" >/dev/null; then echo "ok    $c"; else echo "FALTA $c"; falta=1; fi
done
for f in /usr/share/applications/io.github._4snt.VitrolaBT.desktop \
         /usr/share/icons/hicolor/scalable/apps/vitrola-bt.svg \
         /usr/share/wireplumber/wireplumber.conf.d/51-vitrola-bt.conf \
         /usr/share/pipewire/pipewire.conf.d/51-vitrola-rates.conf; do
  if [[ -f "$f" ]]; then echo "ok    $f"; else echo "FALTA $f"; falta=1; fi
done

vitrola --help >/dev/null
# Importa o app inteiro (GTK, D-Bus, agente) como o lançador do menu faria
python3 -c "import sys; sys.path.insert(0, '/usr/share/vitrola-bt'); import vitrola_bt.gui, vitrola_bt.agente, vitrola_bt.bandeja"
echo "ok    import vitrola_bt.gui"
exit "$falta"
