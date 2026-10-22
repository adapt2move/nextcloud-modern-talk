#!/usr/bin/env python3
"""Run Cargo with the native development environment, shared by Task and CI.

Arguments and exit status pass through unchanged. Cargo owns build scheduling,
profiles and the lockfile; release packaging owns its separate pinned SDK policy.
"""
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FRAMEWORK = Path("/Library/Frameworks/GStreamer.framework/Versions/1.0")
NATIVE_COMMANDS = {"build", "check", "clippy", "run", "test"}


def query(argv, env):
    result = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=15)
    return result.stdout.strip() if result.returncode == 0 else None


def macos_gstreamer(env):
    """Honor pkg-config's selected SDK, then try the framework and Homebrew.

    Homebrew's stable opt path keeps a rebuilt binary independent of the current
    Cellar version. Never substitute it for a different explicitly selected SDK.
    """
    pkg_config = shutil.which("pkg-config", path=env.get("PATH"))
    if not pkg_config:
        raise ValueError("pkg-config is missing. On macOS: brew install pkgconf gstreamer")

    library = query([pkg_config, "--variable=libdir", "gstreamer-1.0"], env)
    brew = shutil.which("brew", path=env.get("PATH"))
    prefix = query([brew, "--prefix", "gstreamer"], env) if brew else None
    if not library:
        candidates = [FRAMEWORK / "lib/pkgconfig"]
        if prefix:
            candidates.append(Path(prefix) / "lib/pkgconfig")
        for directory in candidates:
            if not (directory / "gstreamer-1.0.pc").is_file():
                continue
            candidate = env.copy()
            candidate["PKG_CONFIG_PATH"] = os.pathsep.join(filter(None, [str(directory), env.get("PKG_CONFIG_PATH")]))
            library = query([pkg_config, "--variable=libdir", "gstreamer-1.0"], candidate)
            if library:
                env.update(candidate)
                break
    if not library:
        raise ValueError("GStreamer development files were not found. Install the framework SDK or run: brew install gstreamer")
    if query([pkg_config, "--atleast-version=1.24", "gstreamer-1.0"], env) is None:
        raise ValueError("The selected GStreamer SDK is older than 1.24. Update it or correct PKG_CONFIG_PATH.")
    library = Path(library)
    if not library.is_dir():
        raise ValueError("The selected GStreamer library directory is missing. Repair the SDK or PKG_CONFIG_PATH.")
    if prefix:
        stable = Path(prefix) / "lib"
        if stable.resolve() == library.resolve():
            library = stable
    return library


def environment(argv, source=None, system=None):
    env = dict(os.environ if source is None else source)
    if (system or platform.system()) != "Darwin" or not argv or argv[0] not in NATIVE_COMMANDS:
        return env
    library = macos_gstreamer(env)
    # Cargo splits RUSTFLAGS on whitespace. Encoded flags retain a native SDK
    # path containing spaces and preserve the precedence of existing overrides.
    flags = (env["CARGO_ENCODED_RUSTFLAGS"].split("\x1f") if "CARGO_ENCODED_RUSTFLAGS" in env
             else env.get("RUSTFLAGS", "").split())
    flags = [flag for flag in flags if flag]
    rpath = "link-arg=-Wl,-rpath," + str(library)
    if rpath not in flags:
        env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join([*flags, "-C", rpath])
    # macOS exposes /var as an alias for /private/var. Identity/storage checks
    # require canonical private paths, including their temporary test roots.
    env["TMPDIR"] = str(Path(env.get("TMPDIR") or tempfile.gettempdir()).resolve(strict=True))
    return env


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if not argv:
        print("Usage: python3 tools/cargo.py <cargo command> [arguments...]", file=sys.stderr)
        return 2
    try:
        env = environment(argv)
        cargo = shutil.which("cargo", path=env.get("PATH"))
        if not cargo:
            raise ValueError("Cargo is missing. Install rustup and the repository's pinned toolchain.")
        # Replacing this process preserves terminal input and Ctrl-C for cargo
        # run, including the application it starts. No shell reparses arguments.
        os.chdir(ROOT)
        os.execvpe(cargo, [cargo, *argv], env)
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        print("Cargo environment: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
