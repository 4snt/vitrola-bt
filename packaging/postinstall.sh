#!/bin/sh
# Atualiza menu/ícones e lembra de reiniciar o áudio para aplicar as configs.
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true
fi
echo "vitrola-bt: para aplicar as configs de áudio, rode como seu usuário:"
echo "  systemctl --user restart wireplumber pipewire pipewire-pulse"
exit 0
