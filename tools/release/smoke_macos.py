#!/usr/bin/env python3
"""Exercise the moved app offline through Makepad instrumentation, using an empty HOME.

No physical robot is opened. The real-errors journey only exercises a mock bus.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import sys

ROOT = Path(__file__).resolve().parents[2]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("app", type=Path)
    p.add_argument("--out", type=Path, required=True, help="New disposable acceptance folder")
    p.add_argument("--remote", type=Path, required=True)
    a = p.parse_args()
    root = a.out.resolve()
    root.mkdir(parents=True, exist_ok=False)
    app = root / "Relocated Folder/DoRobot Studio.app"
    app.parent.mkdir()
    subprocess.run(["ditto", str(a.app.resolve()), str(app)], check=True)
    (root / "home").mkdir()
    denied = [ROOT, Path("/tmp/dorobot-rc-build"), Path.home() / ".cargo", Path.home() / ".cache", Path.home() / ".local"]
    profile = '(version 1)\n(allow default)\n(deny network-outbound)\n(allow network-outbound (remote ip "localhost:*") (subpath "/private/tmp") (subpath "/tmp"))\n'
    profile += '(deny file-read* ' + ' '.join('(subpath ' + json.dumps(str(d)) + ')' for d in denied) + ')\n'
    profile += '(deny process-exec (regex #".*/python[0-9.]*$"))\n'
    profile += '(deny file-write* (subpath ' + json.dumps(str(app)) + '))\n'
    (root / "offline.sb").write_text(profile)
    env = dict(os.environ, HOME=str(root / "home"), PATH="/usr/bin:/bin:/usr/sbin:/sbin",
               MAKEPAD_HIDE_WINDOWS="1", DOROBOT_STUDIO_STATE_DIR=str(root / "state"),
               DOROBOT_UX_REPORT=str(root / "ui.json"))
    for key in ["DOROBOT_RESOURCES", "DOROBOT_AISLE", "DOROBOT_PYTHON", "DOROBOT_NATIVE_RUNTIME", "DOROBOT_DORA", "PYTHONPATH", "PYTHONHOME", "DYLD_LIBRARY_PATH"]:
        env.pop(key, None)
    sandbox = ["/usr/bin/sandbox-exec", "-f", str(root / "offline.sb")]
    # The host test runner stays outside the app sandbox. Python execution inside
    # the app is explicitly denied, in addition to source/cache/network access.
    check = subprocess.run([*sandbox, "/usr/bin/curl", "-v", "--noproxy", "*", "--connect-timeout", "2", "https://192.0.2.1"], env=env, capture_output=True, text=True)
    if check.returncode == 0 or "Operation not permitted" not in check.stderr:
        raise AssertionError("External network denial was not verified")
    check = subprocess.run([*sandbox, "/usr/bin/python3", "-V"], env=env, capture_output=True, text=True)
    if check.returncode == 0 or "Operation not permitted" not in check.stderr:
        raise AssertionError("Python execution denial was not verified")
    with (root / "app.log").open("w") as log:
        process = subprocess.Popen([*sandbox, str(app / "Contents/MacOS/dorobot-flex")], cwd="/tmp", env=env,
                                   stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    (root / "pid").write_text(str(process.pid))
    try:
        deadline = time.monotonic() + 30
        while not (root / "ui.json").exists():
            if process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("Packaged app failed to start; see app.log")
            time.sleep(.2)
        for mode in ["simulation", "real-errors"]:
            subprocess.run([sys.executable, "-B", str(ROOT / "tools/ux/workflow_journeys.py"), "--pid", str(process.pid),
                            "--remote", str(a.remote), "--report", str(root / "ui.json"), "--out", str(root / mode), "--mode", mode], check=True)
        if "is not packaged with this app" in (root / "app.log").read_text():
            raise AssertionError("Missing packaged fonts")
        print("PASS: moved bundle; empty HOME; no Python execution, external network or source checkout; simulation, capture, replay and mock-base journeys")
    finally:
        subprocess.run([str(a.remote), "app", str(process.pid), "quit"], check=False)
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=10)


if __name__ == "__main__":
    main()
