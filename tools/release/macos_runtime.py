#!/usr/bin/env python3
"""The pinned macOS codec closure, relocation and structural verification.

No installed Homebrew library or host plugin registry is a packaging input.
The exact official runtime PKG is expanded privately. Only the application,
scanner and explicitly used plugins seed the transitive Mach-O dependency walk.
Native tools inspect/sign code; they never execute the application or plugins.
"""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import struct
import subprocess
import tarfile
import tempfile
import time
import urllib.request

VERSION = "1.28.4"
RUNTIME_URL = "https://gstreamer.freedesktop.org/data/pkg/osx/1.28.4/gstreamer-1.0-1.28.4-universal.pkg"
RUNTIME_SHA256 = "9f9ae8ba1a25eb42d6740c4e5f15b09ee6b9127b7359e9662c45a2b9a5029f85"
INSTALLED = "/Library/Frameworks/GStreamer.framework/Versions/1.0/"
PREFIX = "runtime/gstreamer/"
SCANNER = "libexec/gstreamer-1.0/gst-plugin-scanner"
MANIFEST = "assets/gstreamer-runtime.json"
MAX_FILES, MAX_BYTES = 128, 256 * 1024 * 1024
MAIN_IDENTITY = "codesign-remove-signature-sha256-v1"
DYLIB = re.compile(r"lib[a-zA-Z0-9_.+-]+\.dylib")

# Actual element/device usage, not every codec shipped by the SDK. The engine
# uses fixed Opus/VP8 RTP pipelines, never decodebin/playbin/autoplugging.
# https://gstreamer.freedesktop.org/documentation/deploying/mac-osx.html
PLUGINS = {
    "coreelements": ["capsfilter"],
    "app": ["appsrc", "appsink"],
    "opus": ["opusenc", "opusdec"],
    "vpx": ["vp8enc", "vp8dec"],
    "rtp": ["rtpopusdepay", "rtpvp8depay"],
    "audioconvert": ["audioconvert"],
    "audioresample": ["audioresample"],
    "videoconvertscale": ["videoconvert", "videoscale"],
    "videorate": ["videorate"],
    "applemedia": ["avfvideosrc", "avfdeviceprovider"],
}


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(argv, timeout=120):
    result = subprocess.run(list(map(str, argv)), check=False, capture_output=True, timeout=timeout)
    if result.returncode:
        raise ValueError(f"{argv[0]} failed ({result.returncode}): " +
                         result.stderr.decode(errors="replace")[-4000:])
    return result.stdout


def unsigned_executable_sha256(path):
    """Hash Apple's signature-removed representation without modifying the input.

    An outer app signature changes its main executable to bind the resource
    envelope. A manifest sealed inside those resources therefore cannot contain
    the final raw executable hash. Keep that hash in the detached release report;
    the sealed manifest explicitly names this signature-independent algorithm.
    Packaging must prove the before/after signing representations are identical.
    """
    path = Path(path)
    metadata = path.lstat()
    if not path.is_file() or path.is_symlink() or metadata.st_nlink != 1 or metadata.st_size > MAX_BYTES:
        raise ValueError("Invalid main executable for signature-independent identity")
    deadline = time.monotonic() + 60
    with tempfile.TemporaryDirectory(prefix="nmt-code-identity-") as temporary:
        copied = Path(temporary) / "talk-desktop"
        total = 0
        with path.open("rb") as source, copied.open("xb") as output:
            copied.chmod(0o600)
            while chunk := source.read(64 * 1024):
                total += len(chunk)
                if total > MAX_BYTES or time.monotonic() >= deadline:
                    raise ValueError("Main executable identity copy exceeded its bounds")
                output.write(chunk)
        if total != metadata.st_size:
            raise ValueError("Main executable changed while copying")
        # subprocess.run kills and reaps its child on timeout or interruption.
        run(["/usr/bin/codesign", "--remove-signature", copied], timeout=30)
        digest = hashlib.sha256()
        total = 0
        with copied.open("rb") as source:
            while chunk := source.read(64 * 1024):
                total += len(chunk)
                if total > MAX_BYTES or time.monotonic() >= deadline:
                    raise ValueError("Main executable identity hashing exceeded its bounds")
                digest.update(chunk)
        return digest.hexdigest()


