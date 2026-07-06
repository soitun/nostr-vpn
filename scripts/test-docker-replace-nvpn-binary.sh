#!/usr/bin/env bash
# Local self-test for the Docker nvpn binary replacement guard.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
"$ROOT_DIR/scripts/docker-replace-nvpn-binary" --self-test
