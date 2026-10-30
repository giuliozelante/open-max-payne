#!/usr/bin/env bash
# Build Open Max Payne Studio (works from fish/zsh/bash — do not `source` this).
set -euo pipefail
cd "$(dirname "$0")"
# shellcheck disable=SC1091
source ./rust-env.sh
exec bun run tauri build "$@"
