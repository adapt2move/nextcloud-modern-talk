#!/usr/bin/env python3
"""Build/package a selected CLEAN candidate. Never run on an active shared checkout.

Uses only the repository toolchain/lock and default-free talk-desktop release target.
Native dependency notices fail closed when absent. This is not a license approval.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib

from package import TARGETS, assemble, sha, relative, regular

MAC_GSTREAMER_LIB = Path("/Library/Frameworks/GStreamer.framework/Versions/1.0/lib")


def macos_linkage(linked, load_commands, runtime_lib=MAC_GSTREAMER_LIB):
    """Resolve non-system install names against the declared runtime, not Homebrew.

    The official GStreamer pkg-config libraries use @rpath/lib*.dylib rather
    than the framework install name. A matching name alone is insufficient:
    the executable must carry the runtime search path and the library must
    exist inside that verified framework installation.
    https://gstreamer.freedesktop.org/documentation/installing/on-mac-osx.html
    """
    rpaths = re.findall(r"\bcmd LC_RPATH\s+cmdsize \d+\s+path (.+?) \(offset \d+\)", load_commands)
    libraries = [line.strip() for line in linked]
    for line in libraries:
        name = line.split(" (compatibility version ", 1)[0]
        if ".." not in Path(name).parts and name.startswith(("/System/Library/", "/usr/lib/")):
            continue
        if name.startswith("@rpath/") and str(runtime_lib) in rpaths:
            basename = name.removeprefix("@rpath/")
            if re.fullmatch(r"lib[a-zA-Z0-9_.+-]+\.dylib", basename):
                library = runtime_lib / basename
                if library.is_file() and library.resolve().is_relative_to(runtime_lib.resolve()):
                    continue
        raise ValueError("Undeclared or unresolved macOS dynamic library: " + line)
    return {"libraries": libraries, "rpaths": rpaths}


def run(argv, root, env=None, timeout=60):
    try:
        return subprocess.run(argv, cwd=root, env=env, check=True, capture_output=True, text=True, timeout=timeout).stdout.strip()
    except subprocess.CalledProcessError as error:
        # Keep useful compiler/packager diagnostics instead of only a Python stack.
        details = ((error.stdout or "") + "\n" + (error.stderr or ""))[-16000:]
        raise RuntimeError(argv[0] + " failed with exit " + str(error.returncode) + ":\n" + details) from error


def snapshot(root):
    if run(["git", "status", "--porcelain", "--untracked-files=all"], root):
        raise ValueError("Release build requires a clean committed checkout")
    paths = run(["git", "ls-files", "-z"], root).split("\0")
    files = {}
    for name in paths:
        if not name:
            continue
        relative(name)
        p = root / name
        if p.is_symlink():
            # Repository skill aliases are source links, never package executables.
            if not p.resolve(strict=True).is_relative_to(root):
                raise ValueError("Source symlink leaves the checkout")
            files[name] = {"symlink": os.readlink(p)}
        elif p.is_file():
            files[name] = sha(p)
        else:
            raise ValueError("Submodules are not supported")
    return files


def notice_supplement(root, package, checksum):
    """Use checked-in upstream notices only for the exact published crate."""
    prefix = "tools/release/notices/"
    if not (root / prefix / "manifest.json").exists():
        return None
    manifest = json.loads(regular(root, prefix + "manifest.json").read_text())
    if manifest.get("schema") != 1:
        raise ValueError("Unknown dependency notice supplement schema")
    matches = [entry for entry in manifest["packages"]
               if (entry["name"], entry["version"]) == (package["name"], package["version"])]
    if not matches:
        return None
    if len(matches) != 1:
        raise ValueError("Ambiguous dependency notice supplement")
    entry = matches[0]
    if (not (package.get("source") or "").startswith("registry+") or not checksum
            or entry["crate_sha256"] != checksum or entry["license"] != package["license"]):
        raise ValueError("Dependency notice supplement identity differs from published crate")
    files = set()
    for item in entry["files"]:
        rel = relative(item["path"])
        if len(rel.parts) < 2 or rel.parts[0] != package["name"] + "-" + package["version"]:
            raise ValueError("Dependency notice supplement leaves its package directory")
        path = regular(root, prefix + rel.as_posix())
        if path in files or path.stat().st_size > 2 * 1024 * 1024 or sha(path) != item["sha256"]:
            raise ValueError("Dependency notice supplement is duplicate, oversized or has changed")
        files.add(path)
    if not files:
        raise ValueError("Empty dependency notice supplement")
    return entry, files


def dependency_notices(root, stage, target, env):
    # Keep host build dependencies in metadata even when collecting for another
    # target. The shipping cargo-tree closure below is the selection authority.
    data = json.loads(run(["cargo", "metadata", "--locked", "--format-version", "1", "--no-default-features"], root, env, 300))
    packages = {p["id"]: p for p in data["packages"]}
    # cargo metadata unifies workspace dev features. cargo tree on the actual
    # shipping root, excluding dev edges, supplies the compiled feature closure.
    tree = run(["cargo", "tree", "--locked", "-p", "talk-desktop", "--target", target,
                "--edges", "normal,build", "--prefix", "none", "--no-dedupe", "--format", "{p}|{f}"], root, env, 300)
    selected = {}
    for line in tree.splitlines():
        match = re.fullmatch(r"([^ ]+) v([^ |]+)(?: [^|]*)?\|([^|]*)", line)
        if not match:
            raise ValueError("Unexpected cargo tree output: " + line)
        matches = [k for k, p in packages.items() if (p["name"], p["version"]) == match.group(1, 2)]
        if len(matches) != 1:
            raise ValueError("Ambiguous shipping dependency identity: " + match[1] + " " + match[2])
        selected.setdefault(matches[0], set()).update(filter(None, match[3].split(",")))
    lock = tomllib.loads((root / "Cargo.lock").read_text())
    sums = {(p["name"], p["version"], p.get("source")): p.get("checksum") for p in lock["package"]}
    cargo_home = Path(env.get("CARGO_HOME", Path.home() / ".cargo"))
    inventory = {"schema": 1, "target": target, "lock_sha256": sha(root / "Cargo.lock"),
                 "scope": "desktop resolved closure including build dependencies; notice collection, not legal or vulnerability clearance", "packages": []}
    for ident in sorted(selected):
        p = packages[ident]
        features = sorted(selected[ident])
        if "backend-test" in features:
            raise ValueError("Diagnostic backend-test feature is reachable in shipping graph")
        checksum = sums.get((p["name"], p["version"], p.get("source")))
        if p.get("source"):
            if not p["source"].startswith("registry+") or not checksum:
                raise ValueError("Unqualified non-registry dependency")
            archives = list((cargo_home / "registry/cache").glob(f"*/{p['name']}-{p['version']}.crate"))
            if not archives or any(sha(a) != checksum for a in archives):
                raise ValueError("Cached crate missing or checksum differs from lock: " + p["name"])
        if not p.get("license"):
            raise ValueError("Missing declared license: " + p["name"])
        package_dir = Path(p["manifest_path"]).parent.resolve()
        notice_files = set()
        for path in package_dir.rglob("*"):
            rel = path.relative_to(package_dir)
            if len(rel.parts) <= 3 and path.is_file() and re.match(r"^(license|licence|copying|notice|authors|copyright)([._-]|$)", path.name, re.I):
                notice_files.add(path)
        if p.get("license_file"):
            notice_files.add(package_dir / p["license_file"])
        if not p.get("source"):
            notice_files.add(root / "LICENSE")
        supplement = None
        supplement_files = set()
        if not notice_files:
            supplemented = notice_supplement(root, p, checksum)
            if supplemented:
                supplement, supplement_files = supplemented
                notice_files = supplement_files
        if not notice_files:
            raise ValueError("Missing license text; resolve notice closure explicitly: " + p["name"])
        notices = []
        for index, path in enumerate(sorted(notice_files)):
            resolved = path.resolve()
            if path.is_symlink() or not (resolved.is_relative_to(package_dir) or resolved == root / "LICENSE" or path in supplement_files) or not path.is_file() or path.stat().st_size > 2 * 1024 * 1024:
                raise ValueError("Unsafe or oversized dependency notice")
            dest = Path("licenses/dependencies") / (p["name"] + "-" + p["version"]) / (str(index) + "-" + path.name)
            (stage / dest).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, stage / dest)
            notices.append(dest.as_posix())
        entry = {"name": p["name"], "version": p["version"], "license": p["license"], "source": p.get("source"),
                 "archive_sha256": checksum, "features": features, "notices": notices}
        if supplement:
            entry["notice_supplement"] = supplement
        inventory["packages"].append(entry)
    (stage / "dependencies.json").write_text(json.dumps(inventory, indent=2, sort_keys=True) + "\n")


def native_engine(root, stage, target, env):
    """Validate the exact SDK before building and ship its complete native notices."""
    spec = importlib.util.spec_from_file_location("nmt_native_sdk", root / "tools/media/prepare_native.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    native_target = {"x86_64-unknown-linux-gnu": "linux-x64-release", "aarch64-apple-darwin": "mac-arm64-release"}[target]
    configured = Path(env["LK_CUSTOM_WEBRTC"]).resolve()
    verified = module.prepare(native_target, configured.parent)
    if verified != configured:
        raise ValueError("Native SDK path differs from verified input")
    notice = verified / "LICENSE.md"
    destination = stage / "licenses/native-libwebrtc-LICENSE.md"
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(notice, destination)
    record = json.loads((verified / ".nmt-native.json").read_text())
    (stage / "native-media.json").write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
    return {"enabled": True, "engine": "libwebrtc", "version": "0.3.52", "sdk_tag": module.TAG,
            "sdk_target": native_target, "sdk_sha256": record["archive_sha256"],
            "notice": "licenses/native-libwebrtc-LICENSE.md"}


def build(root, output, target, version):
    root, output = root.resolve(), output.resolve()
    if output == root or root in output.parents or output.exists():
        raise ValueError("Use a new output directory outside the checkout")
    before = snapshot(root)
    toolchain = tomllib.loads((root / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", toolchain):
        raise ValueError("Exact Rust toolchain required")
    policy = json.loads((root / "tools/release/package-policy.json").read_text())
    if policy.get("schema") != 1 or policy.get("composition") not in ("unconfigured", "account-connection"):
        raise ValueError("Unknown application composition")
    if policy["media"].get("enabled") and policy["media"].get("engine") != "libwebrtc":
        raise ValueError("Only the maintained production engine is distributable")
    workspace_version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if version.split("-")[0] != workspace_version.split("-")[0]:
        raise ValueError("Requested package version disagrees with canonical Cargo version")
    commit = run(["git", "rev-parse", "HEAD"], root)
    source = {"commit": commit, "tree": run(["git", "rev-parse", "HEAD^{tree}"], root),
              "epoch": int(run(["git", "show", "-s", "--format=%ct", "HEAD"], root)),
              "lock_sha256": before["Cargo.lock"], "toolchain_sha256": before["rust-toolchain.toml"]}
    with tempfile.TemporaryDirectory(prefix="nmt-release-build-") as tmp:
        tmp = Path(tmp)
        env = dict(os.environ, RUSTUP_TOOLCHAIN=toolchain, CARGO_TARGET_DIR=str(tmp / "target"), SOURCE_DATE_EPOCH=str(source["epoch"]))
        # Do not inherit environment feature/build-output overrides from a developer shell.
        for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
            if env.get(key):
                raise ValueError("Unreviewed build override: " + key)
        if target == "aarch64-apple-darwin":
            if run(["uname", "-sm"], root) != "Darwin arm64":
                raise ValueError("macOS arm64 package must be built on native macOS arm64")
            env["MACOSX_DEPLOYMENT_TARGET"] = "15.7"
            # pkg-config supplies -L for linking but not the runtime search path.
            # Pin the same external framework location documented by the bundle.
            env["RUSTFLAGS"] = "-C link-arg=-Wl,-rpath," + str(MAC_GSTREAMER_LIB)
            for tool in ("metal", "metallib"):
                run(["xcrun", "--sdk", "macosx", "--find", tool], root)
        elif run(["uname", "-sm"], root) != "Linux x86_64":
            raise ValueError("Linux package must be built on native Linux x86_64")
        stage = tmp / "input"
        stage.mkdir()
        native = None
        if policy["media"].get("enabled"):
            native = native_engine(root, stage, target, env)
        dependency_notices(root, stage, target, env)
        run(["cargo", "build", "--locked", "--release", "--no-default-features", "-p", "talk-desktop", "--target", target], root, env, 3600)
        executable = tmp / "target" / target / "release/talk-desktop"
        (stage / "bin").mkdir()
        shutil.copyfile(executable, stage / "bin/talk-desktop")
        if target == "aarch64-apple-darwin":
            run(["python3", "tools/native/build-macos-helper.py", "--output",
                 str(stage / "bin/talk-native-helper")], root, env, 300)
        copies = {"LICENSE": "licenses/project/LICENSE", "NOTICE.md": "licenses/project/NOTICE.md",
                  "apps/desktop/NOTICE.md": "licenses/desktop/NOTICE.md",
                  "apps/desktop/assets/fonts/Manrope-LICENSE.txt": "licenses/manrope/Manrope-LICENSE.txt",
                  "apps/desktop/assets/GPUI-LICENSE-APACHE": "licenses/gpui/GPUI-LICENSE-APACHE"}
        for src, dest in copies.items():
            (stage / dest).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(root / src, stage / dest)
        for path in (root / "apps/desktop/assets").rglob("*"):
            if path.is_file():
                dest = stage / "assets" / path.relative_to(root / "apps/desktop/assets")
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, dest)
        (stage / "source-manifest.json").write_text(json.dumps(before, sort_keys=True, indent=2) + "\n")
        source["manifest_sha256"] = sha(stage / "source-manifest.json")
        build_info = {"package": "talk-desktop", "profile": "release", "features": [],
                      "rustc": run(["rustc", "--version", "--verbose"], root, env), "cargo": run(["cargo", "--version"], root, env)}
        if target.startswith("x86_64-"):
            debian = tmp / "debian"
            debian.mkdir()
            (debian / "control").write_text("Source: nextcloud-modern-talk\nSection: net\nPriority: optional\nMaintainer: Nextcloud Modern Talk contributors\n\nPackage: nextcloud-modern-talk\nArchitecture: any\nDescription: Native Talk development candidate\n")
            deps = run(["dpkg-shlibdeps", "-O", "-e" + str(executable)], tmp)
            # GPUI/provider loaders can open libraries not visible in ELF NEEDED.
            # Keep explicit runtime prerequisites alongside dpkg's symbol-derived set.
            build_info["runtime_packages"] = ["libvulkan1", "libxkbcommon-x11-0", "libfontconfig1", "libfreetype6", "libx11-xcb1", "libxcb1", "libdbus-1-3", "libpulse0", "libgstreamer1.0-0", "gstreamer1.0-plugins-base", "gstreamer1.0-plugins-good", "xdg-desktop-portal", "xdg-utils", "desktop-file-utils", "libnotify-bin", "libglib2.0-bin", "libxfixes3", "libxdamage1", "libxrandr2", "libxcomposite1", "libgbm1"]
            inferred = next(line.split("=", 1)[1] for line in deps.splitlines() if line.startswith("shlibs:Depends="))
            build_info["debian_depends"] = inferred + ", " + ", ".join(build_info["runtime_packages"])
            build_info["system_libraries"] = run(["objdump", "-p", str(executable)], root).splitlines()
            build_info["system_libraries"] = [line.strip() for line in build_info["system_libraries"] if "NEEDED" in line]
        else:
            build_info["xcode"] = run(["xcodebuild", "-version"], root)
            build_info["sdk"] = run(["xcrun", "--sdk", "macosx", "--show-sdk-version"], root)
            build_info["native_helper"] = {
                "source_sha256": before["native/macos/TalkNativeHelper.swift"],
                "builder_sha256": before["tools/native/build-macos-helper.py"],
                "sha256": sha(stage / "bin/talk-native-helper"),
                "minimum_system_version": "15.7",
            }
            build_info["native_linkage"] = {}
            for binary in (executable, stage / "bin/talk-native-helper"):
                linked = run(["otool", "-L", str(binary)], root).splitlines()[1:]
                commands = run(["otool", "-l", str(binary)], root)
                build_info["native_linkage"][binary.name] = macos_linkage(linked, commands)
            build_info["rustflags"] = env["RUSTFLAGS"]
            build_info["required_runtime"] = {
                "name": "GStreamer official runtime framework", "version": "1.28.4",
                "url": "https://gstreamer.freedesktop.org/data/pkg/osx/1.28.4/gstreamer-1.0-1.28.4-universal.pkg",
                "sha256": "9f9ae8ba1a25eb42d6740c4e5f15b09ee6b9127b7359e9662c45a2b9a5029f85",
            }
        if before != snapshot(root):
            raise ValueError("Source changed while building")
        spec = {"schema": 1, "version": version, "kind": "development", "target": target, "source": source,
                "build": build_info, "media": native or policy["media"], "files": {p.relative_to(stage).as_posix(): sha(p) for p in sorted(stage.rglob("*")) if p.is_file()}}
        (stage / "input.json").write_text(json.dumps(spec, indent=2, sort_keys=True) + "\n")
        report = assemble(stage, output)
        base = next(output.glob("*.provenance.json")).name.removesuffix(".provenance.json")
        source_tar = output / (base + ".source.tar.gz")
        subprocess.run(["git", "archive", "--format=tar.gz", "--prefix=nextcloud-modern-talk/", "-o", str(source_tar), commit], cwd=root, check=True, timeout=120)
        report["assets"][source_tar.name] = {"sha256": sha(source_tar), "bytes": source_tar.stat().st_size}
        (output / (base + ".provenance.json")).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        (output / (base + ".SHA256SUMS")).write_text("".join(f"{sha(p)}  {p.name}\n" for p in sorted(output.iterdir()) if not p.name.endswith(".SHA256SUMS")))
    return report


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--target", choices=TARGETS, required=True)
    p.add_argument("--version", required=True)
    a = p.parse_args()
    print(json.dumps(build(a.root, a.output, a.target, a.version), indent=2))
