#!/usr/bin/env python3
"""Build a relocatable Nano RC; build-time tools are not needed by recipients.

Requires macOS arm64, Rust, uv, Python 3.13.5 (uv python install), and FFmpeg
headers/libraries exposed through PKG_CONFIG_PATH. All subprocesses use argv.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import runpy
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
CONFIG = json.loads((ROOT / "tools/release/release-config.json").read_text())


def run(*args, **kwargs):
    return subprocess.run([str(x) for x in args], check=True, **kwargs)


def output(*args):
    return subprocess.check_output([str(x) for x in args], text=True).strip()


def tracked(source, target, prefixes):
    names = output("git", "-C", source, "ls-files", "-z").split("\0")
    for name in names:
        if name and any(name == p or name.startswith(p + "/") for p in prefixes):
            src, dst = source / name, target / name
            if src.is_file():
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dst)


def macho(path):
    if path.is_symlink() or not path.is_file():
        return False
    with path.open("rb") as stream:
        return stream.read(4) in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xca\xfe\xba\xbe")


def dependencies(path):
    return [line.strip().split(" (compatibility")[0]
            for line in output("otool", "-L", path).splitlines()[1:] if line.startswith("\t")]


def relocate_ffmpeg(binary, frameworks):
    """Copy the full non-system dylib closure and rewrite load commands."""
    pending, done = [binary], set()
    while pending:
        item = pending.pop()
        if item in done:
            continue
        done.add(item)
        for dep in dependencies(item):
            if dep.startswith(("/System/Library/", "/usr/lib/", "@")):
                continue
            original = Path(dep)
            if not original.is_file():
                raise RuntimeError(f"Unresolved native dependency: {dep}")
            dest = frameworks / original.name
            if not dest.exists():
                shutil.copy2(original, dest)
                run("chmod", "u+w", dest)
                run("install_name_tool", "-id", "@rpath/" + dest.name, dest)
                pending.append(dest)
            run("install_name_tool", "-change", dep, "@rpath/" + dest.name, item)
    # Strip build-machine rpaths (the loader must not depend on Homebrew/cache).
    lines = output("otool", "-l", binary).splitlines()
    for i, line in enumerate(lines):
        if line.strip() == "cmd LC_RPATH":
            old = lines[i+2].strip().split("path ", 1)[1].split(" (offset")[0]
            if old.startswith("/"):
                run("install_name_tool", "-delete_rpath", old, binary)
    run("install_name_tool", "-add_rpath", "@executable_path/../Frameworks", binary)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--aisle", type=Path, required=True)
    p.add_argument("--uv", default="uv")
    p.add_argument("--python-home", type=Path)
    p.add_argument("--target", type=Path, default=ROOT / "target/rc")
    p.add_argument("--out", type=Path, default=ROOT / "dist")
    p.add_argument("--build-root", type=Path, default=Path("/tmp/dorobot-rc-build"))
    p.add_argument("--ffmpeg-source", type=Path, required=True, help="Unmodified FFmpeg 8.0.3 source archive")
    p.add_argument("--skip-build", action="store_true")
    p.add_argument("--reuse-python", action="store_true", help="Reuse an already installed locked runtime")
    a = p.parse_args()
    if sys.platform != "darwin" or platform.machine() != "arm64":
        p.error("Build this target on macOS Apple Silicon")
    aisle = a.aisle.resolve()
    if output("git", "-C", aisle, "rev-parse", "HEAD") != CONFIG["aisle"]["revision"]:
        p.error("AISLE must match the pinned release revision")
    if output("git", "-C", aisle, "status", "--porcelain", "--untracked-files=no"):
        p.error("AISLE tracked runtime files must be clean")
    a.out = a.out.resolve()
    a.target = a.target.resolve()
    env = os.environ.copy()
    env.update(CARGO_TARGET_DIR=str(a.target), MAKEPAD_PACKAGE_DIR="../Resources/makepad",
               RUSTFLAGS=f"--remap-path-prefix={Path.home()}=/build -C link-arg=-Wl,-headerpad_max_install_names")
    if not a.skip_build:
        # Makepad embeds CARGO_MANIFEST_DIR in script metadata. Rust path remapping
        # alone cannot remove it, so compile a clean staging copy under /tmp.
        build_root = a.build_root.resolve()
        if str(Path.home()) in str(build_root):
            p.error("Use a build root outside the personal home directory")
        source = build_root / "source"
        if source.exists():
            shutil.rmtree(source)
        source.mkdir(parents=True)
        for name in output("git", "-C", ROOT, "ls-files", "--cached", "--others", "--exclude-standard", "-z").split("\0"):
            if name and (ROOT / name).is_file():
                dest = source / name
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, dest)
        cargo_home = build_root / "cargo"
        cargo_home.mkdir(exist_ok=True)
        original_cargo = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
        for name in ["registry", "git"]:
            link = cargo_home / name
            if not link.exists():
                link.symlink_to(original_cargo / name, target_is_directory=True)
        env["CARGO_HOME"] = str(cargo_home)
        run("cargo", "build", "--offline", "--locked", "--release", "-p", "dorobot-flex", "--bin", "dorobot-flex", cwd=source, env=env)
    app = a.out / "DoRobot Studio.app"
    contents = app / "Contents"
    resources = contents / "Resources"
    binary = contents / "MacOS/dorobot-flex"
    frameworks = contents / "Frameworks"
    for stale in [frameworks, resources / "makepad"]:
        if stale.exists():
            shutil.rmtree(stale)
    for d in [resources, binary.parent, frameworks]:
        d.mkdir(parents=True, exist_ok=True)
    shutil.copy2(a.target / "release/dorobot-flex", binary)
    run("strip", "-x", binary)
    relocate_ffmpeg(binary, frameworks)
    for lib in frameworks.glob("lib*.dylib"):
        name = re.sub(r"\.\d+(?=\.dylib$)", "", lib.name)
        if name != lib.name and not (frameworks / name).exists():
            (frameworks / name).symlink_to(lib.name)
    pkg_config = os.environ.get("PKG_CONFIG", "pkg-config")
    ffmpeg = Path(output(pkg_config, "--variable=prefix", "libavutil"))
    headers = resources / "runtime/ffmpeg"
    shutil.copytree(ffmpeg / "include", headers / "include", dirs_exist_ok=True)
    pcdir = headers / "lib/pkgconfig"
    pcdir.mkdir(parents=True, exist_ok=True)
    for pc in (ffmpeg / "lib/pkgconfig").glob("*.pc"):
        lines = pc.read_text().splitlines()
        replacements = {"prefix": "${pcfiledir}/../..", "libdir": "${pcfiledir}/../../../../../Frameworks", "includedir": "${prefix}/include"}
        (pcdir / pc.name).write_text("\n".join(
            key + "=" + replacements[key] if (key := line.split("=", 1)[0]) in replacements else line
            for line in lines) + "\n")
    tracked(ROOT, resources / "studio", ["data", "tools/aisle-validation", "tools/capture", "robots/adora_nano_feetech"])
    # Tests/docs are small and useful for diagnosing an RC; never copy ignored runs.
    tracked(aisle, resources / "aisle", ["src/aisle", "pyproject.toml", "uv.lock", "README.md", "LICENSE", "LICENSE.md"])
    meta = json.loads(output("cargo", "metadata", "--locked", "--format-version", "1", "--manifest-path", ROOT / "Cargo.toml"))
    notices = resources / "ThirdPartyNotices"
    notices.mkdir(exist_ok=True)
    archive = a.ffmpeg_source.resolve()
    expected = "6136812ea6d4e68bdba27e33c2a94382711cdf4f8602ffef056ff792bd6f9818"
    if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
        p.error("FFmpeg source does not match the pinned 8.0.3 release")
    shutil.copy2(archive, notices / "ffmpeg-8.0.3.tar.xz")
    shutil.copy2(ROOT / "tools/release/build_ffmpeg.sh", notices / "build_ffmpeg.sh")
    (notices / "README.txt").write_text(
        "Python distribution licenses are included in each package's dist-info directory and Python LICENSE.\n"
        "Rust package license texts and metadata are in rust/ and rust-packages.json.\n"
        "FFmpeg 8.0.3 is dynamically linked; its complete unmodified source, license texts and build recipe are included here.\n"
        "AISLE source is included at the pinned revision in ../aisle. That revision has no top-level license declaration; no license is inferred or added.\n"
        "Studio workspace metadata declares MIT. Nano resource provenance is in ../studio/data/adora_nano/provenance.json.\n")
    packages = []
    for pkg in meta["packages"]:
        folder = Path(pkg["manifest_path"]).parent
        name = pkg["name"].replace("-", "_")
        if (folder / "resources").is_dir():
            shutil.copytree(folder / "resources", resources / "makepad" / name / "resources", dirs_exist_ok=True,
                            ignore=shutil.ignore_patterns(".DS_Store", "__pycache__"))
        packages.append({"name": pkg["name"], "version": pkg["version"], "license": pkg.get("license"), "source": pkg.get("source")})
        license_dir = notices / "rust" / f'{pkg["name"]}-{pkg["version"]}'
        candidates = list(folder.glob("LICENSE*")) + list(folder.glob("COPYING*"))
        if pkg.get("license_file"):
            candidates.append(folder / pkg["license_file"])
        for f in candidates:
            if f.is_file():
                license_dir.mkdir(parents=True, exist_ok=True)
                shutil.copy2(f, license_dir / f.name)
    # Splash theme modules have synthetic crate names but use widgets' files.
    # Keep aliases in Resources: macOS treats every file in MacOS/ as code.
    for theme in ["ios", "ios_dark"]:
        for phase in ["theme", "roles", "widgets"]:
            (resources / "makepad" / f"desktop_style_{theme}_{phase}").symlink_to("makepad_widgets", target_is_directory=True)
    for old in [binary.parent / "resources", binary.parent / "dorobot-flex.makepad-package-paths"]:
        if old.exists() or old.is_symlink():
            old.unlink()
    (notices / "rust-packages.json").write_text(json.dumps(packages, indent=2) + "\n")
    pyhome = resources / "runtime/python"
    if not (a.reuse_python and (pyhome / "bin/python3").exists()):
        if pyhome.exists():
            shutil.rmtree(pyhome)
        source = a.python_home or Path(output(a.uv, "python", "find", CONFIG["python"])).resolve().parent.parent
        shutil.copytree(source, pyhome, symlinks=True, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        python = pyhome / "bin/python3"
        run(a.uv, "pip", "install", "--python", python, "--system", "--break-system-packages", "--no-config", "--only-binary", ":all:",
            "--require-hashes", "-r", ROOT / "tools/release/requirements.lock")
    for config in (pyhome / "lib/python3.13").glob("_sysconfigdata_*.py"):
        values = runpy.run_path(str(config))["build_time_vars"]
        prefix = values["prefix"]
        values = {k: v.replace(prefix, "__DOROBOT_RUNTIME_PREFIX__") if isinstance(v, str) else v for k, v in values.items()}
        config.write_text("# Relocatable Python build configuration.\nimport sys\nbuild_time_vars = " + repr(values) +
                          "\nbuild_time_vars = {k: v.replace('__DOROBOT_RUNTIME_PREFIX__', sys.base_prefix) if isinstance(v, str) else v for k, v in build_time_vars.items()}\n")
    # Installed console scripts contain build paths. Keep only the interpreter;
    # Dora gets a relative launcher, and other tools are run as Python modules.
    for f in (pyhome / "bin").iterdir():
        if f.name not in {"python", "python3", "python3.13"}:
            f.unlink()
    runtime_bin = resources / "runtime/bin"
    runtime_bin.mkdir(exist_ok=True)
    dora = runtime_bin / "dora"
    dora.write_text('#!/bin/sh\nROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)\nexec "$ROOT/python/bin/python3" -I -B -c "from dora_cli import py_main; py_main()" "$@"\n')
    dora.chmod(0o755)
    for f in resources.rglob("__pycache__"):
        shutil.rmtree(f)
    for f in resources.rglob("*.pyc"):
        f.unlink()
    # Build metadata/config helpers carry absolute configure prefixes; not used at runtime.
    for f in (pyhome / "lib").glob("pkgconfig/*"):
        f.unlink()
    python_packages = output(pyhome / "bin/python3", "-I", "-B", "-c", "import importlib.metadata as m,json; print(json.dumps(sorted([{'name':d.metadata['Name'],'version':d.version} for d in m.distributions()],key=lambda d:d['name']),indent=2))")
    (notices / "python-packages.json").write_text(python_packages + "\n")
    for f in ["requirements.lock", "release-config.json"]:
        shutil.copy2(ROOT / "tools/release" / f, resources / f)
    if (ROOT / "LICENSE").is_file():
        shutil.copy2(ROOT / "LICENSE", resources / "LICENSE")
    with (contents / "Info.plist").open("wb") as stream:
        plistlib.dump({"CFBundleExecutable": "dorobot-flex", "CFBundleIdentifier": "org.dorobot.studio",
                      "CFBundleName": "DoRobot Studio", "CFBundleDisplayName": "DoRobot Studio",
                      "CFBundlePackageType": "APPL", "CFBundleShortVersionString": "0.1.0",
                      "CFBundleVersion": "1", "LSMinimumSystemVersion": "14.0", "NSHighResolutionCapable": True,
                      "NSCameraUsageDescription": "Capture robot camera observations when you start a real recording.",
                      "NSBluetoothAlwaysUsageDescription": "Use your paired game controller to operate Nano.",
                      "NSMicrophoneUsageDescription": "Optional capture device access."}, stream)
    release = dict(CONFIG, studio_revision=output("git", "-C", ROOT, "rev-parse", "HEAD"),
                   studio_dirty=bool(output("git", "-C", ROOT, "status", "--porcelain", "--untracked-files=no")),
                   signing="ad-hoc; not notarized", scope="Nano AISLE/Nexus simulation, three cameras, Dora recording/replay and Feetech base adapter")
    (resources / "release.json").write_text(json.dumps(release, indent=2) + "\n")
    # Re-sign every bundled native extension after copying/relocation, inside out.
    for f in sorted(app.rglob("*"), key=lambda f: len(f.parts), reverse=True):
        if macho(f):
            if f.suffix == ".dylib":
                run("install_name_tool", "-id", "@rpath/" + f.name, f,
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for dep in dependencies(f):
                if dep.startswith("/") and not dep.startswith(("/usr/lib/", "/System/Library/")):
                    raise RuntimeError(f"Nonportable dependency in {f.relative_to(app)}: {dep}")
            run("codesign", "--force", "--sign", "-", f, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    run("codesign", "--force", "--sign", "-", app)
    run("codesign", "--verify", "--deep", "--strict", app)
    a.out.mkdir(exist_ok=True)
    print(json.dumps({"app": str(app), "version": CONFIG["version"], "signing": release["signing"]}))


if __name__ == "__main__":
    main()
