#!/usr/bin/env python3
"""Assemble versioned development packages from a closed, hash-bound input directory.

No build, network access, helper discovery or credential access occurs here.
build.py creates inputs on a selected clean candidate; tests create labelled fixtures.
This is packaging integrity, not proof of compilation, license clearance or runtime support.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import tarfile
import tempfile
import zipfile


TARGETS = {"x86_64-unknown-linux-gnu": "linux-x86_64", "aarch64-apple-darwin": "macos-arm64-unsigned"}
REQUIRED_NOTICES = (
    "licenses/project/LICENSE", "licenses/project/NOTICE.md", "licenses/desktop/NOTICE.md",
    "licenses/manrope/Manrope-LICENSE.txt", "licenses/gpui/GPUI-LICENSE-APACHE",
)


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def digest(value, length=64):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{%d}" % length, value):
        raise ValueError("Invalid source or artifact digest")
    return value


def relative(value):
    if not isinstance(value, str) or not value or "\\" in value or any(ord(c) < 32 for c in value):
        raise ValueError("Invalid package path")
    p = PurePosixPath(value)
    if p.is_absolute() or any(x in (".", "..") for x in value.split("/")) or str(p) != value:
        raise ValueError("Package path escapes or is not canonical")
    return p


def regular(root, name):
    p = root
    for part in relative(name).parts:
        p = p / part
        if p.is_symlink():
            raise ValueError("Symlinks are not accepted as package inputs")
    if not p.is_file() or p.stat().st_nlink != 1:
        raise ValueError("Expected an unlinked regular input file: " + name)
    return p


def native_target(path, target):
    with path.open("rb") as f:
        header = f.read(32)
    if target == "x86_64-unknown-linux-gnu":
        valid = len(header) == 32 and header[:6] == b"\x7fELF\x02\x01" and struct.unpack_from("<H", header, 18)[0] == 62
    else:
        valid = len(header) == 32 and header[:4] == b"\xcf\xfa\xed\xfe" and struct.unpack_from("<I", header, 4)[0] == 0x0100000C
    if not valid:
        raise ValueError("Executable format/architecture does not match target")


def media_metadata(packages):
    """Bind the engine to the actual Cargo shipping closure, not a second SDK."""
    if any(p["name"] in ("libwebrtc", "webrtc-sys", "webrtc-sys-build") for p in packages):
        raise ValueError("Obsolete native WebRTC SDK in the shipping dependency closure")
    required = ("webrtc", "rtc", "gstreamer", "gstreamer-app", "gstreamer-video")
    selected = {}
    for name in required:
        matches = [p for p in packages if p["name"] == name]
        if len(matches) != 1:
            raise ValueError("Missing or ambiguous media dependency: " + name)
        package = matches[0]
        if not (package.get("source") or "").startswith("registry+"):
            raise ValueError("Media dependency must use its pinned registry package: " + name)
        digest(package.get("archive_sha256"))
        selected[name] = package
    engine = selected["webrtc"]
    return {"enabled": True, "engine": "webrtc", "version": engine["version"],
            "archive_sha256": engine["archive_sha256"], "codecs": "gstreamer"}


def validate(stage, allow_fixture=False):
    stage = Path(stage).resolve()
    spec = json.loads(regular(stage, "input.json").read_text())
    if spec.get("schema") != 1 or spec.get("target") not in TARGETS:
        raise ValueError("Unsupported package schema/target")
    kind = spec.get("kind")
    if kind != "development" and not (allow_fixture and kind == "packaging-test-fixture"):
        raise ValueError("Only development inputs (or explicitly labelled test fixtures) are supported")
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[a-z0-9][a-z0-9.-]*)?", spec.get("version", "")):
        raise ValueError("Version must be a safe explicit SemVer without build metadata")
    source = spec["source"]
    digest(source["commit"], 40)
    digest(source["tree"], 40)
    for key in ("manifest_sha256", "lock_sha256", "toolchain_sha256"):
        digest(source[key])
    if type(source.get("epoch")) is not int or not 315532800 <= source["epoch"] <= 4102444799:
        raise ValueError("Source epoch is outside supported archive range")
    build = spec["build"]
    if build.get("profile") != "release" or build.get("features") != [] or build.get("package") != "talk-desktop":
        raise ValueError("Only the default-free release desktop is distributable; diagnostic feature bridges are forbidden")
    files = spec["files"]
    if not isinstance(files, dict) or len(files) > 20000:
        raise ValueError("Invalid or excessive file inventory")
    actual = set()
    total = 0
    for p in stage.rglob("*"):
        if p.is_symlink():
            raise ValueError("Symlink in input directory")
        if p.is_file() and p.relative_to(stage).as_posix() != "input.json":
            actual.add(p.relative_to(stage).as_posix())
    if actual != set(files):
        raise ValueError("Input files differ from closed manifest")
    for name, expected in files.items():
        p = regular(stage, name)
        total += p.stat().st_size
        if total > 1024 * 1024 * 1024 or sha(p) != digest(expected):
            raise ValueError("Input size or checksum mismatch: " + name)
        if not (name.startswith(("licenses/", "assets/")) or name in ("bin/talk-desktop", "bin/talk-native-helper", "source-manifest.json", "dependencies.json")):
            raise ValueError("Unexpected payload category: " + name)
    for name in (*REQUIRED_NOTICES, "dependencies.json", "source-manifest.json", "assets/fonts/Manrope-Regular.ttf", "assets/fonts/Manrope-SemiBold.ttf", "assets/fonts/Manrope-Bold.ttf"):
        if name not in files:
            raise ValueError("Required notice/provenance/asset missing: " + name)
    if files["source-manifest.json"] != source["manifest_sha256"]:
        raise ValueError("Source manifest binding mismatch")
    source_files = json.loads((stage / "source-manifest.json").read_text())
    if source_files.get("Cargo.lock") != source["lock_sha256"] or source_files.get("rust-toolchain.toml") != source["toolchain_sha256"]:
        raise ValueError("Lock/toolchain do not match source inventory")
    dependencies = json.loads((stage / "dependencies.json").read_text())
    if dependencies.get("target") != spec["target"] or dependencies.get("lock_sha256") != source["lock_sha256"]:
        raise ValueError("Dependency inventory does not match target/lock")
    if not dependencies.get("packages"):
        raise ValueError("Dependency inventory must not be empty")
    for dep in dependencies["packages"]:
        if not dep.get("license") or not dep.get("notices") or "backend-test" in dep.get("features", []):
            raise ValueError("Missing dependency license/notice or forbidden diagnostic feature")
        for notice in dep["notices"]:
            if notice not in files or not notice.startswith("licenses/"):
                raise ValueError("Dependency notice missing from payload")
    native_target(regular(stage, "bin/talk-desktop"), spec["target"])
    native_helper = build.get("native_helper")
    if native_helper is not None or "bin/talk-native-helper" in files:
        if spec["target"] != "aarch64-apple-darwin" or not isinstance(native_helper, dict):
            raise ValueError("Native settings helper requires an explicit macOS build binding")
        if (native_helper.get("minimum_system_version") != "15.7"
                or files.get("bin/talk-native-helper") != digest(native_helper.get("sha256"))
                or source_files.get("native/macos/TalkNativeHelper.swift") != digest(native_helper.get("source_sha256"))
                or source_files.get("tools/native/build-macos-helper.py") != digest(native_helper.get("builder_sha256"))):
            raise ValueError("Native settings helper does not match its source/build binding")
        if files.get("assets/sounds/call-ringtone.wav") != digest(source_files.get("apps/desktop/assets/sounds/call-ringtone.wav")):
            raise ValueError("Native settings helper requires the original source-bound ringtone")
        native_target(regular(stage, "bin/talk-native-helper"), spec["target"])
    media = spec["media"]
    if type(media.get("enabled")) is not bool:
        raise ValueError("Media availability must be explicit")
    if media["enabled"]:
        if media != media_metadata(dependencies["packages"]):
            raise ValueError("Media engine does not match the shipping dependency inventory")
    elif media.get("helper"):
        raise ValueError("Disabled media cannot silently carry a helper")
    return spec


def tar_gz(root, output, epoch):
    with output.open("xb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as gz, tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for p in [root, *sorted(root.rglob("*"))]:
            info = archive.gettarinfo(str(p), arcname=p.relative_to(root.parent).as_posix())
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = epoch
            info.mode = 0o755 if p.is_dir() or p.stat().st_mode & stat.S_IXUSR else 0o644
            if p.is_file():
                with p.open("rb") as f:
                    archive.addfile(info, f)
            else:
                archive.addfile(info)


def zip_bundle(root, output, epoch):
    import time
    with zipfile.ZipFile(output, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for p in [root, *sorted(root.rglob("*"))]:
            name = p.relative_to(root.parent).as_posix() + ("/" if p.is_dir() else "")
            info = zipfile.ZipInfo(name, time.gmtime(epoch)[:6])
            info.create_system = 3
            mode = 0o755 if p.is_dir() or p.stat().st_mode & stat.S_IXUSR else 0o644
            info.external_attr = ((stat.S_IFDIR if p.is_dir() else stat.S_IFREG) | mode) << 16
            archive.writestr(info, b"" if p.is_dir() else p.read_bytes(), compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)


def assemble(stage, output, allow_fixture=False):
    stage, output = Path(stage).resolve(), Path(output).resolve()
    spec = validate(stage, allow_fixture)
    if stage == output or stage in output.parents:
        raise ValueError("Output must be outside inputs")
    output.mkdir(parents=True, exist_ok=False)
    suffix = "-test-fixture" if spec["kind"] == "packaging-test-fixture" else ""
    base = f"nextcloud-modern-talk-{spec['version']}-{TARGETS[spec['target']]}{suffix}"
    epoch = spec["source"]["epoch"]
    with tempfile.TemporaryDirectory(prefix="nmt-package-") as temp:
        temp = Path(temp)
        mac = spec["target"] == "aarch64-apple-darwin"
        root = temp / ("NextCloud Modern Talk.app" if mac else base)
        binary_dir = root / ("Contents/MacOS" if mac else "bin")
        resources = root / ("Contents/Resources" if mac else "share/nextcloud-modern-talk")
        binary_dir.mkdir(parents=True)
        resources.mkdir(parents=True)
        for name in spec["files"]:
            if name == "bin/talk-desktop":
                dest = binary_dir / "talk-desktop"
            elif name == "bin/talk-native-helper":
                dest = binary_dir / "talk-native-helper"
            elif mac and name.startswith("assets/sounds/") and name.endswith(".wav"):
                dest = resources / Path(name).name
            else:
                dest = resources / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(stage / name, dest)
            dest.chmod(0o755 if name.startswith("bin/") else 0o644)
        (resources / "provenance.json").write_text(json.dumps(spec, indent=2, sort_keys=True) + "\n")
        (resources / "INSTALL.txt").write_text(
            "NextCloud Modern Talk development package. No product or platform acceptance.\n"
            + ("Copy this .app to Applications. No Developer ID signature or notarization is supplied. Gatekeeper may block launch. Do not disable system protection.\n" if mac else
               "Ubuntu 24.04 x86_64 candidate: install the matching .deb using sudo apt install ./FILE.deb; launch talk-desktop from the application menu. The tar archive needs the runtime libraries recorded in provenance.\n")
            + "Media: " + ("Rust webrtc engine with GStreamer codecs; see release notes for supported server profiles.\n" if spec["media"]["enabled"] else "unavailable.\n")
        )
        if mac:
            plist = {"CFBundleName": "NextCloud Modern Talk", "CFBundleDisplayName": "NextCloud Modern Talk",
                     "CFBundleIdentifier": "org.adapt2move.nextcloud-modern-talk", "CFBundleExecutable": "talk-desktop",
                     "CFBundlePackageType": "APPL", "CFBundleShortVersionString": spec["version"].split("-")[0],
                     "CFBundleVersion": spec["version"].split("-")[0], "LSMinimumSystemVersion": "15.7",
                     "NSHighResolutionCapable": True, "NMTDevelopmentVersion": spec["version"]}
            if spec["build"].get("native_helper"):
                plist["NSMicrophoneUsageDescription"] = "Use the microphone when you explicitly check it in Devices or join a call with audio."
                plist["NSCameraUsageDescription"] = "Use the camera when you explicitly start a preview or enable video in a call."
            # No Developer ID is claimed. The final Mac check must qualify native
            # permissions and this sibling helper with the distributable bundle.
            if runtime := spec["build"].get("required_runtime"):
                install = resources / "INSTALL.txt"
                install.write_text(install.read_text() + "\nRequired before launch: " + runtime["name"] + " " + runtime["version"] + "\n" + runtime["url"] + "\nSHA256 " + runtime["sha256"] + "\n")
            (root / "Contents/Info.plist").write_bytes(plistlib.dumps(plist, sort_keys=True))
            zip_bundle(root, output / (base + ".zip"), epoch)
        else:
            tar_gz(root, output / (base + ".tar.gz"), epoch)
            depends = spec["build"].get("debian_depends", "")
            if not depends or not re.fullmatch(r"[a-zA-Z0-9.,+~():<>= \-]+", depends):
                raise ValueError("Validated Debian shared-library dependencies required")
            deb = temp / "deb"
            app = deb / "usr/lib/nextcloud-modern-talk"
            shutil.copytree(root, app)
            desktop = deb / "usr/share/applications/nextcloud-modern-talk.desktop"
            desktop.parent.mkdir(parents=True)
            desktop.write_text("[Desktop Entry]\nType=Application\nName=NextCloud Modern Talk (development)\nExec=/usr/lib/nextcloud-modern-talk/bin/talk-desktop\nTerminal=false\nCategories=Network;Chat;\n")
            control = deb / "DEBIAN/control"
            control.parent.mkdir()
            debian_version = spec["version"].replace("-", "~", 1)
            control.write_text(f"Package: nextcloud-modern-talk\nVersion: {debian_version}\nArchitecture: amd64\nMaintainer: NextCloud Modern Talk contributors\nSection: net\nPriority: optional\nDepends: {depends}\nRecommends: mesa-vulkan-drivers, fonts-noto-core, fonts-noto-color-emoji\nDescription: Native Talk development candidate\n Not a qualified production client. See installed provenance and notices.\n")
            for p in [deb, *deb.rglob("*")]:
                p.chmod(0o755 if p.is_dir() or p.stat().st_mode & stat.S_IXUSR else 0o644)
                os.utime(p, (epoch, epoch))
            env = dict(os.environ, SOURCE_DATE_EPOCH=str(epoch))
            try:
                subprocess.run(["dpkg-deb", "--build", "--root-owner-group", "-Zxz", str(deb), str(output / (base + ".deb"))], env=env, check=True, capture_output=True, timeout=120)
            except subprocess.CalledProcessError as error:
                raise ValueError("Debian assembly failed: " + error.stderr.decode(errors="replace")[:4000]) from error
    # Record actual output bytes, never a placeholder download URL.
    report = {"schema": 1, "kind": spec["kind"], "version": spec["version"], "target": spec["target"],
              "source": spec["source"], "input_sha256": sha(stage / "input.json"), "media_enabled": spec["media"]["enabled"],
              "signing": "no Developer ID / not notarized" if mac else "unsigned distribution archive",
              "assets": {p.name: {"sha256": sha(p), "bytes": p.stat().st_size} for p in sorted(output.iterdir())}}
    provenance = output / (base + ".provenance.json")
    provenance.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    (output / (base + ".SHA256SUMS")).write_text("".join(f"{sha(p)}  {p.name}\n" for p in sorted(output.iterdir()) if p.is_file()))
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    print(json.dumps(assemble(args.input, args.output), indent=2))
