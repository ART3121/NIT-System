#!/bin/sh

set -eu

REPOSITORY="${NIT_REPOSITORY:-ART3121/NIT-System}"
INSTALL_DIR="${NIT_INSTALL_DIR:-${HOME}/.local/bin}"
VERSION="${NIT_VERSION:-latest}"
COMPONENT="${NIT_COMPONENT:-all}"
INSTALL_COMPLETIONS="${NIT_COMPLETIONS:-1}"
DATA_HOME="${XDG_DATA_HOME:-${HOME}/.local/share}"
CONFIG_HOME="${XDG_CONFIG_HOME:-${HOME}/.config}"

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
case "$COMPONENT" in
    all) asset="nit-system-${target}.tar.gz" ;;
    nit) asset="nit-${target}.tar.gz" ;;
    nitcat) asset="nitcat-${target}.tar.gz" ;;
    *) fail "NIT_COMPONENT must be all, nit, or nitcat" ;;
esac

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

case "$COMPONENT" in
    all) expected_members='LICENSE
nit
nitcat' ;;
    nit) expected_members='LICENSE
nit' ;;
    nitcat) expected_members='LICENSE
nitcat' ;;
esac
archive_members="$(tar -tzf "${temporary_directory}/${asset}" | LC_ALL=C sort)"
[ "$archive_members" = "$expected_members" ] || fail "release archive contains unexpected paths"
tar -tvzf "${temporary_directory}/${asset}" | awk '$1 !~ /^-/ { exit 1 }' \
    || fail "release archive contains links or non-regular files"
tar -xzf "${temporary_directory}/${asset}" -C "$temporary_directory" --no-same-owner --no-same-permissions

mkdir -p "$INSTALL_DIR"
if [ "$COMPONENT" = "all" ] || [ "$COMPONENT" = "nit" ]; then
    [ -f "${temporary_directory}/nit" ] || fail "release archive does not contain nit"
    install -m 755 "${temporary_directory}/nit" "${INSTALL_DIR}/nit"
    printf 'Installed nit to %s/nit\n' "$INSTALL_DIR"
fi
if [ "$COMPONENT" = "all" ] || [ "$COMPONENT" = "nitcat" ]; then
    [ -f "${temporary_directory}/nitcat" ] || fail "release archive does not contain nitcat"
    install -m 755 "${temporary_directory}/nitcat" "${INSTALL_DIR}/nitcat"
    printf 'Installed nitcat to %s/nitcat\n' "$INSTALL_DIR"
fi

install_completions() {
    executable="$1"
    completion_name="$2"
    bash_directory="${DATA_HOME}/bash-completion/completions"
    zsh_directory="${DATA_HOME}/zsh/site-functions"
    fish_directory="${CONFIG_HOME}/fish/completions"

    mkdir -p "$bash_directory" "$zsh_directory" "$fish_directory"
    "${INSTALL_DIR}/${executable}" -completions bash > "${temporary_directory}/${completion_name}.bash"
    "${INSTALL_DIR}/${executable}" -completions zsh > "${temporary_directory}/_${completion_name}"
    "${INSTALL_DIR}/${executable}" -completions fish > "${temporary_directory}/${completion_name}.fish"
    install -m 644 "${temporary_directory}/${completion_name}.bash" "${bash_directory}/${completion_name}"
    install -m 644 "${temporary_directory}/_${completion_name}" "${zsh_directory}/_${completion_name}"
    install -m 644 "${temporary_directory}/${completion_name}.fish" "${fish_directory}/${completion_name}.fish"
    printf 'Installed %s completions for Bash, Zsh, and Fish\n' "$executable"
}

if [ "$INSTALL_COMPLETIONS" != "0" ]; then
    if [ "$COMPONENT" = "all" ] || [ "$COMPONENT" = "nit" ]; then
        install_completions nit nit
    fi
    if [ "$COMPONENT" = "all" ] || [ "$COMPONENT" = "nitcat" ]; then
        install_completions nitcat nitcat
    fi
fi

if [ -f "${INSTALL_DIR}/nit-view" ]; then
    rm -f "${INSTALL_DIR}/nit-view"
    printf 'Removed legacy executable %s/nit-view\n' "$INSTALL_DIR"
fi
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *) printf 'Add %s to PATH before running nit.\n' "$INSTALL_DIR" ;;
esac
