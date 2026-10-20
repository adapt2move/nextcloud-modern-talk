# Native UI boundaries

The GPUI desktop has an ordinary account-connected mode and explicitly labelled fictional development scenarios. Local settings are persisted. See [current limitations](../../status.md) for unfinished server/platform behavior. Keyboard styling does not establish native semantic accessibility or VoiceOver support.

## Responsibilities

The composition layer owns window lifetime, embedded assets and the injected application port. Presentation consumes `talk_core::ui` snapshots and typed intents. Views do not import protocol DTOs, adapters, credentials, poll/login URLs, media frames or worker APIs. The application owner projects domain state, maintains policy freshness and clears denied content across every surface.

The [product specifications](../../product/README.md), [flows](../flows.md) and [tokens](../tokens.md) govern intended behavior. Mockups remain illustrative. Use the [development guide](../../development.md#native-ui-entrypoint) for the native entrypoint; launch commands do not imply a current integrated build pass.

## State and resource boundaries

Snapshots, input strings, drafts, focus identities and completion retention must have finite capacity and explicit account/runtime/host/conversation scope. Removed controls retire their handles; stale completions cannot overwrite a newer state. Busy means no admission. Uncertain sends cannot be replayed as if they failed safely. Keep detailed limits and implementation rationale with the owning code and its API documentation.

A draft clears only after its matching server-accepted result if the user has not edited it since submission. Server acceptance does not mean delivery or read. Unknown, stale or denied policy keeps gated actions unavailable. A preview cannot invent a successful backend operation or grant device capture.

Update activation carries the displayed release generation and is hidden during nonterminal calls. Before restart, every retained draft and unresolved submission, the current editor and any active text composition must be clear. Once the composition owner admits installation, presentation immediately makes editors and ordinary actions inert while leaving native window close available. A later install failure does not reopen drained services; the composition owner provides the native restart/quit outcome.

## Settings contract

Blue/System/100%/Comfortable are the intended defaults. Persistent settings require a bounded owner, correlated revisions, one writer and latest-value coalescing. Under [DESK-004](../../product/desktop-and-accessibility.md), a failed write restores the last confirmed active/displayed values, explains the failure and offers explicit Retry. A bounded retry draft is non-authoritative; a stale failure must not replace a newer confirmed revision. No view performs file I/O. These invariants apply to every preference, including platform effects with their own failure states.

## Input, privacy and native qualification

Keep server/search/creation input and draft text distinct from backend snapshots. Clipboard admission for drafts is an explicit privacy-policy decision; generic send permission is insufficient. GPUI 0.2.2 materializes the platform clipboard String before a widget can check its size, so that transient allocation remains an explicit resource limitation.

Bounded multiline plain-text editing, IME integration, focus restoration and modal isolation each need appropriate checks. Actual language/IME sessions, localization, bidi handling, macOS permissions, native accessibility roles/announcements and VoiceOver remain unqualified. Direct incoming acceptance skips voluntary conference prejoin while preserving mandatory permission, consent and security gates.

Functional icons use pinned official Lucide SVGs; the application mark remains separate project artwork. Embedded [icon provenance](../../../apps/desktop/assets/icons/PROVENANCE.md) and font licenses remain authoritative. Preserve Manrope's OFL notice. A native screenshot must identify its actual source/platform and fictional data; a browser mockup or static contrast calculation does not establish integrated rendering or accessibility.


Settings/sidebar composition uses one platform AccountConnection for ordinary launches and an explicitly labelled fictional port for development scenarios. Presentation imports bounded UI/preferences values only. Account preferences, archive, favorites and tags are correlated through the admitted account owner; old-scope results cannot populate a replacement account. A send allocates one operation ID and retains it through uncertain status checks. Native permission and device runtimes remain separate owners: permission checks never capture, and preview replacement waits for disposal acknowledgement.
