#!/usr/bin/env bash
# Instala o Vitrola BT do tarball em qualquer distro (precisa de sudo).
# Uso: sudo ./instalar.sh   |   sudo ./instalar.sh --remover
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"
[[ $EUID -eq 0 ]] || { echo "rode com sudo: sudo ./instalar.sh" >&2; exit 1; }

arquivos=(
  /usr/local/bin/vitrola-bt
  /usr/local/bin/vitrola
  /usr/lib/vitrola-bt/classe-caixa
  /usr/share/polkit-1/actions/io.github._4snt.VitrolaBT.policy
  /usr/local/share/applications/io.github._4snt.VitrolaBT.desktop
  /usr/local/share/icons/hicolor/scalable/apps/vitrola-bt.svg
  /usr/local/share/icons/hicolor/256x256/apps/vitrola-bt.png
  /etc/wireplumber/wireplumber.conf.d/51-vitrola-bt.conf
  /etc/pipewire/pipewire.conf.d/51-vitrola-rates.conf
)
if [[ "${1:-}" == "--remover" ]]; then
  rm -f "${arquivos[@]}"
  rmdir /usr/lib/vitrola-bt 2>/dev/null || true
  echo "Vitrola BT removido."
  exit 0
fi
install -Dm755 vitrola-bt /usr/local/bin/vitrola-bt
install -Dm755 bin/vitrola /usr/local/bin/vitrola
install -Dm755 packaging/classe-caixa /usr/lib/vitrola-bt/classe-caixa
install -Dm644 packaging/io.github._4snt.VitrolaBT.policy /usr/share/polkit-1/actions/io.github._4snt.VitrolaBT.policy
install -Dm644 data/io.github._4snt.VitrolaBT.desktop /usr/local/share/applications/io.github._4snt.VitrolaBT.desktop
install -Dm644 data/vitrola-bt.svg /usr/local/share/icons/hicolor/scalable/apps/vitrola-bt.svg
install -Dm644 data/vitrola-bt-256.png /usr/local/share/icons/hicolor/256x256/apps/vitrola-bt.png
install -Dm644 config/wireplumber/51-vitrola-bt.conf /etc/wireplumber/wireplumber.conf.d/51-vitrola-bt.conf
install -Dm644 config/pipewire/51-vitrola-rates.conf /etc/pipewire/pipewire.conf.d/51-vitrola-rates.conf
update-desktop-database -q /usr/local/share/applications 2>/dev/null || true
echo "Vitrola BT instalado. Reinicie o áudio na sua sessão:"
echo "  systemctl --user restart wireplumber pipewire pipewire-pulse"
