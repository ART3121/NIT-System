#!/bin/sh

set -eu

REPOSITORY="${NIT_REPOSITORY:-ART3121/NIT-System}"
INSTALL_DIR="${NIT_INSTALL_DIR:-${HOME}/.local/bin}"
VERSION="${NIT_VERSION:-latest}"

fail() {
    printf 'nit installer: %s\n' "$1" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"
command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is required"

case "$(uname -s)" in
    Linux) os="unknown-linux-gnu" ;;
    *) fail "unsupported operating system: $(uname -s)" ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) arch="x86_64" ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
esac

target="${arch}-${os}"
asset="nit-${target}.tar.gz"

if [ "$VERSION" = "latest" ]; then
    download_base="https://github.com/${REPOSITORY}/releases/latest/download"
else
    case "$VERSION" in
        v*) tag="$VERSION" ;;
        *) tag="v${VERSION}" ;;
    esac
    download_base="https://github.com/${REPOSITORY}/releases/download/${tag}"
fi

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT HUP INT TERM

printf 'Downloading NIT System for %s...\n' "$target"
curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
    --output "${temporary_directory}/${asset}" \
    "${download_base}/${asset}"
curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
    --output "${temporary_directory}/${asset}.sha256" \
    "${download_base}/${asset}.sha256"

(
    cd "$temporary_directory"
    sha256sum --check "${asset}.sha256"
)

tar -xzf "${temporary_directory}/${asset}" -C "$temporary_directory"
[ -f "${temporary_directory}/nit" ] || fail "release archive does not contain nit"

mkdir -p "$INSTALL_DIR"
install -m 755 "${temporary_directory}/nit" "${INSTALL_DIR}/nit"

printf 'Installed nit to %s/nit\n' "$INSTALL_DIR"
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *) printf 'Add %s to PATH before running nit.\n' "$INSTALL_DIR" ;;
esac
