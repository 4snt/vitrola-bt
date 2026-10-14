#!/usr/bin/env bash
# Instala o pacote de dist/ dentro de um contêiner da distro e confere que o
# Vitrola BT, as bibliotecas que ele carrega e as ferramentas que chama ficaram
# disponíveis. Uso (no contêiner, na raiz do repo): packaging/test-install.sh
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
ok() { echo "ok    $1"; }
nao() { echo "FALTA $1"; falta=1; }

for c in vitrola-bt vitrola bluetoothctl btmgmt pw-cat pw-dump pw-link pw-record pw-metadata jq pkexec rfkill xdg-open; do
  if command -v "$c" >/dev/null; then ok "$c"; else nao "$c"; fi
done
for f in /usr/lib/vitrola-bt/classe-caixa \
         /usr/share/polkit-1/actions/io.github._4snt.VitrolaBT.policy \
         /usr/share/applications/io.github._4snt.VitrolaBT.desktop \
         /usr/share/icons/hicolor/scalable/apps/vitrola-bt.svg \
         /usr/share/icons/hicolor/256x256/apps/vitrola-bt.png \
         /usr/share/wireplumber/wireplumber.conf.d/51-vitrola-bt.conf \
         /usr/share/pipewire/pipewire.conf.d/51-vitrola-rates.conf; do
  if [[ -e "$f" ]]; then ok "$f"; else nao "$f"; fi
done
# Bibliotecas carregadas em tempo de execução (dlopen)
for lib in libsbc.so.1 libEGL.so.1 libGL.so.1 libwayland-client.so.0 libxkbcommon.so.0; do
  if ldconfig -p 2>/dev/null | grep -q "$lib" || find /usr/lib /usr/lib64 -name "$lib" 2>/dev/null | grep -q .; then
    ok "$lib"
  else
    nao "$lib"
  fi
done
if ldd /usr/bin/vitrola-bt | grep -q "not found"; then
  ldd /usr/bin/vitrola-bt; nao "dependências do binário"
else
  ok "ldd vitrola-bt"
fi
vitrola-bt --version
vitrola --help >/dev/null && ok "vitrola --help"
exit "$falta"
