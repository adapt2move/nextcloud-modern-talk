"""Identity and source inventory for the one patched upstream dependency.

The original inventory is pinned independently of UPSTREAM.json. Its recorded
delta describes the shipped source without relabelling GPUI as project code.
"""
import hashlib
import json
from pathlib import Path, PurePosixPath
import re


GPUI_ARCHIVE = "979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707"
GPUI_INVENTORY = "e19dd725f79a9e348c07388f12bae1f2c38e85660012a0e29c2d7a96e0bdcb51"
GPUI_MANIFEST = "f5686c4367d640e41e57a53679b89417ddf82b11ef4a484b8d1e6134763f74fc"


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def source_path(name):
    if (not isinstance(name, str) or not name or "\\" in name
            or any(ord(c) < 32 for c in name)):
        raise ValueError("Invalid vendored source path")
    path = PurePosixPath(name)
    if (path.is_absolute() or any(part in ("", ".", "..") for part in name.split("/"))
            or path.as_posix() != name):
        raise ValueError("Vendored source path leaves its package")
    return name


def digest(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise ValueError("Invalid vendored source digest")
    return value


def verify_gpui(root):
    package = Path(root) / "vendor/gpui"
    provenance = package / "UPSTREAM.json"
    if package.is_symlink() or provenance.is_symlink() or provenance.stat().st_size > 512 * 1024:
        raise ValueError("Invalid GPUI provenance file")
    data = json.loads(provenance.read_text())
    if (data.get("schema_version") != 1 or data.get("package") != "gpui"
            or data.get("version") != "0.2.2" or data.get("license") != "Apache-2.0"
            or data.get("archive_sha256") != GPUI_ARCHIVE):
        raise ValueError("Vendored GPUI identity differs from the published pin")
    original = data.get("original_files")
    if not isinstance(original, dict) or len(original) != 183:
        raise ValueError("Original GPUI inventory is incomplete")
    encoded = json.dumps(original, sort_keys=True, separators=(",", ":")).encode()
    if hashlib.sha256(encoded).hexdigest() != GPUI_INVENTORY:
        raise ValueError("Original GPUI inventory differs from the published archive")
    expected = dict(original)
    changes = data.get("files")
    if not isinstance(changes, list) or not 1 <= len(changes) <= 64:
        raise ValueError("Invalid GPUI patch inventory")
    changed = set()
    for entry in changes:
        name = source_path(entry["path"])
        if name in changed or name == "UPSTREAM.json":
            raise ValueError("Duplicate or recursive GPUI patch entry")
        changed.add(name)
        if entry["original_sha256"] != original.get(name):
            raise ValueError("GPUI patch preimage differs from upstream: " + name)
        expected[name] = digest(entry["sha256"])
    if expected["Cargo.toml"] != GPUI_MANIFEST or expected["LICENSE-APACHE"] != original["LICENSE-APACHE"]:
        raise ValueError("GPUI manifest or upstream license changed")
    actual = {}
    for path in package.rglob("*"):
        if path.is_symlink():
            raise ValueError("Symlink in vendored GPUI source")
        if path.is_file() and path != provenance:
            actual[path.relative_to(package).as_posix()] = sha(path)
    if actual != expected:
        differing = sorted(name for name in actual.keys() | expected.keys()
                           if actual.get(name) != expected.get(name))
        raise ValueError("Unrecorded or missing GPUI source: " + ", ".join(differing[:5]))
    return data
