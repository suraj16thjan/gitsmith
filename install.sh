#!/usr/bin/env bash
# gitsmith installer
#
#   curl -fsSL https://raw.githubusercontent.com/suraj16thjan/gitsmith/main/install.sh | bash
#
# Downloads the matching release binary from GitHub and installs it.
# Override behaviour with env vars:
#   GITSMITH_VERSION=v0.2.2   pin a specific tag (default: latest release)
#   GITSMITH_BIN_DIR=~/bin    install location (default: ~/.local/bin, or
#                             /usr/local/bin when writable / running as root)
set -euo pipefail

REPO="suraj16thjan/gitsmith"
BIN="gitsmith"

err() { printf 'error: %s\n' "$1" >&2; exit 1; }
info() { printf '%s\n' "$1" >&2; }

need() { command -v "$1" >/dev/null 2>&1 || err "required command not found: $1"; }
need curl
need tar

# --- detect platform --------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Darwin) os_part="apple-darwin" ;;
  Linux)  os_part="unknown-linux-gnu" ;;
  *) err "unsupported OS: $os (use 'cargo install gitsmith --locked' instead)" ;;
esac

case "$arch" in
  x86_64|amd64) arch_part="x86_64" ;;
  arm64|aarch64) arch_part="aarch64" ;;
  *) err "unsupported architecture: $arch" ;;
esac

# Only these target triples are published as tarballs (see .github/workflows/release.yml).
target="${arch_part}-${os_part}"
case "$target" in
  x86_64-apple-darwin|aarch64-apple-darwin|x86_64-unknown-linux-gnu) ;;
  *) err "no prebuilt binary for $target (use 'cargo install gitsmith --locked' instead)" ;;
esac

# --- resolve version --------------------------------------------------------
version="${GITSMITH_VERSION:-}"
if [ -z "$version" ]; then
  info "resolving latest release..."
  version="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)"
  [ -n "$version" ] || err "could not determine the latest release tag"
fi

asset="${BIN}-${version}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/${version}/${asset}"

# --- choose install dir -----------------------------------------------------
bin_dir="${GITSMITH_BIN_DIR:-}"
if [ -z "$bin_dir" ]; then
  if [ "$(id -u)" = "0" ] || [ -w /usr/local/bin ]; then
    bin_dir="/usr/local/bin"
  else
    bin_dir="${HOME}/.local/bin"
  fi
fi
mkdir -p "$bin_dir"

# --- download & install -----------------------------------------------------
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

info "downloading ${asset}..."
curl -fSL "$url" -o "${tmp}/${asset}" || err "download failed: $url"

tar xzf "${tmp}/${asset}" -C "$tmp"
[ -f "${tmp}/${BIN}" ] || err "archive did not contain the ${BIN} binary"

chmod +x "${tmp}/${BIN}"
install -m 0755 "${tmp}/${BIN}" "${bin_dir}/${BIN}" 2>/dev/null \
  || mv "${tmp}/${BIN}" "${bin_dir}/${BIN}"

info "installed ${BIN} ${version} to ${bin_dir}/${BIN}"
case ":${PATH}:" in
  *":${bin_dir}:"*) ;;
  *) info "note: ${bin_dir} is not on your PATH — add it, e.g.:"
     info "  export PATH=\"${bin_dir}:\$PATH\"" ;;
esac