def download(url, expected, destination, limit):
    total, digest = 0, hashlib.sha256()
    with urllib.request.urlopen(url, timeout=60) as response, destination.open("xb") as output:
        while chunk := response.read(1024 * 1024):
            total += len(chunk)
            if total > limit:
                raise ValueError("Native source/runtime download exceeds its limit")
            digest.update(chunk)
            output.write(chunk)
    if digest.hexdigest() != expected:
        raise ValueError("Native source/runtime checksum mismatch: " + destination.name)


def macho(path):
    """Read only the bounded arm64 header/load commands, including a fat input.

    This permits Linux fixture/PKG inspection without executing native code.
    All shipped files are subsequently thinned to arm64 and checked by otool.
    """
    path = Path(path)
    size = path.stat().st_size
    if size < 32 or size > MAX_BYTES:
        raise ValueError("Invalid Mach-O size")
    with path.open("rb") as stream:
        head = stream.read(32)
        offset, extent = 0, size
        if head[:4] in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"):
            count = struct.unpack_from(">I", head, 4)[0]
            width = 32 if head[:4] == b"\xca\xfe\xba\xbf" else 20
            if not 1 <= count <= 8:
                raise ValueError("Invalid universal Mach-O architecture count")
            stream.seek(8)
            archs = stream.read(width * count)
            if len(archs) != width * count:
                raise ValueError("Truncated universal Mach-O header")
            matches = []
            for i in range(count):
                fields = struct.unpack_from(">IIQQII" if width == 32 else ">IIIII", archs, i * width)
                if fields[0] == 0x0100000C:
                    matches.append(fields[2:4])
            if len(matches) != 1:
                raise ValueError("Mach-O must contain exactly one arm64 slice")
            offset, extent = matches[0]
            if offset < 8 + width * count or extent < 32 or offset + extent > size:
                raise ValueError("Invalid universal Mach-O slice extent")
            stream.seek(offset)
            head = stream.read(32)
        if head[:4] != b"\xcf\xfa\xed\xfe" or struct.unpack_from("<I", head, 4)[0] != 0x0100000C:
            raise ValueError("Expected arm64 Mach-O")
        _, _, _, kind, count, length, _, _ = struct.unpack("<8I", head)
        if kind not in (2, 6, 8) or not 1 <= count <= 1024 or length > 2 * 1024 * 1024 or 32 + length > extent:
            raise ValueError("Invalid Mach-O load-command bounds/type")
        commands = stream.read(length)
    dependencies, rpaths, identity = [], [], None
    at = 0
    for _ in range(count):
        if at + 8 > length:
            raise ValueError("Truncated Mach-O load command")
        command, width = struct.unpack_from("<II", commands, at)
        if width < 8 or width % 8 or at + width > length:
            raise ValueError("Invalid Mach-O load command size")
        dylib = command in (0xC, 0xD, 0x80000018, 0x8000001F, 0x20, 0x80000023)
        if dylib or command == 0x8000001C:
            minimum = 24 if dylib else 12
            if width <= minimum:
                raise ValueError("Truncated Mach-O path")
            start = struct.unpack_from("<I", commands, at + 8)[0]
            if not minimum <= start < width:
                raise ValueError("Invalid Mach-O path offset")
            raw = commands[at + start:at + width]
            if b"\0" not in raw:
                raise ValueError("Unterminated Mach-O path")
            value = raw.split(b"\0", 1)[0].decode("utf-8")
            if not value or any(ord(c) < 32 for c in value):
                raise ValueError("Invalid Mach-O path")
            if command == 0xD:
                if identity is not None:
                    raise ValueError("Duplicate Mach-O install ID")
                identity = value
            elif dylib:
                dependencies.append(value)
            else:
                rpaths.append(value)
        at += width
    if at != length or (kind == 6) != (identity is not None):
        raise ValueError("Inconsistent Mach-O commands/install ID")
    return {"kind": kind, "dependencies": dependencies, "rpaths": rpaths, "id": identity}


