#!/usr/bin/env python3
"""Enforce the reviewed bootstrap graph and renderer boundary; Python 3.11+.

This is a conservative lexical guard, not a Rust parser, sandbox or information-flow
proof. Reviewed UI exports are checked as explicit paths, including grouped imports.
Aliases/globs and unsupported import syntax are rejected at the inspected boundaries.
Direct standard growable containers are rejected by identifier, including qualified
spellings. Type resolution, transitive field bounds, macro expansion and all possible
Rust encodings need compiler plus human review; this guard does not prove them.
Independent positive/negative fixtures must exercise the usable renderer surface as
well as forbidden dependencies, containers, worker types and endpoint leakage.
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


# This is the reviewed transitive data vocabulary, not permission to reexport modules.
UI_EXPORTS = {
    *(('crate', 'bounded', name) for name in ('LimitExceeded', 'Page', 'Text')),
    ('crate', 'identity', 'PersistenceState'),
    *(('crate', 'ids', name) for name in (
        'AccountId', 'AccountScope', 'CallGeneration', 'CallScope', 'ConversationId',
        'HostId', 'RuntimeGeneration', 'VaultGeneration',
    )),
    ('crate', 'policy', 'Decision'),
}
UNBOUNDED = (
    r"\b(?:String|Vec|Box|VecDeque|LinkedList|HashMap|HashSet|BTreeMap|BTreeSet|"
    r"BinaryHeap|PathBuf|OsString|CString|Cow)\b"
)
WORKER_OR_FRAME = (
    r"\b(?:Future|Task|Secret|Credential|Token|Frame|Surface|PortError|Bootstrap|"
    r"Admission|Cancellation|IdentityPort|NextcloudPort|MediaPort|OperationContext|"
    r"Arc|Rc|Mutex|RwLock|RefCell)\b"
)
USE = re.compile(r"\b(?:(pub(?:\s*\([^)]*\))?)\s+)?use\s+([^;]+);", re.S)


def use_paths(expression: str) -> list[tuple[str, ...]] | None:
    """Flatten simple/nested grouped paths; reject aliases/globs/unknown grammar.

    This parses only the deliberately narrow import grammar, not Rust source.
    Keeping the original path means braces cannot hide a forbidden dependency.
    """
    tokens = re.findall(r"::|[A-Za-z_][A-Za-z_0-9]*|[{},]", expression)
    if ''.join(tokens) != re.sub(r"\s+", '', expression):
        return None
    cursor = 0
    leaves: list[tuple[str, ...]] = []

    def tree(prefix: tuple[str, ...]) -> None:
        nonlocal cursor
        if cursor >= len(tokens):
            raise ValueError('incomplete import')
        token = tokens[cursor]
        cursor += 1
        if token == '{':
            while cursor < len(tokens) and tokens[cursor] != '}':
                tree(prefix)
                if cursor < len(tokens) and tokens[cursor] == ',':
                    cursor += 1
                elif cursor >= len(tokens) or tokens[cursor] != '}':
                    raise ValueError('invalid group separator')
            if cursor >= len(tokens) or tokens[cursor] != '}':
                raise ValueError('unclosed group')
            cursor += 1
        elif re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", token):
            path = prefix + (token,)
            if cursor < len(tokens) and tokens[cursor] == '::':
                cursor += 1
                tree(path)
            else:
                leaves.append(path)
        else:
            raise ValueError('unsupported import')

    try:
        if tokens and tokens[0] == '::':
            cursor += 1
        tree(())
        return leaves if cursor == len(tokens) and leaves else None
    except ValueError:
        return None


def checked_imports(source: str, boundary: bool) -> str:
    exports: set[tuple[str, ...]] = set()
    for match in USE.finditer(source):
        paths = use_paths(match[2])
        require(paths is not None, 'boundary imports need explicit paths without aliases/globs')
        for path in paths or []:
            if boundary:
                require(match[1] == 'pub' and path in UI_EXPORTS,
                        'renderer contract has an unreviewed data export/import')
                exports.add(path)
            else:
                allowed = (len(path) >= 2 and path[:2] == ('talk_core', 'ui')) or (
                    len(path) >= 2 and path[0] == 'std'
                    and path[1] not in {'net', 'fs', 'process', 'thread', 'os'}
                )
                require(allowed, 'presentation import must use the core UI boundary or local formatting')
    if boundary:
        require(exports == UI_EXPORTS, 'renderer data exports differ from the reviewed vocabulary')
    remainder = USE.sub(' ', source)
    require(not re.search(r"\b(?:use|extern\s+crate)\b", remainder),
            'uninspected boundary import syntax')
    return remainder


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
    require(not re.search(UNBOUNDED, boundary),
            "renderer contract exposes an unbounded container type")
    require(not re.search(WORKER_OR_FRAME, boundary),
            "renderer contract exposes a secret, worker or frame type")
    require(not re.search(r"\b(?:secrets?|credentials?|tokens?|passwords?|http|authorization)\b", boundary, re.I),
            "renderer contract contains secret/transport-bearing identifiers")
    boundary_body = checked_imports(boundary, boundary=True)
    require(not re.search(r"\b(?:crate|super|talk_core|talk_adapters)\s*::|\bmod\b|\w+\s*!", boundary_body),
            "renderer contract reaches outside reviewed data exports or generates uninspected types")
    presentation = without_comments((ROOT / "apps/bootstrap/src/presentation.rs").read_text(encoding="utf-8"))
    presentation_body = checked_imports(presentation, boundary=False)
    require(not re.search(r"talk_adapters|talk_core\s*::\s*(?!ui\b)\w+|crate\s*::|super\s*::|std\s*::\s*(?:net|fs|process|thread|os)|\b(?:include|include_str|include_bytes)\s*!|\bmod\s+\w+\s*;", presentation_body),
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
