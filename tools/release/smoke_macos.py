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
    profile += '(deny file-write* (subpath ' + json.dumps(str(app)) + '))\n'
    (root / "offline.sb").write_text(profile)
    env = dict(os.environ, HOME=str(root / "home"), PATH="/usr/bin:/bin:/usr/sbin:/sbin",
               MAKEPAD_HIDE_WINDOWS="1", DOROBOT_STUDIO_STATE_DIR=str(root / "state"),
               DOROBOT_UX_REPORT=str(root / "ui.json"), PYTHONDONTWRITEBYTECODE="1", PYTHONNOUSERSITE="1")
    for key in ["DOROBOT_RESOURCES", "DOROBOT_AISLE", "DOROBOT_PYTHON", "PYTHONPATH", "PYTHONHOME", "DYLD_LIBRARY_PATH"]:
        env.pop(key, None)
    sandbox = ["/usr/bin/sandbox-exec", "-f", str(root / "offline.sb")]
    python = app / "Contents/Resources/runtime/python/bin/python3"
    # Prove the restriction, rather than infer offline operation from a successful run.
    deny_check = '''import socket
s=socket.socket(); s.settimeout(2)
try: s.connect(("192.0.2.1",443))
except PermissionError: print("External network denied")
else: raise AssertionError("External network was not denied")
'''
    subprocess.run([*sandbox, str(python), "-I", "-B", "-c", deny_check], check=True, env=env)
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
            subprocess.run([str(python), "-B", str(ROOT / "tools/ux/workflow_journeys.py"), "--pid", str(process.pid),
                            "--remote", str(a.remote), "--report", str(root / "ui.json"), "--out", str(root / mode), "--mode", mode], check=True)
        if "is not packaged with this app" in (root / "app.log").read_text():
            raise AssertionError("Missing packaged fonts")
        print("PASS: moved bundle; empty HOME; no external network or source checkout; simulation, capture, replay and mock-base journeys")
    finally:
        subprocess.run([str(a.remote), "app", str(process.pid), "quit"], check=False)
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=10)


if __name__ == "__main__":
    main()
