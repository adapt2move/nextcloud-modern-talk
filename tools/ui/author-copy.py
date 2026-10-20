#!/usr/bin/env python3
"""Create an explicitly isolated author build; never alter the canonical workspace/lock.

Run in the development VM: python3 tools/ui/author-copy.py /tmp/talk-ui-author
The integration owner must subsequently resolve and check the canonical workspace.
"""
from pathlib import Path
import shutil
import sys

source = Path(__file__).resolve().parents[2]
dest = Path(sys.argv[1]).resolve()
if dest == source or source in dest.parents or dest in source.parents:
    raise SystemExit("Author output must be outside the repository and its ancestors")
dest.mkdir(parents=True, exist_ok=True)
for path in ("apps/desktop", "crates/talk-core", "tools/ui"):
    # Fresh VM mtimes are intentional: host/VM clock skew must not leave stale
    # Cargo artifacts when a newly copied source has an older host timestamp.
    shutil.copytree(source / path, dest / path, dirs_exist_ok=True, copy_function=shutil.copy)
shutil.copy2(source / "rust-toolchain.toml", dest / "rust-toolchain.toml")
(dest / "Cargo.toml").write_text('''[workspace]
members = ["crates/talk-core", "apps/desktop"]
resolver = "3"
[workspace.package]
version = "0.0.0"
edition = "2024"
rust-version = "1.99"
license = "EUPL-1.2"
publish = false
[workspace.dependencies]
talk-core = { path = "crates/talk-core" }
gpui = { version = "=0.2.2", default-features = false, features = ["font-kit", "x11"] }
unicode-segmentation = "=1.12.0"
[workspace.lints.rust]
unsafe_code = "forbid"
[workspace.lints.clippy]
all = "warn"
[profile.dev]
debug = 0
''')
print(dest)
