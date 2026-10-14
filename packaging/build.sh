#!/usr/bin/env bash
# Gera em dist/ os pacotes para várias distros:
#   .deb (Debian/Ubuntu/Pop!_OS/Mint) · .rpm (Fedora/openSUSE)
#   .pkg.tar.zst (Arch/Manjaro) · .tar.gz (qualquer distro: bin/vitrola instalar)
# Uso: packaging/build.sh [versão]   (padrão: derivada do git describe)
# Usa o nfpm instalado ou, na falta dele, a imagem docker goreleaser/nfpm.
set -euo pipefail

ROOT="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
NFPM_VERSION="${NFPM_VERSION:-v2.41.0}"
cd "$ROOT"

version="${1:-$(git describe --tags --always 2>/dev/null || echo 0.0.0)}"
version="${version#v}"
# v0.1.0-3-gabc123 -> 0.1.0+3.gabc123 (sem "-": rpm e pacman não aceitam)
version="$(sed -E 's/-([0-9]+)-g/+\1.g/; s/-/./g' <<<"$version")"
[[ "$version" =~ ^[0-9] ]] || version="0.0.0+git.$version"
export VERSION="$version"

nfpm() {
  if type -P nfpm >/dev/null; then
    command nfpm "$@"
  else
    docker run --rm --user "$(id -u):$(id -g)" -e VERSION -v "$ROOT:/src" -w /src \
      "goreleaser/nfpm:$NFPM_VERSION" "$@"
  fi
}

rm -rf dist && mkdir -p dist
for packager in deb rpm archlinux; do
  nfpm package --config packaging/nfpm.yaml --packager "$packager" --target dist/ >/dev/null
done

# Tarball universal: o projeto como está no git (instala com bin/vitrola instalar)
git ls-files -z | tar --null -T - --transform "s|^|vitrola-bt-$VERSION/|" -czf "dist/vitrola-bt-$VERSION.tar.gz"

(cd dist && sha256sum -- * > SHA256SUMS)
ls -1 dist
