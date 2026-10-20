#!/usr/bin/env python3
"""One private Linux update/relaunch proof; all artifacts stay below an owned home temp dir.

Build the backend-test-only update-install-probe example first. This runner makes
labelled 0.0.0-fixture.1/.2 packages, compiles one tiny std-only replacement using
the selected rustc, tests digest refusal, then installs and relaunches that fixture.
It does not use GitHub, PackageKit, /usr, the app checkout, or real app data.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib

OLD = "0.0.0-fixture.1"
NEW = "0.0.0-fixture.2"
RESOURCE = "share/nextcloud-modern-talk"


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def bundle(root, executable, version):
    files = {"bin/talk-desktop": executable.read_bytes()}
    for name in ("licenses/project/LICENSE", "licenses/project/NOTICE.md", "licenses/desktop/NOTICE.md", "source-manifest.json", "dependencies.json"):
        files[name] = b"SYNTHETIC PRIVATE UPDATE FIXTURE; NOT A SHIPPING PACKAGE\n"
    manifest = {}
    for name, data in files.items():
        relative = name if name.startswith("bin/") else f"{RESOURCE}/{name}"
        path = root / relative
        path.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o755 if name.startswith("bin/") else 0o644)
        manifest[name] = sha(path)
    provenance = {"schema": 1, "kind": "development", "version": version, "target": "x86_64-unknown-linux-gnu",
                  "build": {"profile": "release", "package": "talk-desktop", "features": []}, "files": manifest}
    (root / RESOURCE / "provenance.json").write_text(json.dumps(provenance) + "\n")
    (root / RESOURCE / "INSTALL.txt").write_text("SYNTHETIC PRIVATE UPDATE FIXTURE ONLY\n")
    for path in [root, *root.rglob("*")]:
        if path.is_dir():
            path.chmod(0o755)
        elif path.name != "talk-desktop":
            path.chmod(0o644)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", required=True, type=Path, help="Compiled update-install-probe example")
    args = parser.parse_args()
    if os.uname().sysname != "Linux" or os.uname().machine != "x86_64":
        raise SystemExit("This fixture qualifies only Linux x86_64.")
    probe = args.probe.resolve(strict=True)
    source = Path(__file__).resolve().parents[2]
    pin = tomllib.loads((source / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    rustc = subprocess.run(["rustc", "--version"], check=True, text=True, capture_output=True, timeout=10).stdout
    if not rustc.startswith(f"rustc {pin} "):
        raise SystemExit("Use the repository's pinned Rust toolchain.")
    home = Path.home().resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix=".nmt-private-update-fixture-", dir=home) as temporary:
        base = Path(temporary)
        base.chmod(0o700)
        (base / "synthetic-update-probe").write_bytes(b"NMT PRIVATE UPDATE FIXTURE ONLY\n")
        new_source = base / "new.rs"
        new_source.write_text('fn main() { assert_eq!(std::env::args().nth(1).as_deref(), Some("synthetic-restart")); println!("NMT UPDATE FIXTURE NEW"); }\n')
        new_exe = base / "new-label"
        subprocess.run(["rustc", "--edition=2024", "-O", str(new_source), "-o", str(new_exe)], check=True, timeout=30)
        old_root = base / "owned-installed-fixture"
        new_root = base / f"nextcloud-modern-talk-{NEW}-linux-x86_64"
        bundle(old_root, probe, OLD)
        bundle(new_root, new_exe, NEW)
        archive = base / f"{new_root.name}.tar.gz"
        with archive.open("xb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as gz, tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as tar:
            for path in [new_root, *sorted(new_root.rglob("*"))]:
                info = tar.gettarinfo(str(path), arcname=path.relative_to(new_root.parent).as_posix())
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mtime = 1
                if path.is_file():
                    with path.open("rb") as content:
                        tar.addfile(info, content)
                else:
                    tar.addfile(info)
        executable = old_root / "bin/talk-desktop"
        old_hash = sha(executable)
        refused = subprocess.run([str(executable), "synthetic-update-fixture", str(archive), "0" * 64], capture_output=True, text=True, timeout=60)
        if refused.returncode == 0 or "REFUSED Integrity" not in refused.stdout or sha(executable) != old_hash:
            raise SystemExit("Digest refusal did not preserve the old fixture executable.")
        installed = subprocess.run([str(executable), "synthetic-update-fixture", str(archive), sha(archive)], capture_output=True, text=True, timeout=60)
        if installed.returncode != 0 or "OLD_RUNNING_IMAGE_RELOCATED" not in installed.stdout or "INSTALLED_AND_RELAUNCHED_EXACT_STAGED_FIXTURE" not in installed.stdout:
            raise SystemExit("Private update/relaunch failed: " + installed.stdout[-2000:] + installed.stderr[-2000:])
        if sha(executable) != sha(new_exe):
            raise SystemExit("Installed executable differs from the staged replacement.")
        print("PASS: digest refusal preserved OLD; standard bundle swap installed NEW; captured installed path relaunched NEW; all private fixtures removed on exit.")


if __name__ == "__main__":
    main()
