#!/bin/sh
# Atualiza menu/ícones e lembra de reiniciar o áudio para aplicar as configs.
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q /usr/share/applications || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true
echo "vitrola-bt: para aplicar as configs de áudio, rode como seu usuário:"
echo "  systemctl --user restart wireplumber pipewire pipewire-pulse"
exit 0
