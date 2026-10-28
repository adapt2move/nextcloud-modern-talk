# Architecture

NextCloud Modern Talk is a Rust workspace with native GPUI presentation. The
workspace manifests, `Cargo.lock` and `rust-toolchain.toml` are the build inputs;
[development](../development.md) covers prerequisites and checks.

## Module boundaries

| Owner | Responsibility |
| --- | --- |
| `nextcloud-talk-api` | Origin-bound HTTP/OCS, typed protocol values, signaling and file transport |
| `talk-core` | Domain policy, account/conversation/call identities, bounded UI snapshots and typed intents |
| `talk-adapters` | Credential providers, persistence transactions, network integration, updates and media engine |
| `apps/desktop` | Application composition, native devices, settings, GPUI windows and presentation |
| `apps/bootstrap` | Headless bootstrap and contract checks |

A consequential resource has one owner with a small interface. Extend that owner
before adding facade layers, event buses or feature-specific crates. Views do no
network, database, credential or media I/O. The API crate has no application-policy
or native-device dependency. Rust application code forbids unsafe code; native FFI
stays in reviewed dependencies.

[Architecture checks](../../tools/check_architecture.py) enforce dependency
boundaries, feature selection, source provenance and presentation restrictions.
[Boundary fixtures](../../tests/contracts/README.md) test intended rejection paths.
These lexical checks supplement compilation and behavioral tests; they are not a sandbox.

## Native UI

GPUI owns rendering, focus and window lifetimes. Composition injects bounded
snapshots and typed intents/results into presentation. No embedded WebView is used;
Login Flow v2 opens the system browser. Collections are paged or virtualized, and
input entities own cancellable caret tasks. Frames never enter chat serialization.

The local GPUI patch and bundled assets retain exact provenance in
[UPSTREAM.json](../../vendor/gpui/UPSTREAM.json) and [NOTICE.md](../../NOTICE.md).
See [design](../design/README.md) for UI rules. Compilation does not establish
accessibility, IME, device or permission support on another platform.

## Accounts and storage

Every operation carries its account/runtime generation and narrower conversation
or call scope. Retired completions cannot restore authority or update a new scope.
The account owner coordinates login, restoration, cancellation, logout and cleanup.
Normal sign-in is durable: prefer the native credential store, with an explicitly
selected **unencrypted private-file** fallback when native storage is unavailable.
There is no silent provider migration or master-password setup.

Use macOS Keychain, Windows Credential Manager or Linux Secret Service through the
platform adapter. Private files require verified per-user ownership and restrictive
POSIX permissions or Windows DACLs. The committed ledger names the exact eligible
provider/item; enumeration and a successful write alone cannot authorize restoration.
Retain the previous committed credential until replacement is confirmed. Late or
uncertain writes require settlement, and deletion failure remains visible. Logout
suppresses restoration account-wide, including residual items in another provider.

The detailed [identity contract](identity-contract.md), [account requirements](../product/accounts.md)
and [security requirements](../quality/security.md) own transaction, crash recovery,
file-access and cancellation invariants. Implementation rationale belongs beside code.

## Transport

`nextcloud-talk-api` owns bounded HTTP/TLS/OCS requests, canonical origins, response
limits, timeouts and typed errors. Credentials are confined to the authorized origin;
redirects, downloads and URLs supplied by servers cannot broaden that authority.
Errors preserve safe context without credentials or message contents.

Adapters map protocol observations to application policy. Mutations with uncertain
outcomes use bounded read-only reconciliation instead of blind retries. A room-level
denial, timeout or rate limit is not account revocation. The [integration contract](nextcloud-contract.md)
defines admission, completion and shutdown; [protocol sources](source-evidence.md)
record the upstream references.

## Policy and lifetime

Capabilities, configuration, current permissions, session state and explicit user
intent determine available actions. A server version or cached snapshot grants no
permission. Policy observations have finite freshness; unknown, restrictive or expired
state denies the affected operation. Hash-only responses cannot renew omitted policy.

Account rejection invalidates its generation, stops new authenticated work and closes
local media without waiting for server leave or credential cleanup. Room restrictions
act on their narrower scope. Resume revalidates authority and devices before use.
Every queue/cache/operation has finite capacity, lifetime, cancellation and overload
behavior. Preserve durable intents and required transitions when rejecting new work.

## Native media

The in-process `webrtc` engine and same-version `rtc` core own SDP, ICE, DTLS/SRTP
and SCTP. Safe GStreamer bindings provide codec and frame conversion; desktop device
owners use CPAL for audio and GStreamer for cameras. Account/call generations bind
signaling, publication grants, devices and frames. Leaving or losing authority closes
publication irreversibly; remote leave and device disposal are separate outcomes.

Selecting a conversation or ringing never opens devices. Explicit preview is separate
from call publication. Required E2EE and unsupported profiles fail closed. See
[known limitations](../limitations.md) for the actual supported profile.

The [legacy native helper](../../native/media/README.md) is diagnostic-only. Its
process isolation and allocation model do not apply to the shipping engine.
Application frame/queue bounds do not cap decoder, OS or GPU allocations. Complete
resource claims require the [measurement contract](measurement.md) and
[performance requirements](../quality/performance.md).
