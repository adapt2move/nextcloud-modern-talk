#!/usr/bin/env python3
"""Enforce the reviewed bootstrap graph and renderer boundary; Python 3.11+.

This is a conservative source guard, not a Rust parser or security proof.
The compile check enforces crate edges. Independent negative fixtures must exercise
new dependencies, renderer adapter access, raw boundary containers and endpoint leakage.
"""

from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
GRAPH = {
    "talk-core": ("crates/talk-core", set()),
    "talk-adapters": ("crates/talk-adapters", {"talk-core"}),
    "talk-bootstrap": ("apps/bootstrap", {"talk-core", "talk-adapters"}),
}
failures: list[str] = []


def require(condition: bool, message: str) -> None:
    if not condition:
        failures.append(message)


def read_toml(path: Path) -> dict:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def without_comments(source: str) -> str:
    # Preserve string contents so endpoint literals cannot hide in an accepted interface.
    # Nested block comments are forbidden here instead of being parsed incorrectly.
    require(not re.search(r"/\*(?:(?!\*/).)*?/\*", source, re.S),
            "nested Rust block comments need a parser-aware boundary check")
    return re.sub(r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*.*?\*/',
                  lambda match: match[0] if match[0].startswith('"') else " ",
                  source, flags=re.S)


def check() -> None:
    workspace = read_toml(ROOT / "Cargo.toml")["workspace"]
    require(set(workspace["members"]) == {path for path, _ in GRAPH.values()},
            "workspace members differ from the reviewed three-package graph")
    require(workspace["package"]["license"] == "EUPL-1.2", "workspace license changed")
    require(workspace["lints"]["rust"]["unsafe_code"] == "forbid", "unsafe policy changed")
    shared = workspace.get("dependencies", {})
    require(set(shared) == {"talk-core", "talk-adapters"},
            "new shared dependencies require architecture/license review")
    for name, entry in shared.items():
        require(name in GRAPH and entry == {"path": GRAPH.get(name, (None,))[0]},
                f"{name}: dependency must use its reviewed local path")

    for name, (directory, expected) in GRAPH.items():
        manifest = read_toml(ROOT / directory / "Cargo.toml")
        require(manifest["package"]["name"] == name, f"{directory}: unexpected package name")
        require(set(manifest) <= {"package", "dependencies", "lints"},
                f"{name}: unreviewed manifest section")
        require(set(manifest["package"]) == {"name", "version", "edition", "rust-version", "license", "publish"},
                f"{name}: unreviewed package/build configuration")
        for field in ("version", "edition", "rust-version", "license", "publish"):
            require(manifest["package"].get(field) == {"workspace": True},
                    f"{name}: {field} must inherit the workspace")
        require(manifest.get("lints") == {"workspace": True}, f"{name}: inherit workspace lints")
        deps = manifest.get("dependencies", {})
        require(set(deps) == expected, f"{name}: forbidden/missing dependency edge")
        require(all(value == {"workspace": True} for value in deps.values()),
                f"{name}: dependencies must use reviewed workspace definitions")
        require(not any(key in manifest for key in ("target", "build-dependencies", "dev-dependencies")),
                f"{name}: hidden platform/build/test edges need review")
        require(not (ROOT / directory / "build.rs").exists(), f"{name}: unreviewed build script")

    manifests = {str(path.relative_to(ROOT)) for base in ("crates", "apps")
                 for path in (ROOT / base).rglob("Cargo.toml")}
    require(manifests == {f"{path}/Cargo.toml" for path, _ in GRAPH.values()},
            "unreviewed package outside workspace membership")
    lock = read_toml(ROOT / "Cargo.lock")
    require(lock.get("version") == 4, "unexpected lockfile format")
    packages = lock.get("package", [])
    require({p["name"] for p in packages} == set(GRAPH) and len(packages) == len(GRAPH),
            "lockfile must contain exactly the reviewed local packages")
    for package in packages:
        name = package["name"]
        require(package.get("version") == workspace["package"]["version"],
                f"{name}: lockfile version mismatch")
        require("source" not in package and "checksum" not in package,
                f"{name}: registry/git dependency is outside this bootstrap")
        require(set(package.get("dependencies", [])) == GRAPH.get(name, (None, set()))[1],
                f"{name}: lockfile dependency mismatch")

    for path in (ROOT / "crates/talk-core/src").rglob("*.rs"):
        source = without_comments(path.read_text(encoding="utf-8"))
        require(not re.search(r"\b(?:gpui|reqwest|hyper|tokio|talk_adapters)\b|std\s*::\s*(?:net|fs|process|thread)", source),
                f"{path.relative_to(ROOT)}: core contains adapter/runtime I/O")
        require(not re.search(r"https?://|/ocs/|/remote\.php/", source),
                f"{path.relative_to(ROOT)}: raw protocol route leaked into core")
        require(not re.search(r"\b(?:include|include_str|include_bytes)!", source),
                f"{path.relative_to(ROOT)}: included source/data bypasses boundary inspection")

    boundary = without_comments((ROOT / "crates/talk-core/src/ui.rs").read_text(encoding="utf-8"))
    require(not re.search(r"\b(?:String|Vec|Box|Future|Task|Secret|Credential|Token|Frame|Surface|PortError)\b", boundary),
            "renderer contract exposes an unbounded, secret, I/O or frame type")
    require(not re.search(r"\b(?:secrets?|credentials?|tokens?|passwords?|http|authorization)\b", boundary, re.I),
            "renderer contract contains secret/transport-bearing identifiers")
    require(not re.search(r"\bpub\s+use\b|crate\s*::\s*(?:ports|application|lifecycle)", boundary),
            "renderer contract reaches worker-side implementation types")
    presentation = without_comments((ROOT / "apps/bootstrap/src/presentation.rs").read_text(encoding="utf-8"))
    require(not re.search(r"talk_adapters|talk_core\s*::\s*(?!ui\b)\w+|crate\s*::|super\s*::|std\s*::\s*(?:net|fs|process|thread)|\b(?:include|include_str|include_bytes)!", presentation),
            "presentation must depend only on the core UI boundary and bounded local formatting")


try:
    check()
except (OSError, KeyError, TypeError, ValueError) as error:
    failures.append(f"cannot inspect architecture: {error}")

if failures:
    for failure in failures:
        print(f"architecture: {failure}", file=sys.stderr)
    raise SystemExit(1)
print("architecture: reviewed bootstrap dependency and boundary rules hold")