def system_library(name):
    return (name.startswith(("/System/Library/", "/usr/lib/"))
            and ".." not in PurePosixPath(name).parts and "//" not in name)


def library_relative(name):
    """Only the exact SDK namespace is a permissible non-system input."""
    if name.startswith("@rpath/"):
        name = name.removeprefix("@rpath/")
    elif name.startswith(INSTALLED + "lib/"):
        name = name.removeprefix(INSTALLED + "lib/")
    else:
        raise ValueError("Undeclared native dependency: " + name)
    if not DYLIB.fullmatch(name):
        raise ValueError("Invalid runtime library name: " + name)
    return "lib/" + name


def bundle_path(name):
    """Closed logical-to-physical map also implemented by the updater."""
    if not name.startswith(PREFIX):
        raise ValueError("Unknown runtime namespace")
    relative = name.removeprefix(PREFIX)
    path = PurePosixPath(relative)
    if relative == SCANNER:
        return "Contents/Helpers/gst-plugin-scanner"
    if len(path.parts) == 2 and path.parts[0] == "lib" and DYLIB.fullmatch(path.name):
        return "Contents/Frameworks/" + path.name
    if (len(path.parts) == 3 and path.parts[:2] == ("lib", "gstreamer-1.0")
            and path.name in {"libgst" + p + ".dylib" for p in PLUGINS}):
        return "Contents/PlugIns/" + path.name
    raise ValueError("Unknown runtime payload: " + name)


def closure(find, executable):
    pending = [SCANNER, *("lib/gstreamer-1.0/libgst" + p + ".dylib" for p in sorted(PLUGINS))]
    for dep in macho(executable)["dependencies"]:
        if not system_library(dep):
            pending.append(library_relative(dep))
    found, total = {}, 0
    while pending:
        name = pending.pop()
        if name in found:
            continue
        bundle_path(PREFIX + name)
        path = find(name)
        total += path.stat().st_size
        if len(found) >= MAX_FILES or total > MAX_BYTES:
            raise ValueError("GStreamer dependency closure exceeds its bound")
        info = macho(path)
        if (name == SCANNER and info["kind"] != 2) or (name != SCANNER and info["kind"] != 6):
            raise ValueError("Unexpected runtime Mach-O type: " + name)
        found[name] = (path, info)
        for dep in info["dependencies"]:
            if not system_library(dep):
                pending.append(library_relative(dep))
    return dict(sorted(found.items()))


def expanded_finder(roots):
    """Resolve only selected PKG files; aliases may not leave their payload."""
    if not 1 <= len(roots) <= 64:
        raise ValueError("Unexpected runtime component count")

    def find(name):
        matches = []
        for root in roots:
            candidate = root / name
            if candidate.exists():
                resolved = candidate.resolve(strict=True)
                if not resolved.is_relative_to(root.resolve()) or not resolved.is_file():
                    raise ValueError("Runtime package alias escapes its component")
                matches.append(resolved)
        if not matches or len({sha(p) for p in matches}) != 1:
            raise ValueError("Missing or conflicting runtime file: " + name)
        return matches[0]
    return find


def loader_reference(source, target):
    return "@loader_path/" + os.path.relpath(target, start=source.parent).replace(os.sep, "/")


def relocate(path, info, physical, libraries):
    args = ["install_name_tool"]
    for dep in info["dependencies"]:
        if not system_library(dep):
            target = libraries[library_relative(dep)]
            args += ["-change", dep, loader_reference(physical, target)]
    # No inherited SDK/Homebrew/Xcode/build-tree runpath remains. Each load is
    # relative to the object that owns it, including plugins and the scanner.
    for rpath in dict.fromkeys(info["rpaths"]):
        args += ["-delete_rpath", rpath]
    if info["id"] and info["id"] != "@rpath/" + path.name:
        args += ["-id", "@rpath/" + path.name]
    if len(args) > 1:
        run([*args, path])
    run(["codesign", "--force", "--sign", "-", "--timestamp=none", path])
    run(["codesign", "--verify", "--strict", path])


