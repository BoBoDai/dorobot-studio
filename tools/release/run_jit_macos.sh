#!/bin/sh
# Development entry: edit this checkout's Rust/Splash and reuse the RC runtime.
set -eu
SOURCE=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
APP=${DOROBOT_APP:-/Applications/DoRobot Studio.app}
if [ ! -f "$APP/Contents/Resources/release.json" ]; then
    echo 'Set DOROBOT_APP to the installed DoRobot Studio.app path.' >&2
    exit 1
fi
export DOROBOT_RESOURCES="$APP/Contents/Resources"
export PATH="$DOROBOT_RESOURCES/runtime/bin:$PATH"
export PKG_CONFIG_PATH="$DOROBOT_RESOURCES/runtime/ffmpeg/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
exec "$SOURCE/tools/run_jit.sh" "$@"
