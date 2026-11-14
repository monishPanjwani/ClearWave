#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

if ! command -v cargo >/dev/null 2>&1 && [[ -x "$HOME/.cargo/bin/cargo" ]]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi

if ! command -v cargo >/dev/null 2>&1; then
  cat >&2 <<'MESSAGE'
Rust is not installed or cargo is not on PATH.
Install a current stable Rust toolchain from https://rustup.rs, then rerun:
  ./scripts/build.sh
This script does not install system packages or change audio settings.
MESSAGE
  exit 1
fi

if [[ "${1:-gui}" != gui && "${1:-gui}" != cli ]]; then
  printf 'Usage: %s [gui|cli]\n' "$0" >&2
  exit 2
fi
cargo fmt --check

case "${1:-gui}" in
  gui) cargo build --release --locked ;;
  cli) cargo build --release --locked --no-default-features ;;
  *) printf 'Usage: %s [gui|cli]\n' "$0" >&2; exit 2 ;;
esac
mkdir -p dist
cp target/release/clearwave dist/clearwave
printf 'Built %s/dist/clearwave\n' "$PWD"
printf 'Launch: ./dist/clearwave\nDiagnose: ./dist/clearwave doctor --json\n'

