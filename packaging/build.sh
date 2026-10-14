#!/usr/bin/env bash
# Gera em dist/ os pacotes do Vitrola BT (binário Rust) para várias distros:
#   .deb (Debian/Ubuntu/Pop!_OS/Mint) · .rpm (Fedora/openSUSE)
#   .pkg.tar.zst (Arch/Manjaro) · .tar.gz (qualquer distro: ./instalar.sh)
# Uso: packaging/build.sh [versão]   (padrão: derivada do git describe)
# Compila com cargo se preciso; usa o nfpm instalado ou a imagem docker.
set -euo pipefail

ROOT="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
NFPM_VERSION="${NFPM_VERSION:-v2.41.0}"
cd "$ROOT"

# vX.Y.Z[-rcN] exato -> como está; fora de tag -> vX.Y.Z-dev.N+gSHA (semver)
desc="${1:-$(git describe --tags --always --match 'v*' 2>/dev/null || echo v0.0.0)}"
if [[ "$desc" =~ ^(v?[0-9]+\.[0-9]+\.[0-9]+)(-[0-9A-Za-z.]+)?-([0-9]+)-g([0-9a-f]+)$ ]]; then
  pre="${BASH_REMATCH[2]#-}"
  desc="${BASH_REMATCH[1]}-${pre:+$pre.}dev.${BASH_REMATCH[3]}+g${BASH_REMATCH[4]}"
fi
[[ "$desc" =~ ^v?[0-9] ]] || desc="v0.0.0-dev+g$desc"
desc="${desc#v}"
# Versão e pré-versão separadas para o nFPM formatar cada pacote do jeito certo
# (deb/rpm: 0.3.0~rc1, que vem ANTES de 0.3.0; arch: 0.3.0rc1)
export VERSION="${desc%%-*}"
if [[ "$desc" == *-* ]]; then
  export PRERELEASE="${desc#*-}"
else
  export PRERELEASE=""
fi
export ARCH="${ARCH:-amd64}"

if [[ ! -x target/release/vitrola-bt || -n "${COMPILAR:-}" ]]; then
  cargo build --release --locked
fi

nfpm() {
  if type -P nfpm >/dev/null; then
    command nfpm "$@"
  else
    docker run --rm --user "$(id -u):$(id -g)" -e VERSION -e PRERELEASE -e ARCH -v "$ROOT:/src" -w /src \
      "goreleaser/nfpm:$NFPM_VERSION" "$@"
  fi
}

rm -rf dist && mkdir -p dist
for packager in deb rpm archlinux; do
  nfpm package --config packaging/nfpm.yaml --packager "$packager" --target dist/ >/dev/null
done

# Tarball para outras distros: binário + dados + instalador
pasta="vitrola-bt-$desc-linux-x86_64"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/$pasta"
cp -r target/release/vitrola-bt bin data config README.md LICENSE THIRD_PARTY.md "$tmp/$pasta/"
mkdir -p "$tmp/$pasta/packaging"
cp packaging/classe-caixa packaging/io.github._4snt.VitrolaBT.policy packaging/instalar.sh "$tmp/$pasta/packaging/"
cp packaging/instalar.sh "$tmp/$pasta/instalar.sh"
tar -C "$tmp" -czf "dist/$pasta.tar.gz" "$pasta"

(cd dist && sha256sum -- * > SHA256SUMS)
ls -1 dist
