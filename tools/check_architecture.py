#!/usr/bin/env python3
"""Enforce the reviewed three-layer graph and renderer boundary; Python 3.11+.

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

from vendor import verify_gpui

ROOT = Path(__file__).resolve().parents[1]
GRAPH = {
    "talk-core": ("crates/talk-core", set()),
    "nextcloud-talk-api": ("crates/nextcloud-talk-api", {"reqwest", "tokio", "serde", "serde_json", "url", "zeroize", "cookie", "futures-lite", "futures-util", "tokio-tungstenite", "rustls", "rustls-native-certs"}),
    "talk-adapters": ("crates/talk-adapters", {"talk-core", "nextcloud-talk-api", "reqwest", "tokio", "serde", "serde_json", "url", "zeroize", "getrandom", "keyring-core", "rusqlite", "dirs", "semver", "self_update", "sha2", "tempfile", "flate2", "tar", "zip", "webrtc", "rtc", "async-trait", "gstreamer", "gstreamer-app", "gstreamer-video", "futures-util"}),
    "talk-bootstrap": ("apps/bootstrap", {"talk-core", "talk-adapters"}),
    "talk-desktop": ("apps/desktop", {"talk-core", "talk-adapters", "gpui", "unicode-segmentation", "atomicwrites", "cpal", "gstreamer", "gstreamer-app", "gstreamer-video", "image"}),
}
PLATFORM_EDGES = {
    'cfg(unix)': {'rustix'},
    'cfg(target_os = "linux")': {'zbus-secret-service-keyring-store', 'dbus-secret-service', 'secret-service', 'zbus'},
    'cfg(target_os = "macos")': {'apple-native-keyring-store', 'exacl', 'security-framework'},
    'cfg(target_os = "windows")': {'windows-native-keyring-store', 'windows-permissions', 'winapi-util', 'atomicwrites'},
}
DESKTOP_PLATFORM_EDGES = {
    'cfg(unix)': {'rustix'},
    'cfg(target_os = "macos")': {'core-graphics', 'exacl'},
    'cfg(target_os = "linux")': {'ashpd', 'async-io', 'futures-lite'},
    'cfg(target_os = "windows")': {'enigo', 'windows', 'windows-permissions'},
}
failures: list[str] = []


def require(condition: bool, message: str) -> None:
    if not condition:
        failures.append(message)


def read_toml(path: Path) -> dict:
    return tomllib.loads(path.read_text(encoding="utf-8"))


# Order matters: quotes inside comments/chars and raw strings are not string openers.
# Match a complete character literal, never an apostrophe starting a Rust lifetime.
RUST_LITERAL_OR_COMMENT = re.compile(
    r'(?P<raw>\b(?:br|cr|r)(?P<hashes>#*)".*?"(?P=hashes))'
    r'|(?P<string>(?:b|c)?"(?:\\.|[^"\\])*")'
    r"|(?P<char>b?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|[^\n])|[^'\\\n])')"
    r'|(?P<comment>//[^\n]*|/\*.*?\*/)', re.S,
)


def blank(source: str, fill: str = ' ') -> str:
    return ''.join('\n' if char == '\n' else fill for char in source)


def without_comments(source: str) -> str:
    # Preserve string contents so endpoint literals cannot hide in an accepted interface.
    # Nested block comments are forbidden here instead of being parsed incorrectly.
    require(not re.search(r"/\*(?:(?!\*/).)*?/\*", source, re.S),
            "nested Rust block comments need a parser-aware boundary check")
    return RUST_LITERAL_OR_COMMENT.sub(
        lambda match: blank(match[0]) if match.group('comment') else match[0], source)


def import_source(source: str) -> str:
    # Keep offsets aligned with the original. A non-whitespace sentinel prevents a
    # literal inside malformed import syntax from becoming an accepted explicit path.
    return RUST_LITERAL_OR_COMMENT.sub(
        lambda match: blank(match[0], ' ' if match.group('comment') else '\0'), source)


def without_imports(source: str) -> str:
    body = list(source)
    for match in USE.finditer(import_source(source)):
        # Never erase a literal caught between an incomplete `use` and a later `;`:
        # content rules must still see every original literal, including its quotes.
        if '\0' not in match[0]:
            body[match.start():match.end()] = blank(match[0])
    return ''.join(body)


def allow_input_caret_timer(source: str) -> str:
    """Scope one foreground visual timer without exempting its other calls.

    The input owner retains/cancels the task and tests its focus/generation bounds.
    This lexical exception is not a proof of those lifecycle properties.
    """
    code = import_source(source)
    starts = list(re.finditer(r"\bfn\s+schedule_caret_blink\s*\([^;{}]*\)\s*\{", code))
    if not starts:
        return source
    require(len(starts) == 1, "native input must have one caret timer owner")
    if len(starts) != 1:
        return source
    start = starts[0].end()
    depth, end = 1, start
    while depth and end < len(code):
        depth += (code[end] == '{') - (code[end] == '}')
        end += 1
    require(depth == 0, "native input caret timer method is not closed")
    if depth:
        return source
    calls = list(re.finditer(r"\bcx\s*\.\s*spawn_in\s*\(", code[start:end - 1]))
    require(len(calls) == 1, "native input caret timer must own one foreground task")
    if len(calls) != 1:
        return source
    call = calls[0]
    begin, finish = start + call.start(), start + call.end()
    return source[:begin] + "cx.reviewed_input_caret_timer(" + source[finish:]


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
# These modules contain only recursively inspected bounded presentation values.
UI_MODULE_TYPES = {
    "presentation": {"Completion", "ConversationRef", "FeatureSupport", "RequestId", "MessageView"},
    "conversation": {
        "Availability", "RemoteState", "ConversationPanel", "ConversationAction",
        "AccountPersonRef", "AccountPerson", "CreateAction", "CreationResult", "CreatePanel",
    },
    "messaging": {"MessageMetadata", "MessagePanel", "MessageAction"},
    "content": {"ContentPanel", "ContentAction"},
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
    for match in USE.finditer(import_source(source)):
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
    remainder = without_imports(source)
    require(not re.search(r"\b(?:use|extern\s+crate)\b", import_source(remainder)),
            'uninspected boundary import syntax')
    return remainder


def check() -> None:
    workspace = read_toml(ROOT / "Cargo.toml")["workspace"]
    require(set(workspace["members"]) == {path for path, _ in GRAPH.values()},
            "workspace members differ from the reviewed reviewed API/adapter/core/presentation graph")
    require(workspace["package"]["license"] == "EUPL-1.2", "workspace license changed")
    require(workspace["lints"]["rust"]["unsafe_code"] == "forbid", "unsafe policy changed")
    shared = workspace.get("dependencies", {})
    reviewed = {
        "talk-core": {"path": "crates/talk-core"},
        "nextcloud-talk-api": {"path": "crates/nextcloud-talk-api"},
        "talk-adapters": {"path": "crates/talk-adapters"},
        "webrtc": {"version": "=0.21.1", "default-features": False, "features": ["runtime-tokio", "crypto-ring"]},
        "rtc": {"version": "=0.21.1", "default-features": False},
        "async-trait": "=0.1.89",
        "futures-util": {"version": "=0.3.34", "default-features": False, "features": ["std"]},
        "tokio-tungstenite": {"version": "=0.30.0", "default-features": False, "features": ["connect", "rustls-tls-native-roots"]},
        "rustls": {"version": "=0.23.46", "default-features": False, "features": ["std", "aws_lc_rs", "tls12"]},
        "rustls-native-certs": {"version": "=0.8.5", "default-features": False},
        "reqwest": {"version": "=0.13.5", "default-features": False, "features": ["rustls", "form", "stream"]},
        "tokio": {"version": "=1.53.3", "default-features": False, "features": ["rt", "time", "sync", "macros", "net", "io-util", "fs"]},
        "serde": {"version": "=1.0.229", "default-features": False, "features": ["std"]},
        "serde_json": {"version": "=1.0.154", "default-features": False, "features": ["std"]},
        "url": {"version": "=2.5.8", "default-features": False, "features": ["std"]},
        "zeroize": {"version": "=1.9.1", "default-features": False, "features": ["alloc"]},
        "getrandom": {"version": "=0.4.3", "default-features": False, "features": ["std"]},
        "cookie": {"version": "=0.18.1", "default-features": False},
        "semver": {"version": "=1.0.28", "default-features": False, "features": ["std"]},
        'self_update': {'version': '=1.3.1', 'default-features': False, 'features': ['reqwest', 'rustls', 'checksums', 'archive-tar', 'compression-tar-gz', 'archive-zip', 'compression-zip-deflate']},
        'sha2': {'version': '=0.11.1', 'default-features': False},
        'tempfile': {'version': '=3.27.0', 'default-features': False, 'features': ['getrandom']},
        'flate2': {'version': '=1.1.10', 'default-features': False, 'features': ['rust_backend']},
        'tar': {'version': '=0.4.46', 'default-features': False},
        'zip': {'version': '=9.0.4', 'default-features': False, 'features': ['deflate']},
        "gpui": {"version": "=0.2.2", "default-features": False, "features": ["font-kit", "x11"]},
        "unicode-segmentation": "=1.12.0",
        "cpal": {"version": "=0.18.2", "default-features": False, "features": ["pulseaudio"]},
        "gstreamer": {"version": "=0.25.4", "default-features": False, "features": ["v1_24"]},
        "gstreamer-app": {"version": "=0.25.2", "default-features": False, "features": ["v1_24"]},
        "gstreamer-video": {"version": "=0.25.4", "default-features": False, "features": ["v1_24"]},
        "image": {"version": "=0.25.10", "default-features": False, "features": ["png", "jpeg", "webp", "gif"]},
        "enigo": {"version": "=0.6.1", "default-features": False},
        "windows": {"version": "=0.61.3", "default-features": False, "features": ["UI_ViewManagement"]},
        "core-graphics": {"version": "=0.24.0", "default-features": False, "features": ["link"]},
        "ashpd": {"version": "=0.12.3", "default-features": False, "features": ["async-std"]},
        "async-io": {"version": "=2.6.0", "default-features": False},
        "futures-lite": {"version": "=2.6.1", "default-features": False, "features": ["std"]},
        "keyring-core": {"version": "=1.0.1", "default-features": False},
        "rusqlite": {"version": "=0.40.2", "default-features": False, "features": ["bundled", "limits"]},
        "dirs": {"version": "=5.0.1", "default-features": False},
        "rustix": {"version": "=1.1.5", "default-features": False, "features": ["std", "fs", "process"]},
        "apple-native-keyring-store": {"version": "=1.0.2", "default-features": False, "features": ["keychain"]},
        "windows-native-keyring-store": {"version": "=1.1.0", "default-features": False},
        "zbus-secret-service-keyring-store": {"version": "=1.0.1", "default-features": False, "features": ["crypto-rust"]},
        "dbus-secret-service": {"version": "=4.1.0", "default-features": False, "features": ["crypto-rust"]},
        "secret-service": {"version": "=5.2.0", "default-features": False, "features": ["crypto-rust"]},
        "zbus": {"version": "=5.19.0", "default-features": False, "features": ["blocking-api", "async-io"]},
        "exacl": {"version": "=0.13.0", "default-features": False},
        "security-framework": {"version": "=3.7.0", "default-features": False},
        "windows-permissions": {"version": "=0.2.4", "default-features": False},
        "winapi-util": {"version": "=0.1.11", "default-features": False},
        "atomicwrites": {"version": "=0.4.4", "default-features": False},
    }
    require(shared == reviewed, "shared dependency pins/features differ from reviewed network/native plan")
    require(set(workspace) <= {"members", "exclude", "resolver", "package", "dependencies", "lints"},
            "unreviewed workspace patch/target/configuration")
    require(workspace.get("exclude") == ["vendor/gpui"], "unreviewed workspace exclusions")
    root = read_toml(ROOT / "Cargo.toml")
    require(set(root) == {"workspace", "patch"}, "unreviewed root manifest section")
    require(root.get("patch") == {"crates-io": {"gpui": {"path": "vendor/gpui"}}},
            "only the source-bound GPUI renderer patch is allowed")
    try:
        verify_gpui(ROOT)
    except (ValueError, KeyError, TypeError, OSError) as error:
        require(False, "vendored GPUI source verification failed: " + str(error))

    for name, (directory, expected) in GRAPH.items():
        manifest = read_toml(ROOT / directory / "Cargo.toml")
        require(manifest["package"]["name"] == name, f"{directory}: unexpected package name")
        require(set(manifest) <= ({"package", "dependencies", "lints", "features", "example", "target", "dev-dependencies"} if name == "talk-adapters" else {"package", "dependencies", "lints", "features"} if name == "nextcloud-talk-api" else {"package", "dependencies", "lints", "target"} if name == "talk-desktop" else {"package", "dependencies", "lints"}),
                f"{name}: unreviewed manifest section")
        package_fields = {"name", "version", "edition", "rust-version", "license", "publish"}
        if name == "talk-adapters":
            package_fields.add("autoexamples")
            require(manifest["package"].get("autoexamples") is False,
                    "adapter examples must use the explicit default-off allowlist")
        require(set(manifest["package"]) == package_fields,
                f"{name}: unreviewed package/build configuration")
        for field in ("version", "edition", "rust-version", "license", "publish"):
            require(manifest["package"].get(field) == {"workspace": True},
                    f"{name}: {field} must inherit the workspace")
        require(manifest.get("lints") == {"workspace": True}, f"{name}: inherit workspace lints")
        deps = manifest.get("dependencies", {})
        require(set(deps) == expected, f"{name}: forbidden/missing dependency edge")
        local_features = {("talk-adapters", "tokio"): ["process"], ("nextcloud-talk-api", "futures-util"): ["sink"]}
        require(all(value == ({"workspace": True, "features": local_features[(name, dep)]} if (name, dep) in local_features else {"workspace": True}) for dep, value in deps.items()),
                f"{name}: dependencies must use reviewed workspace definitions")
        require("build-dependencies" not in manifest and manifest.get("dev-dependencies", {}) == ({"nextcloud-talk-api": {"workspace": True, "features": ["backend-test"]}} if name == "talk-adapters" else {}),
                f"{name}: hidden platform/build/test edges need review")
        platform_edges = PLATFORM_EDGES if name == "talk-adapters" else DESKTOP_PLATFORM_EDGES if name == "talk-desktop" else {}
        platform = {target: {"dependencies": {dep: {"workspace": True} for dep in dependencies}}
                    for target, dependencies in platform_edges.items()}
        require(manifest.get("target", {}) == platform,
                f"{name}: hidden platform/build/test edges need review")
        if name == "talk-adapters":
            require(manifest.get("features") == {"default": [], "backend-test": ["nextcloud-talk-api/backend-test"]}, "fixture HTTP must remain explicitly opt-in")
            require(manifest.get("example") == [
                {"name": "media-talk-bridge", "path": "../../tools/media/talk_bridge.rs", "required-features": ["backend-test"]},
                {"name": "identity-i2-lab", "path": "../../tests/identity/driver.rs", "required-features": ["backend-test"]},
                {"name": "media-native-peer", "path": "../../tools/media/native_peer.rs", "required-features": ["backend-test"]},
                {"name": "media-normal-talk", "path": "../../tools/media/normal_talk.rs", "required-features": ["backend-test"]},
                {"name": "update-install-probe", "path": "../../tools/updates/install_probe.rs", "required-features": ["backend-test"]},
            ], "only explicit default-off synthetic diagnostic executables are reviewed")
        if name == "nextcloud-talk-api":
            require(manifest.get("features") == {"default": [], "backend-test": []}, "API diagnostics must remain explicitly opt-in")
        require(not (ROOT / directory / "build.rs").exists(), f"{name}: unreviewed build script")

    manifests = {str(path.relative_to(ROOT)) for base in ("crates", "apps")
                 for path in (ROOT / base).rglob("Cargo.toml")}
    require(manifests == {f"{path}/Cargo.toml" for path, _ in GRAPH.values()},
            "unreviewed package outside workspace membership")
    lock = read_toml(ROOT / "Cargo.lock")
    require(lock.get("version") == 4, "unexpected lockfile format")
    packages = lock.get("package", [])
    local = [p for p in packages if "source" not in p]
    require({p["name"] for p in local} == set(GRAPH) | {"gpui"} and len(local) == len(GRAPH) + 1,
            "lockfile local packages differ from reviewed layers")
    for package in local:
        name = package["name"]
        if name == "gpui":
            require(package.get("version") == "0.2.2" and "checksum" not in package,
                    "vendored GPUI lock identity differs from the source pin")
            continue
        require(package.get("version") == workspace["package"]["version"], f"{name}: lockfile version mismatch")
        platform_edges = PLATFORM_EDGES if name == "talk-adapters" else DESKTOP_PLATFORM_EDGES if name == "talk-desktop" else {}
        expected_edges = GRAPH.get(name, (None, set()))[1] | set().union(*platform_edges.values())
        require({dep.split()[0] for dep in package.get("dependencies", [])} == expected_edges,
                f"{name}: lockfile dependency mismatch")
    for package in packages:
        if "source" in package:
            require(package["source"] == "registry+https://github.com/rust-lang/crates.io-index"
                    and re.fullmatch(r"[0-9a-f]{64}", package.get("checksum", "")) is not None,
                    "third-party lock entries require registry checksums and reviewed provenance")
    require(any(p["name"] == "libc" and p["version"] == "0.2.186" for p in packages),
            "GPUI/xattr compatibility pin missing")
    for name, entry in reviewed.items():
        if name in {"talk-core", "talk-adapters", "nextcloud-talk-api"}:
            continue
        version = (entry if isinstance(entry, str) else entry["version"]).removeprefix("=")
        require(any(p["name"] == name and p["version"] == version for p in packages), f"{name}: locked direct pin missing")

    for path in (ROOT / "crates/nextcloud-talk-api/src").rglob("*.rs"):
        source = without_comments(path.read_text(encoding="utf-8"))
        require(not re.search(r"\b(?:talk_core|talk_adapters|gpui|keyring_core|rusqlite|cpal|gstreamer|ashpd)\b|crate\s*::\s*media", source),
                f"{path.relative_to(ROOT)}: API contains application/storage/native-media dependency")

    for path in (ROOT / "crates/talk-core/src").rglob("*.rs"):
        source = without_comments(path.read_text(encoding="utf-8"))
        require(not re.search(r"\b(?:nextcloud_talk_api|gpui|core_graphics|ashpd|async_io|futures_lite|enigo|reqwest|hyper|tokio|talk_adapters|keyring_core|rusqlite|zbus|secret_service|dbus_secret_service|dirs|rustix|exacl|security_framework|windows_permissions|winapi_util|atomicwrites)\b|windows\s*::|std\s*::\s*(?:net|fs|process|thread)", source),
                f"{path.relative_to(ROOT)}: core contains adapter/runtime I/O")
        # User-entered server input is not validated authority; only these scheme-test
        # literals are allowed in its reviewed UI value type. Actual URLs/routes still fail.
        routes = source.replace('"https://"', '"scheme"').replace('"http://"', '"insecure-scheme"') if path == ROOT / "crates/talk-core/src/ui/presentation.rs" else source
        require(not re.search(r"https?://|/ocs/|/remote\.php/", routes),
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
    for module in UI_MODULE_TYPES:
        boundary_body = boundary_body.replace(f"pub mod {module};", "")
    require(not re.search(r"\b(?:crate|super|talk_core|talk_adapters)\s*::|\bmod\b|\w+\s*!", boundary_body),
            "renderer contract reaches outside reviewed data exports or generates uninspected types")
    ui_files = sorted((ROOT / "crates/talk-core/src/ui").rglob("*.rs"))
    require({str(p.relative_to(ROOT)) for p in ui_files} == {f"crates/talk-core/src/ui/{name}.rs" for name in UI_MODULE_TYPES},
            "new renderer data module needs explicit review")
    for path in ui_files:
        source = without_comments(path.read_text())
        require(not re.search(UNBOUNDED, source), "renderer contract exposes an unbounded container type")
        require(not re.search(WORKER_OR_FRAME, source), "renderer contract exposes a secret, worker or frame type")
        for match in USE.finditer(import_source(source)):
            paths = use_paths(match[2])
            require(paths is not None, "boundary imports need explicit paths without aliases/globs")
            for imported in paths or []:
                allowed = len(imported) == 2 and imported[0] == "super" and imported[1] in {
                    "AccountId", "AccountScope", "CallScope", "ConversationId", "Decision", "MediaIntent", "Page", "SubmitError", "Text", "UnavailableReason", "UserIntent", "AddressInputError", "ServerInput"
                }
                allowed |= imported in {
                    ("crate", "preferences", "navigation", "NavigationAccountKey"),
                    ("crate", "preferences", "navigation", "NavigationRoomKey"),
                }
                allowed |= (len(imported) == 3 and imported[0] == "super"
                            and imported[2] in UI_MODULE_TYPES.get(imported[1], set()))
                require(allowed, "renderer contract has an unreviewed data export/import")
        body = without_imports(source)
        for module, names in UI_MODULE_TYPES.items():
            for name in names:
                body = re.sub(r"\bsuper\s*::\s*" + module + r"\s*::\s*" + name + r"\b", "BoundedUiValue", body)
        body = re.sub(r"#\[cfg\(test\)\]\s*mod tests\s*\{", "{", body)
        require(not re.search(r"\b(?:crate|talk_core|talk_adapters|super)\s*::|\bmod\b|\bmacro_rules!|\binclude(?:_str|_bytes)?!", body),
                "renderer submodule bypasses reviewed data vocabulary")
        require(not re.search(r"\b(?:password|authorization|credential|Secret|Token|Frame)\b", source),
                "renderer contract contains secret/transport-bearing identifiers")

    native = ROOT / "apps/desktop/src"
    for path in native.rglob("*.rs"):
        relative = path.relative_to(native)
        require(relative.parts[0] in {"main.rs", "bundle.rs", "composition.rs", "composition", "presentation", "settings.rs", "settings", "devices.rs", "devices", "diagnostics.rs", "diagnostics"},
                "unreviewed native application module")
    for path in (native / "presentation").rglob("*.rs"):
        source = without_comments(path.read_text())
        for match in USE.finditer(import_source(source)):
            # GPUI's reviewed layout prelude and local test/view imports may use globs.
            expression = match[2].replace("prelude::*", "prelude::ReviewedPrelude").replace("super::*", "super::ReviewedLocal")
            paths = use_paths(expression)
            require(paths is not None, "native presentation imports require explicit paths (except GPUI prelude/local tests)")
            for imported in paths or []:
                allowed = imported[:2] in {("talk_core", "ui"), ("talk_core", "preferences")} or imported[:2] == ("crate", "presentation") or imported[0] in {"gpui", "unicode_segmentation", "input", "state", "theme"}
                allowed |= imported[0] == "std" and len(imported) > 1 and imported[1] in {"ops", "borrow", "fmt", "time", "cmp", "mem", "convert", "str", "collections"}
                allowed |= imported[0] == "super" and len(imported) > 1 and imported[1] not in {"super", "composition"}
                # Bounded native focus/message geometry and finite presentation deadlines.
                allowed |= path in {native / "presentation/navigation.rs", native / "presentation/messaging.rs"} and imported in {
                    ("std", "cell", "RefCell"), ("std", "rc", "Rc")
                }
                require(allowed, "native presentation import must use core UI, GPUI or local view types")
        require(not re.search(r"nextcloud_talk_api|talk_adapters|talk_core\s*::\s*(?!(?:ui|preferences)\b)\w+|\b(?:core_graphics|ashpd|async_io|futures_lite|enigo|reqwest|hyper|tokio|cookie|serde_json|zeroize|sqlite|rusqlite|keyring_core|zbus|secret_service|dbus_secret_service|dirs|rustix|exacl|security_framework|windows_permissions|winapi_util|atomicwrites|cpal|gstreamer|gstreamer_app|gstreamer_video)\b|\b(?:image|windows)\s*::|(?:std|gpui)\s*::\s*(?:net|fs|process|thread|os)|crate\s*::\s*(?!presentation\b)\w+|super\s*::\s*(?:super|composition)|\b(?:include|include_str|include_bytes)\s*!|https?://|/ocs/|/remote\.php/", source),
                "native presentation contains worker/secret/network/file/media I/O or route")
        # Native user-entry clipboard only. GPUI's read allocation is not bounded;
        # admission and privacy gating belong to the reviewed input widget/producer.
        hidden_io = source
        if path == native / "presentation/input.rs":
            hidden_io = allow_input_caret_timer(hidden_io)
            hidden_io = re.sub(r"\.(?:read_from_clipboard|write_to_clipboard)\s*\(", ".reviewed_input_clipboard(", hidden_io)
        if path == native / "presentation/conversation_dialog.rs":
            # Explicit user Copy guest-link action; sensitive/classified rooms refuse it.
            # Clipboard reads, arbitrary OS operations and other views stay forbidden.
            hidden_io = re.sub(r"\.write_to_clipboard\s*\(", ".reviewed_guest_link_copy(", hidden_io)
        require(not re.search(r"\.(?:http_client|background_executor|spawn|spawn_in|read_to_end|open_url|read_from_clipboard|write_to_clipboard)\s*\(", hidden_io),
                "native presentation cannot dispatch hidden I/O; submit an intent")

    presentation = without_comments((ROOT / "apps/bootstrap/src/presentation.rs").read_text(encoding="utf-8"))
    presentation_body = checked_imports(presentation, boundary=False)
    require(not re.search(r"nextcloud_talk_api|talk_adapters|talk_core\s*::\s*(?!ui\b)\w+|crate\s*::|super\s*::|std\s*::\s*(?:net|fs|process|thread|os)|\b(?:include|include_str|include_bytes)\s*!|\bmod\s+\w+\s*;", presentation_body),
            "presentation must depend only on the core UI boundary and bounded local formatting")


try:
    check()
except (OSError, KeyError, TypeError, ValueError) as error:
    failures.append(f"cannot inspect architecture: {error}")

if failures:
    for failure in failures:
        print(f"architecture: {failure}", file=sys.stderr)
    raise SystemExit(1)
print("architecture: reviewed three-layer dependency and recursive presentation boundary rules hold")
