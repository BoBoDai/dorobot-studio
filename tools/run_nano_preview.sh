#!/usr/bin/env bash
set -euo pipefail

studio_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$studio_root/tools/run_jit.sh" --nano "$@"