def inspect_bundle(app):
    """Fail before distribution if any Mach-O edge escapes or is unresolved."""
    app = Path(app).resolve()
    objects, total = {}, 0
    for path in sorted(app.rglob("*")):
        if path.is_symlink():
            raise ValueError("Symlink in self-contained application")
        if not path.is_file():
            continue
        with path.open("rb") as stream:
            magic = stream.read(4)
        if magic not in (b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"):
            continue
        info = macho(path)
        if magic != b"\xcf\xfa\xed\xfe":
            raise ValueError("Shipping Mach-O must be arm64 only")
        relative = path.relative_to(app).as_posix()
        if relative in ("Contents/MacOS/talk-desktop", "Contents/MacOS/talk-native-helper",
                        bundle_path(PREFIX + SCANNER)):
            expected_kind = 2
        elif (relative.startswith("Contents/Frameworks/") and
              DYLIB.fullmatch(relative.removeprefix("Contents/Frameworks/"))):
            expected_kind = 6
        elif relative in {bundle_path(PREFIX + "lib/gstreamer-1.0/libgst" + p + ".dylib") for p in PLUGINS}:
            expected_kind = 6
        else:
            raise ValueError("Mach-O outside a declared bundle code location")
        if info["kind"] != expected_kind or info["id"] not in (None, "@rpath/" + path.name):
            raise ValueError("Unexpected Mach-O type or install ID")
        total += path.stat().st_size
        if len(objects) >= MAX_FILES + 2 or total > 512 * 1024 * 1024:
            raise ValueError("Excessive bundled Mach-O closure")
        if info["rpaths"]:
            raise ValueError("Unexpected runtime search path in bundle")
        for dependency in info["dependencies"]:
            if system_library(dependency):
                continue
            if not dependency.startswith("@loader_path/"):
                raise ValueError("Non-relocated Mach-O dependency: " + dependency)
            target = (path.parent / dependency.removeprefix("@loader_path/")).resolve()
            if not target.is_relative_to(app) or not target.is_file() or target.is_symlink():
                raise ValueError("Unresolved/escaping bundled dependency: " + dependency)
            macho(target)
        objects[relative] = info
    required = {bundle_path(PREFIX + SCANNER), "Contents/MacOS/talk-desktop"}
    required.update(bundle_path(PREFIX + "lib/gstreamer-1.0/libgst" + p + ".dylib") for p in PLUGINS)
    if not required <= objects.keys():
        raise ValueError("Required bundled plugin/scanner/executable missing")
    return objects


def collect_sources(root, stage, files):
    """Ship sources and readable notices with every standalone .app.

    The exact Cerbero tree contains upstream patches/build recipes. The source
    tarballs include all LGPL components in this closure. MoltenVK's permissive
    static dependencies are included as well, with their original notices.
    No written future source offer or upstream download availability is needed.
    """
    lock_path = root / "tools/release/macos-sources.json"
    lock = json.loads(lock_path.read_text())
    if lock.get("schema") != 1 or lock.get("runtime_version") != VERSION or not 1 <= len(lock["sources"]) <= 32:
        raise ValueError("Unknown native source lock")
    declared = [name for record in lock["sources"] for name in record["runtime_files"]]
    if len(declared) != len(set(declared)):
        raise ValueError("Duplicate native source/license coverage")
    if set(declared) != set(files):
        uncovered = sorted(set(files) - set(declared))
        unused = sorted(set(declared) - set(files))
        raise ValueError(
            f"Native closure differs from source/license coverage: "
            f"uncovered={uncovered}, unused={unused}"
        )
    destination = stage / "licenses/gstreamer"
    (destination / "sources").mkdir(parents=True)
    shutil.copyfile(lock_path, destination / "sources.json")
    total, notice_total = 0, 0
    for source in lock["sources"]:
        filename = source["filename"]
        if "/" in filename or "\\" in filename or not re.fullmatch(r"[a-zA-Z0-9._-]+", filename):
            raise ValueError("Invalid source archive name")
        archive = destination / "sources" / filename
        download(source["url"], source["sha256"], archive, 32 * 1024 * 1024)
        total += archive.stat().st_size
        if total > 128 * 1024 * 1024:
            raise ValueError("Native source closure exceeds size limit")
        notices, members = [], 0
        with tarfile.open(archive) as tar:
            for member in tar:
                members += 1
                if members > 100000:
                    raise ValueError("Source archive has excessive entries")
                name = PurePosixPath(member.name)
                if name.is_absolute() or ".." in name.parts or "\\" in member.name:
                    raise ValueError("Unsafe source archive path")
                if (member.isfile() and len(name.parts) <= 5 and
                    (re.match(r"^(license|licence|copying|notice|copyright|patents)([._-]|$)", name.name, re.I)
                     or source["name"] == "zlib" and name.name == "README")):
                    if not 0 < member.size <= 2 * 1024 * 1024:
                        raise ValueError("Oversized native license")
                    body = tar.extractfile(member).read(member.size + 1)
                    # gst/tag/license-translations.dict is a binary lookup table,
                    # not license prose. Preserve original encodings for texts.
                    if b"\0" in body:
                        continue
                    notice_total += member.size
                    if notice_total > 8 * 1024 * 1024 or len(notices) >= 128:
                        raise ValueError("Excessive native notices")
                    dest = destination / "notices" / source["name"] / name
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    dest.write_bytes(body)
                    notices.append(dest.relative_to(stage).as_posix())
        if not notices:
            raise ValueError("Source archive contains no collected license: " + filename)
        source["notices"] = sorted(notices)
    (destination / "NOTICE.txt").write_text(
        "GStreamer 1.28.4 private macOS runtime\n\n"
        "This application dynamically links the official GStreamer runtime and the "
        "specific plugins/libraries recorded in assets/gstreamer-runtime.json. "
        "The binaries were reduced to arm64, their Mach-O load paths were changed "
        "for this app bundle, and ad-hoc code signatures were applied. The library "
        "source code is unmodified.\n\n"
        "Original license, copyright and patent notices are in notices/. "
        "The corresponding source archives are in sources/, with versions, upstream "
        "URLs and SHA-256 digests in sources.json. The Cerbero 1.28.4 archive "
        "includes the build recipes, patches and bootstrap/configuration scripts "
        "used by the official distribution. Follow that tree's README and "
        "config/cross-macos-universal.cbc to build replacement libraries. "
        "MoltenVK 1.2.9 comes from Vulkan SDK 1.3.283.0; its dependency revisions "
        "and source archives are also included.\n\n"
        "The LGPL-covered libraries remain dynamically replaceable. Their license "
        "terms, including modification and debugging rights, are preserved. "
        "Modified native code requires fresh local code signatures on macOS. "
        "This app has no Developer ID signature and is not notarized.\n"
    )
    return {"lock_sha256": sha(lock_path), "source_bytes": total, "sources": lock["sources"]}


def prepare(root, stage, temporary):
    """Populate hash-bound package inputs from the pinned official runtime."""
    pkg = temporary / "gstreamer-runtime.pkg"
    download(RUNTIME_URL, RUNTIME_SHA256, pkg, 256 * 1024 * 1024)
    expanded = temporary / "gstreamer-expanded"
    run(["pkgutil", "--expand-full", pkg, expanded], 180)
    roots = sorted(p for p in expanded.glob("*.pkg/Payload") if p.is_dir())
    files = closure(expanded_finder(roots), stage / "bin/talk-desktop")
    sources = collect_sources(root, stage, files)
    libraries = {name: Path(bundle_path(PREFIX + name)) for name in files if name.startswith("lib/lib")}
    records = {}
    for name, (original, info) in files.items():
        dest = stage / PREFIX / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        # lipo emits only the architecture advertised by this release target.
        with original.open("rb") as stream:
            thin = stream.read(4) == b"\xcf\xfa\xed\xfe"
        if thin:
            shutil.copyfile(original, dest)
        else:
            run(["lipo", original, "-thin", "arm64", "-output", dest])
        dest.chmod(0o755)
        physical = Path(bundle_path(PREFIX + name))
        relocate(dest, info, physical, libraries)
        after = macho(dest)
        records[name] = {"source_sha256": sha(original), "sha256": sha(dest),
                         "bundle_path": physical.as_posix(), **after}
    # Include the native settings helper in the same complete path inspection.
    for name in ("talk-desktop", "talk-native-helper"):
        path = stage / "bin" / name
        if path.is_file():
            path.chmod(0o755)
            relocate(path, macho(path), Path("Contents/MacOS") / name, libraries)
    manifest = {
        "schema": 1, "name": "GStreamer", "version": VERSION, "architecture": "arm64",
        "runtime_archive": {"url": RUNTIME_URL, "sha256": RUNTIME_SHA256},
        "plugins": PLUGINS,
        "plugin_usage_sources": ["crates/talk-adapters/src/media/codecs.rs", "apps/desktop/src/devices/camera.rs"],
        "files": records, "source_distribution": sources,
        "modifications": "arm64 thinning, loader-relative install names, removed runpaths, ad-hoc signing",
        "qualification": "structural inspection only; no application, plugin or scanner execution",
    }
    (stage / MANIFEST).write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    return {"name": "GStreamer", "version": VERSION, "manifest": MANIFEST,
            "manifest_sha256": sha(stage / MANIFEST)}


def validate_inputs(stage, spec):
    """Validate the runtime's closed manifest before assembling any output."""
    declaration = spec["build"].get("bundled_runtime")
    expected = {name.removeprefix(PREFIX): digest for name, digest in spec["files"].items() if name.startswith(PREFIX)}
    if spec["target"] != "aarch64-apple-darwin":
        if declaration is not None or expected:
            raise ValueError("macOS runtime in a different target")
        return
    if spec["kind"] == "packaging-test-fixture" and declaration is None and not expected:
        return
    if not isinstance(declaration, dict) or declaration != {
        "name": "GStreamer", "version": VERSION, "manifest": MANIFEST,
        "manifest_sha256": spec["files"].get(MANIFEST),
    }:
        raise ValueError("Missing or invalid bundled runtime declaration")
    manifest = json.loads((stage / MANIFEST).read_text())
    if (manifest.get("schema") != 1 or manifest.get("version") != VERSION
            or manifest.get("runtime_archive") != {"url": RUNTIME_URL, "sha256": RUNTIME_SHA256}
            or manifest.get("plugins") != PLUGINS or set(manifest["files"]) != set(expected)):
        raise ValueError("Runtime manifest does not bind the required closure")
    required = {SCANNER, "lib/libgstreamer-1.0.0.dylib"}
    required.update("lib/gstreamer-1.0/libgst" + p + ".dylib" for p in PLUGINS)
    if not required <= expected.keys() or len(expected) > MAX_FILES:
        raise ValueError("Runtime plugin/scanner closure incomplete")
    for name, digest in expected.items():
        entry = manifest["files"][name]
        if entry["sha256"] != digest or entry["bundle_path"] != bundle_path(PREFIX + name):
            raise ValueError("Runtime bytes/layout differ from manifest")
        if not re.fullmatch("[0-9a-f]{64}", entry.get("source_sha256", "")):
            raise ValueError("Runtime source identity missing")
        info = macho(stage / PREFIX / name)
        if info["rpaths"] or info["kind"] != (2 if name == SCANNER else 6):
            raise ValueError("Unrelocated runtime/type in package input")
        if any(entry.get(key) != value for key, value in info.items()):
            raise ValueError("Runtime load-command inventory differs from bytes")
    sources = manifest["source_distribution"]
    if not 1 <= len(sources["sources"]) <= 32:
        raise ValueError("Corresponding native source inventory missing")
    coverage = [name for source in sources["sources"] for name in source["runtime_files"]]
    if len(coverage) != len(set(coverage)) or set(coverage) != set(expected):
        raise ValueError("Corresponding native source coverage differs from closure")
    for source in sources["sources"]:
        if spec["files"].get("licenses/gstreamer/sources/" + source["filename"]) != source["sha256"]:
            raise ValueError("Corresponding native source missing or changed")
        if not source.get("notices") or any(n not in spec["files"] for n in source["notices"]):
            raise ValueError("Native license notices missing")
