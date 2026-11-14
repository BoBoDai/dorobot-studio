#!/usr/bin/env bash
set -euo pipefail

studio_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$studio_root"
dataset="$studio_root/dataset/adora-nano-synthetic-preview"

if [[ ! -f "$dataset/meta/info.json" ]]; then
    cargo run --locked -p dorobot-flex --no-default-features \
        --example nano_preview -- "$dataset"
fi

exec cargo run --locked -p dorobot-flex --no-default-features \
    --bin dorobot-flex -- --dataset "$dataset" --screen play "$@"
