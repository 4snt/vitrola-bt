#!/usr/bin/env bash
# Gera dist/vitrola-bt_<versão>_all.deb a partir do repositório.
# Uso: packaging/build-deb.sh [versão]   (padrão: git describe, ou 0.0.0)
set -euo pipefail

ROOT="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
VERSION="${1:-$(git -C "$ROOT" describe --tags --always 2>/dev/null || echo 0.0.0)}"
VERSION="${VERSION#v}"
[[ "$VERSION" =~ ^[0-9] ]] || VERSION="0.0.0+git$VERSION"
APP_ID="io.github._4snt.VitrolaBT"
PKG="$(mktemp -d)"
trap 'rm -rf "$PKG"' EXIT

share="$PKG/usr/share/vitrola-bt"
install -d "$share/bin" "$share/vitrola_bt" "$PKG/usr/bin" \
  "$PKG/usr/share/applications" "$PKG/usr/share/icons/hicolor/scalable/apps" \
  "$PKG/usr/share/doc/vitrola-bt" \
  "$PKG/usr/share/wireplumber/wireplumber.conf.d" "$PKG/usr/share/pipewire/pipewire.conf.d" \
  "$PKG/DEBIAN"

install -m755 "$ROOT/bin/vitrola" "$ROOT/bin/vitrola-gui" "$share/bin/"
install -m644 "$ROOT"/vitrola_bt/*.py "$share/vitrola_bt/"
ln -s ../share/vitrola-bt/bin/vitrola "$PKG/usr/bin/vitrola"
ln -s ../share/vitrola-bt/bin/vitrola-gui "$PKG/usr/bin/vitrola-gui"
install -m644 "$ROOT/data/$APP_ID.desktop" "$PKG/usr/share/applications/"
install -m644 "$ROOT/data/vitrola-bt.svg" "$PKG/usr/share/icons/hicolor/scalable/apps/"
# Configs de áudio valem para todos os usuários (drop-ins do sistema)
install -m644 "$ROOT/config/wireplumber/51-vitrola-bt.conf" "$PKG/usr/share/wireplumber/wireplumber.conf.d/"
install -m644 "$ROOT/config/pipewire/51-vitrola-rates.conf" "$PKG/usr/share/pipewire/pipewire.conf.d/"
install -m644 "$ROOT/README.md" "$ROOT/LICENSE" "$PKG/usr/share/doc/vitrola-bt/"

cat > "$PKG/DEBIAN/control" <<CTRL
Package: vitrola-bt
Version: $VERSION
Architecture: all
Maintainer: Murilo Santiago <murilo.escobedo@protonmail.com>
Depends: bluez, pipewire, pipewire-bin, wireplumber, pipewire-pulse | pulseaudio-utils, python3, python3-gi, python3-dbus, gir1.2-gtk-3.0, jq, pkexec | policykit-1, rfkill
Section: sound
Priority: optional
Homepage: https://github.com/4snt/vitrola-bt
Description: Recebe o áudio Bluetooth de uma vitrola no Linux
 Faz o PC se passar por caixa de som Bluetooth (A2DP Sink) para que a vitrola
 pareie nele, toca o disco nos alto-falantes e grava em FLAC. Traz app
 gráfico (GTK) e o comando de terminal "vitrola".
CTRL

cat > "$PKG/DEBIAN/postinst" <<'POST'
#!/bin/sh
set -e
command -v update-desktop-database >/dev/null && update-desktop-database -q /usr/share/applications || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true
echo "vitrola-bt: reinicie o áudio para aplicar as configs:"
echo "  systemctl --user restart wireplumber pipewire pipewire-pulse"
POST
chmod 755 "$PKG/DEBIAN/postinst"

mkdir -p "$ROOT/dist"
out="$ROOT/dist/vitrola-bt_${VERSION}_all.deb"
dpkg-deb --root-owner-group --build "$PKG" "$out" >/dev/null
echo "$out"
