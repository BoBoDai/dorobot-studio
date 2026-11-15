#!/usr/bin/env python3
"""Verify bundle portability, signing, dependency versions and privacy before shipping."""
import argparse
import hashlib
import json
import os
import plistlib
from pathlib import Path
import re
import subprocess

from build_macos import macho, dependencies


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("app", type=Path)
    p.add_argument("--report", type=Path, required=True)
    a = p.parse_args()
    app = a.app.resolve()
    resources = app / "Contents/Resources"
    release = json.loads((resources / "release.json").read_text())
    errors, native, deployment = [], [], []
    private = str(Path.home()).encode()
    manifest = {}
    for f in sorted(app.rglob("*")):
        relative = str(f.relative_to(app))
        if f.is_symlink():
            if not f.resolve().is_relative_to(app):
                errors.append(f"External symlink: {relative}")
            continue
        if not f.is_file():
            continue
        content = f.read_bytes()
        manifest[relative] = hashlib.sha256(content).hexdigest()
        if private in content:
            errors.append(f"Personal build path: {relative}")
        if f.suffix == ".pyc" or f.name in {".env", ".DS_Store", "session.json"}:
            errors.append(f"Local state/cache: {relative}")
        if macho(f):
            native.append(relative)
            for dep in dependencies(f):
                if dep.startswith("/") and not dep.startswith(("/System/Library/", "/usr/lib/")):
                    errors.append(f"External native dependency: {relative} -> {dep}")
            loads = subprocess.check_output(["otool", "-arch", "arm64", "-l", str(f)], text=True)
            deployment += re.findall(r"\bminos ([\d.]+)", loads)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    python = resources / "runtime/python/bin/python3"
    env = dict(os.environ, PATH=str(resources / "runtime/bin") + ":/usr/bin:/bin", PYTHONDONTWRITEBYTECODE="1", PYTHONNOUSERSITE="1")
    version = subprocess.check_output([str(resources / "runtime/bin/dora"), "--version"], text=True, env=env).strip()
    if version != "dora-cli " + release["dora"]:
        errors.append("Dora CLI version mismatch")
    code = '''import json, sys, importlib.metadata as m
assert sys.version.split()[0] == "3.13.5"
import dora, nexus3d, cv2, pyarrow, serial
sys.path.insert(0, sys.argv[1])
from aisle.sim.nexus_backend import _ensure_nexus, _new_scene, NexusRobot
print(json.dumps({name:m.version(name) for name in ["dora-rs", "dora-rs-cli", "dimforge-nexus3d", "opencv-python-headless", "pyarrow", "pyserial"]}))
'''
    versions = json.loads(subprocess.check_output([str(python), "-I", "-B", "-c", code, str(resources / "aisle/src")], text=True, env=env))
    highest = max(deployment, key=lambda s: tuple(map(int, s.split("."))), default="unknown")
    declared = plistlib.loads((app / "Contents/Info.plist").read_bytes())["LSMinimumSystemVersion"]
    if highest != "unknown" and tuple(map(int, highest.split("."))) > tuple(map(int, declared.split("."))):
        errors.append(f"Deployment target {highest} exceeds declared minimum {declared}")
    if versions["dora-rs"] != release["dora"] or versions["dimforge-nexus3d"] != release["nexus"]:
        errors.append("Python runtime versions differ from the release manifest")
    result = {"passed": not errors, "version": release["version"], "files": len(manifest),
              "native_files": len(native), "highest_macos_deployment_target": highest,
              "dependencies": versions, "errors": errors, "sha256": manifest}
    a.report.parent.mkdir(parents=True, exist_ok=True)
    a.report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k:v for k,v in result.items() if k != "sha256"}))
    if errors:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
