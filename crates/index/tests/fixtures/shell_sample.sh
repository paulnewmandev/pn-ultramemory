#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Deploys the application. Fixture for the extraction tests.
set -euo pipefail

# Prints a message with a prefix.
log() {
  echo "[deploy] $*"
}

# Builds the project for a target.
build_project() {
  local target="$1"
  log "building ${target}"
  if [ -f Makefile ]; then
    make "${target}"
  fi
  case "$target" in
    release) strip_binaries ;;
  esac
}

function cleanup {
  rm -rf "$TMPDIR"
}

deploy_app()
{
  build_project "$1"
  cleanup
}

main() {
  deploy_app "${1:-release}"
}

main "$@"
