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
import macos_runtime
import macos_dmg


TARGETS = {"x86_64-unknown-linux-gnu": "linux-x86_64", "aarch64-apple-darwin": "macos-arm64-unsigned"}
REQUIRED_NOTICES = (
    "licenses/project/LICENSE", "licenses/project/NOTICE.md", "licenses/desktop/NOTICE.md",
    "licenses/google-sans/GoogleSans-LICENSE.txt", "licenses/gpui/GPUI-LICENSE-APACHE",
    "licenses/google-sans/GoogleSans-TRADEMARKS.txt",
    "licenses/google-sans-code/GoogleSansCode-LICENSE.txt",
    "licenses/google-sans-code/GoogleSansCode-TRADEMARKS.txt",
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
        if not (name.startswith(("licenses/", "assets/", macos_runtime.PREFIX)) or name in ("bin/talk-desktop", "bin/talk-native-helper", "source-manifest.json", "dependencies.json")):
            raise ValueError("Unexpected payload category: " + name)
    for name in (*REQUIRED_NOTICES, "dependencies.json", "source-manifest.json",
                 "assets/fonts/GoogleSans-Regular.ttf", "assets/fonts/GoogleSans-Medium.ttf",
                 "assets/fonts/GoogleSans-Bold.ttf", "assets/fonts/GoogleSansCode-Variable.ttf"):
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
    macos_runtime.validate_inputs(stage, spec)
    if spec["target"] == "aarch64-apple-darwin" and spec["kind"] == "development":
        if not {macos_dmg.ICON, macos_dmg.BACKGROUND} <= files.keys():
            raise ValueError("Source-bound macOS artwork missing")
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
        copied = {}
        for name in spec["files"]:
            if name == "bin/talk-desktop":
                dest = binary_dir / "talk-desktop"
            elif name == "bin/talk-native-helper":
                dest = binary_dir / "talk-native-helper"
            elif name.startswith(macos_runtime.PREFIX):
                dest = root / macos_runtime.bundle_path(name)
            elif mac and name == macos_dmg.ICON:
                dest = resources / "AppIcon.icns"
            elif mac and name.startswith("assets/sounds/") and name.endswith(".wav"):
                dest = resources / Path(name).name
            else:
                dest = resources / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(stage / name, dest)
            dest.chmod(0o755 if name.startswith(("bin/", macos_runtime.PREFIX)) else 0o644)
            copied[name] = dest
        (resources / "INSTALL.txt").write_text(
            "NextCloud Modern Talk alpha\n"
            + ("Drag NextCloud Modern Talk to Applications, eject the disk image, then open the installed app. Its media runtime is included; no GStreamer installation is required. The app is not Developer ID signed or notarized. If macOS cannot verify the developer, try opening the app once, then use System Settings > Privacy & Security > Open Anyway and confirm Open. On a German Mac: Systemeinstellungen > Datenschutz & Sicherheit > Dennoch öffnen. Do not disable Gatekeeper.\n" if mac else
               "Ubuntu 24.04 x86_64 candidate: install the matching .deb using sudo apt install ./FILE.deb; launch talk-desktop from the application menu. The tar archive needs the runtime libraries recorded in provenance.\n")
            + f"Installation: https://github.com/adapt2move/nextcloud-modern-talk/blob/v{spec['version']}/docs/install.md\n"
            + f"Known limitations: https://github.com/adapt2move/nextcloud-modern-talk/blob/v{spec['version']}/docs/limitations.md\n"
        )
        if mac:
            plist = {"CFBundleName": "NextCloud Modern Talk", "CFBundleDisplayName": "NextCloud Modern Talk",
                     "CFBundleIdentifier": "org.adapt2move.nextcloud-modern-talk", "CFBundleExecutable": "talk-desktop",
                     "CFBundlePackageType": "APPL", "CFBundleShortVersionString": spec["version"].split("-")[0],
                     "CFBundleVersion": spec["version"].split("-")[0], "LSMinimumSystemVersion": "15.7",
                     "NSHighResolutionCapable": True, "NMTDevelopmentVersion": spec["version"]}
            if macos_dmg.ICON in spec["files"]:
                plist["CFBundleIconFile"] = "AppIcon.icns"
            if spec["build"].get("native_helper"):
                plist["NSMicrophoneUsageDescription"] = "Use the microphone when you explicitly check it in Devices or join a call with audio."
                plist["NSCameraUsageDescription"] = "Use the camera when you explicitly start a preview or enable video in a call."
            (root / "Contents/Info.plist").write_bytes(plistlib.dumps(plist, sort_keys=True))
            # Ad-hoc signing meets Apple Silicon's code-integrity requirement.
            # It supplies neither an authenticated Developer ID nor notarization.
            # Seal the provenance as an ordinary resource before signing. Only
            # the main executable uses the explicitly named signature-independent
            # identity: its final raw hash depends on this resource envelope.
            if spec["kind"] == "development":
                spec["files"]["bin/talk-desktop"] = macos_runtime.unsigned_executable_sha256(binary_dir / "talk-desktop")
                spec["build"]["macos_main_identity"] = macos_runtime.MAIN_IDENTITY
                spec["build"]["bundle_signing"] = "ad-hoc; no Developer ID; not notarized"
                spec["build"]["native_linkage"] = macos_runtime.inspect_bundle(root)
            (resources / "provenance.json").write_text(json.dumps(spec, indent=2, sort_keys=True) + "\n")
            # Archive permissions must work regardless of the build user's umask.
            for path in [root, *root.rglob("*")]:
                path.chmod(0o755 if path.is_dir() or path.stat().st_mode & stat.S_IXUSR else 0o644)
            if spec["kind"] == "development":
                if sorted(path.name for path in root.iterdir()) != ["Contents"]:
                    raise ValueError("Unexpected application bundle root contents")
                # codesign creates CodeResources; its delivered mode must also
                # be predictable under a private build user's restrictive umask.
                old_umask = os.umask(0o022)
                try:
                    macos_runtime.run(["codesign", "--force", "--sign", "-", "--timestamp=none", root])
                finally:
                    os.umask(old_umask)
                # No bundle member is written after this point. Verify immediately
                # so a native layout/sealing failure has an unambiguous checkpoint.
                macos_runtime.run(["codesign", "--verify", "--deep", "--strict", root])
                print("macOS package: strict bundle verification passed immediately after signing", flush=True)
                if macos_runtime.unsigned_executable_sha256(binary_dir / "talk-desktop") != spec["files"]["bin/talk-desktop"]:
                    raise ValueError("Bundle signing changed the signature-independent main executable identity")
                if any(sha(path) != spec["files"][name] for name, path in copied.items() if name != "bin/talk-desktop"):
                    raise ValueError("Bundle signing changed an undeclared payload file")
                final_main_sha256 = sha(binary_dir / "talk-desktop")
                print("macOS package: signature-independent main executable roundtrip passed", flush=True)
                objects = macos_runtime.inspect_bundle(root)
                for path in objects:
                    # Apple's output complements our bounded Mach-O parser.
                    macos_runtime.run(["otool", "-L", root / path])
                    macos_runtime.run(["otool", "-l", root / path])
                dmg_report = macos_dmg.create(root, output / (base + ".dmg"))
            expanded_bytes = sum(path.stat().st_size for path in root.rglob("*") if path.is_file())
            if expanded_bytes > 1024 * 1024 * 1024:
                raise ValueError("Bundle exceeds the updater's expanded size limit")
            zip_bundle(root, output / (base + ".zip"), epoch)
            if (output / (base + ".zip")).stat().st_size > 512 * 1024 * 1024:
                raise ValueError("ZIP exceeds the updater's download size limit")
        else:
            (resources / "provenance.json").write_text(json.dumps(spec, indent=2, sort_keys=True) + "\n")
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
    if mac and spec["kind"] == "development":
        report["manual_download"] = base + ".dmg"
        report["updater_download"] = base + ".zip"
        report["macos_structure"] = dmg_report
        report["bundled_runtime"] = spec["build"]["bundled_runtime"]
        report["bundle_expanded_bytes"] = expanded_bytes
        report["main_executable"] = {
            "path": "Contents/MacOS/talk-desktop", "sha256": final_main_sha256,
            "manifest_identity_algorithm": macos_runtime.MAIN_IDENTITY,
            "manifest_identity_sha256": spec["files"]["bin/talk-desktop"],
        }
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
