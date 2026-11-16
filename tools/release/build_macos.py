#!/usr/bin/env python3
"""Build a relocatable Nano RC; build-time tools are not needed by recipients.

Requires macOS arm64, Rust, cargo-gpu, a native Dora CLI, and FFmpeg
headers/libraries exposed through PKG_CONFIG_PATH. All subprocesses use argv.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
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
    p.add_argument("--dora", type=Path, required=True, help="Native dora-cli 1.0.1 binary")
    p.add_argument("--target", type=Path, default=ROOT / "target/rc")
    p.add_argument("--out", type=Path, default=ROOT / "dist")
    p.add_argument("--build-root", type=Path, default=Path("/tmp/dorobot-rc-build"))
    p.add_argument("--ffmpeg-source", type=Path, required=True, help="Unmodified FFmpeg 8.0.3 source archive")
    p.add_argument("--skip-build", action="store_true")
    a = p.parse_args()
    if sys.platform != "darwin" or platform.machine() != "arm64":
        p.error("Build this target on macOS Apple Silicon")
    if output(a.dora, "--version") != "dora-cli 1.0.1" or not macho(a.dora):
        p.error("Provide the native dora-cli 1.0.1 executable")
    a.out = a.out.resolve()
    a.target = a.target.resolve()
    env = os.environ.copy()
    # A compile-check shortcut must never produce a distributable without shaders.
    env.pop("KHAL_SKIP_SPIRV", None)
    env.update(CARGO_TARGET_DIR=str(a.target), MAKEPAD_PACKAGE_DIR="../Resources/makepad", MACOSX_DEPLOYMENT_TARGET="14.0",
               RUSTFLAGS=f"--remap-path-prefix={Path.home()}=/build -C link-arg=-Wl,-headerpad_max_install_names",
               RUSTGPU_RUSTFLAGS=f"--remap-path-prefix={Path.home()}=/build")
    if not a.skip_build:
        if output("cargo", "gpu", "--version").split()[:2] != ["cargo-gpu", "0.10.0"]:
            p.error("Install the pinned shader launcher: cargo install cargo-gpu --version 0.10.0 --locked --no-default-features")
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
        run("cargo", "build", "--offline", "--locked", "--release", "--manifest-path", source / "runtime/Cargo.toml", cwd=source, env=env)
    app = a.out / "DoRobot Studio.app"
    contents = app / "Contents"
    resources = contents / "Resources"
    binary = contents / "MacOS/dorobot-flex"
    frameworks = contents / "Frameworks"
    for stale in [frameworks, resources / "makepad", resources / "runtime", resources / "aisle", resources / "studio", resources / "ThirdPartyNotices"]:
        if stale.exists():
            shutil.rmtree(stale)
    for d in [resources, binary.parent, frameworks]:
        d.mkdir(parents=True, exist_ok=True)
    shutil.copy2(a.target / "release/dorobot-flex", binary)
    run("strip", "-x", binary)
    relocate_ffmpeg(binary, frameworks)
    runtime_bin = resources / "runtime/bin"
    runtime_bin.mkdir(parents=True)
    native = runtime_bin / "dorobot-nano-runtime"
    shutil.copy2(a.target / "release/dorobot-nano-runtime", native)
    shutil.copy2(a.dora, runtime_bin / "dora")
    for executable in [native, runtime_bin / "dora"]:
        run("strip", "-x", executable)
        relocate_ffmpeg(executable, frameworks)
        run("install_name_tool", "-delete_rpath", "@executable_path/../Frameworks", executable)
        run("install_name_tool", "-add_rpath", "@executable_path/../../../Frameworks", executable)

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
    tracked(ROOT, resources / "studio", ["data"])
    profile = resources / "studio/robots/adora_nano_feetech"
    profile.mkdir(parents=True)
    shutil.copy2(ROOT / "robots/adora_nano_feetech/profile.json", profile / "profile.json")
    meta = json.loads(output("cargo", "metadata", "--offline", "--locked", "--filter-platform", "aarch64-apple-darwin", "--format-version", "1", "--manifest-path", ROOT / "Cargo.toml"))
    native_meta = json.loads(output("cargo", "metadata", "--offline", "--locked", "--filter-platform", "aarch64-apple-darwin", "--format-version", "1", "--manifest-path", ROOT / "runtime/Cargo.toml"))
    seen = {pkg["id"] for pkg in meta["packages"]}
    meta["packages"].extend(pkg for pkg in native_meta["packages"] if pkg["id"] not in seen)
    notices = resources / "ThirdPartyNotices"
    notices.mkdir(exist_ok=True)
    archive = a.ffmpeg_source.resolve()
    expected = "6136812ea6d4e68bdba27e33c2a94382711cdf4f8602ffef056ff792bd6f9818"
    if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
        p.error("FFmpeg source does not match the pinned 8.0.3 release")
    shutil.copy2(archive, notices / "ffmpeg-8.0.3.tar.xz")
    shutil.copy2(ROOT / "tools/release/build_ffmpeg.sh", notices / "build_ffmpeg.sh")
    (notices / "README.txt").write_text(
        "Rust package license texts and metadata are in rust/ and rust-packages.json.\n"
        "FFmpeg 8.0.3 is dynamically linked; its complete unmodified source, license texts and build recipe are included here.\n"
        "The AISLE Nano scene integration is ported to Rust. Pinned Nexus, Khal and Vortx sources and provenance are in the source release. No Python runtime or bindings are included.\n"
        "Studio workspace metadata declares MIT. Nano resource provenance is in ../studio/data/adora_nano/provenance.json.\n")
    packages = []
    for pkg in meta["packages"]:
        folder = Path(pkg["manifest_path"]).parent
        name = pkg["name"].replace("-", "_")
        if (name in ("makepad", "dorobot_flex") or name.startswith("makepad_")) and (folder / "resources").is_dir():
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
    shutil.copy2(ROOT / "tools/release/release-config.json", resources / "release-config.json")
    shutil.copy2(ROOT / "runtime/vendor/provenance.json", notices / "native-engine-provenance.json")
    shutil.copytree(ROOT / "runtime/notices", notices / "native-port", dirs_exist_ok=True)
    # Vendored crates inherit workspace licenses outside their crate directory.
    for engine in ["nexus", "khal", "vortx"]:
        target = notices / "native-engines" / engine
        target.mkdir(parents=True)
        for license in (ROOT / "runtime/vendor" / engine).glob("LICENSE*"):
            shutil.copy2(license, target / license.name)
    for stale in [resources / "requirements.lock"]:
        stale.unlink(missing_ok=True)
    if (ROOT / "LICENSE").is_file():
        shutil.copy2(ROOT / "LICENSE", resources / "LICENSE")
    with (contents / "Info.plist").open("wb") as stream:
        plistlib.dump({"CFBundleExecutable": "dorobot-flex", "CFBundleIdentifier": "org.dorobot.studio",
                      "CFBundleName": "DoRobot Studio", "CFBundleDisplayName": "DoRobot Studio",
                      "CFBundlePackageType": "APPL", "CFBundleShortVersionString": "0.1.0",
                      "CFBundleVersion": "2", "LSMinimumSystemVersion": "14.0", "NSHighResolutionCapable": True,
                      "NSCameraUsageDescription": "Capture robot camera observations when you start a real recording.",
                      "NSBluetoothAlwaysUsageDescription": "Use your paired game controller to operate Nano.",
                      "NSMicrophoneUsageDescription": "Optional capture device access."}, stream)
    release = dict(CONFIG, studio_revision=output("git", "-C", ROOT, "rev-parse", "HEAD"),
                   studio_dirty=bool(output("git", "-C", ROOT, "status", "--porcelain", "--untracked-files=no")),
                   signing="ad-hoc; not notarized", scope="Rust Nano/Nexus simulation, three cameras, native Dora, recording/replay, Feetech adapters and Hub transfers", runtime_language="Rust", python_runtime=False)
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
