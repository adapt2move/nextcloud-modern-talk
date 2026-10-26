# Native UI boundaries

The GPUI desktop has an ordinary account-connected mode and explicitly labelled fictional development scenarios. Local settings are persisted. See [current limitations](../../status.md) for unfinished server/platform behavior. Keyboard styling does not establish native semantic accessibility or VoiceOver support.

## Responsibilities

The composition layer owns window lifetime, embedded assets and the injected application port. Presentation consumes `talk_core::ui` snapshots and typed intents. Views do not import protocol DTOs, adapters, credentials, poll/login URLs, media frames or worker APIs. The application owner projects domain state, maintains policy freshness and clears denied content across every surface.

The private controls module owns buttons, icon actions, switches, selection controls and shared form surfaces/rows/sections. Forms compose retained input entities with those surfaces instead of duplicating border, padding, pointer and focus behavior. Each input owns one cancellable caret task using GPUI’s platform-dispatched timer; paint receipts prevent an unmounted editor from continuing its timer. The settings worker reads macOS Reduce Motion through the pinned safe AppKit binding, including for `cargo run`; no platform call enters a view.

Composite fields own one background and border; transparent editors preserve selection and caret behavior. Focus strengthens the existing border inward without an additional outer outline. Initial collection loads use static bounded skeletons; refresh keeps confirmed content mounted without a routine loading banner.

The conversation header applies a 16-point renderer backdrop blur, with tint opacity from 84% to 80%, before painting its controls. Windows retains its opaque white fallback. Blur does not blur child text, create an idle redraw loop or allow hidden rows to receive input.

The same content-card component frames conversations, Inbox, onboarding and call surfaces. It owns the 12-point radius and subtle outward elevation, with the shadow outside the content's overflow mask. Full-bleed fills and the chat backdrop share that radius at exposed corners because GPUI's overflow mask is rectangular. Modal dialogs retain their separate geometry and elevation.

The [product specifications](../../product/README.md), [flows](../flows.md) and [tokens](../tokens.md) govern intended behavior. Mockups remain illustrative. Use the [development guide](../../development.md#native-ui-entrypoint) for the native entrypoint; launch commands do not imply a current integrated build pass.

## State and resource boundaries

Snapshots, input strings, drafts, focus identities and completion retention must have finite capacity and explicit account/runtime/host/conversation scope. Removed controls retire their handles; stale completions cannot overwrite a newer state. Busy means no admission. Uncertain sends cannot be replayed as if they failed safely. Keep detailed limits and implementation rationale with the owning code and its API documentation.

A draft or edit clears only after its matching queue-staged or server-accepted result, if the user has not edited it since submission. Durable queue staging requires a committed transaction; local acceptance does not mean server acceptance, delivery or read. Supported message actions may stage under unknown cached policy when their scoped queue is available, while known denial and unsupported behavior remain unavailable. The owner validates fresh authority before network dispatch. A preview cannot invent a successful backend operation or grant device capture.

Update activation carries the displayed release generation and is hidden during nonterminal calls. Before restart, unsent drafts, unfinished staging, the current editor and any active text composition must be clear. Already committed durable intents may remain in their account queue; drain local storage work and preserve dispatch uncertainty across restart. Once the composition owner admits installation, presentation immediately makes editors and ordinary actions inert while leaving native window close available. A later install failure does not reopen drained services; the composition owner provides the native restart/quit outcome.

## Settings contract

Blue/System/100%/Comfortable are the intended defaults. Persistent settings require a bounded owner, correlated revisions, one writer and latest-value coalescing. Under [DESK-004](../../product/desktop-and-accessibility.md), a failed write restores the last confirmed active/displayed values, explains the failure and offers explicit Retry. A bounded retry draft is non-authoritative; a stale failure must not replace a newer confirmed revision. No view performs file I/O. These invariants apply to every preference, including platform effects with their own failure states.

## Input, privacy and native qualification

The input module owns native clipboard admission for editable fields and explicit
read-only Copy actions. A desktop retains one bounded read-only selection, not
one entity per message. Live rendered text supplies its current content and
privacy decision; unmounting, replacement and restriction revoke its action
route. A message uses the room's existing native clipboard privacy projection,
independently of send permission. Read-only text has no IME handler or caret.

Keep server/search/creation input and draft text distinct from backend snapshots. Exporting drafts through Copy or Cut requires an explicit privacy-policy decision; generic send permission is insufficient. Paste imports text into an editable local draft, including offline, and uses document-size and composition admission independently of export authority. GPUI 0.2.2 materializes the platform clipboard String before a widget can check its size, so that transient allocation remains an explicit resource limitation.

Bounded multiline plain-text editing, IME integration, focus restoration and modal isolation each need appropriate checks. Actual language/IME sessions, localization, bidi handling, macOS permissions, native accessibility roles/announcements and VoiceOver remain unqualified. Direct incoming acceptance skips voluntary conference prejoin while preserving mandatory permission, consent and security gates.

Functional icons use pinned Lucide SVGs, with the filled-star variation identified in [icon provenance](../../../apps/desktop/assets/icons/PROVENANCE.md); the application mark remains separate project artwork. Runtime typography uses bundled Google Sans with its OFL notice and trademark guidance. Historical Manrope design assets retain their own attribution. A native screenshot must identify its actual source/platform and fictional data; a browser mockup or static contrast calculation does not establish integrated rendering or accessibility.


Settings/sidebar composition uses one platform AccountConnection for ordinary launches and an explicitly labelled fictional port for development scenarios. Presentation imports bounded UI/preferences values only. Account preferences, archive, favorites and tags are correlated through the admitted account owner; old-scope results cannot populate a replacement account. A send allocates one operation ID and retains it through uncertain status checks. Native permission and device runtimes remain separate owners: permission checks never capture, and preview replacement waits for disposal acknowledgement.
