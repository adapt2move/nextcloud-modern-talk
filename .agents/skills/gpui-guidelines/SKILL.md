---
name: gpui-guidelines
description: Implement or review GPUI entity and task ownership, cancellation, rendering, bounded lists, stable identity, subscriptions, focus and text interaction. Apply to GPUI presentation work using the project's published dependency, not unintegrated component libraries.
source: https://github.com/AprilNEA/gpui-skills/tree/3961d8cb27c345c04b592e79794eff0d278865ed/gpui
---

# GPUI presentation engineering

Use [AGENTS.md](../../../AGENTS.md) for project boundaries and execution constraints. The checks below target published GPUI **0.2.2** with Rust **1.99.1**; [source and adaptation notes](references/sources.md) identify verified APIs and upstream omissions. A separate toolkit experiment supplies no implicit dependency or capability to this application.

## Entity, task and subscription lifetimes

Keep one deliberate owner for each view, operation and subscription. Strong `Entity` handles retain state; use `Entity::downgrade`/`WeakEntity` for back-references and work that must not keep a closed view alive. A weak upgrade only proves existence: verify account/room/request generation again before applying asynchronous success **or failure**. Stale errors must not replace newer success.

`Context::spawn` supplies a `WeakEntity<T>` and `&mut AsyncApp`; `spawn_in` supplies an async window context. Use short update closures and release entity borrows before awaiting. Re-enter through the weak handle and handle its failed update as a possible closed entity. Do not update an entity recursively while its update is active. `cx.spawn` runs on the foreground; it does not move heavy CPU work off the UI thread. Send eligible bounded computation to `cx.background_executor().spawn(...)`; I/O and secrets remain behind the project's owning interfaces.

Retain tasks whose lifetime belongs to an operation or view, and await outcomes that affect state. Dropping a task cancels its future, but does not prove an already-started external effect, blocking computation or cleanup has settled. Couple cancellation to the owner's protocol and reject late completions. For debounce, replace the owned timer/task and check the request generation at completion. Detach only when a separately owned, finite lifecycle and error route justify it; detaching is not a general fix for dropped-task warnings.

Store `Subscription` handles for as long as observation is needed. Dropping them unregisters callbacks; deliberate teardown/replacement is useful. Avoid adding a fresh subscription every render. `observe` reacts to entity notification; `subscribe` handles typed events from an `EventEmitter`. Keep snapshot updates and semantic transitions distinct, with meaningful event payloads and no generic event bus. Do not promote account/room state into a global solely to avoid passing its owner.

## Rendering and loaded collections

Keep rendering a bounded projection of prepared state: no I/O, unbounded parsing/sorting, expensive repeated allocation or request spawning. Apply a coherent change, then notify the entity whose observable state changed. `cx.notify()` invalidates dependents and observers; an assignment that changes no observable state needs no notification. Do not notify continuously from rendering or set up feedback loops between observers.

Virtualize visible rows **and** bound the loaded data, retained entities, decoded images and queued work. `uniform_list` assumes equal row heights and needs a bounded viewport; it is unsuitable for arbitrary multiline message heights. Variable-height `list`/`ListState` caches offscreen heights: use its `splice`/`reset` contract when content heights change. Virtualization alone does not cap storage or all list metadata. Preserve the user's scroll anchor and selection through paging and eviction.

Key interactive rows by stable domain identity within the proper account/conversation namespace. Index-based IDs can attach focus, selection or element state to the wrong message after insertion/reordering. `Window::use_keyed_state` persists state only while the keyed element is rendered in consecutive frames; keep drafts, durable selection and async ownership above virtualized rows. Reuse focus/list handles across renders.

## Focus, modal interaction and text

Connect a retained `FocusHandle` through `track_focus`; route actions through the intended `key_context` and action handlers. Verify actual Tab/Shift-Tab navigation and disabled controls. In 0.2.2, `Focusable::focus_handle` takes `&App`, and `FocusHandle::focus` takes `&mut Window` without an `App` argument. Do not copy incompatible upstream signatures.

Give dialogs a defined focus entry, containment, dismissal action and return target. Check Escape and shortcuts against [desktop interaction requirements](../../../docs/product/desktop-and-accessibility.md), including retained drafts/calls and IME composition. Use the text-input contract (`EntityInputHandler`, UTF-16 ranges, selection and marked text), not only key-down callbacks. Keep user content separate from translated UI labels and preserve en/de/es requirements.

Visible focus is only visual feedback. Accessible names, roles, values, actions and announcements require a semantic platform integration and independent evidence. Screenshots, keyboard navigation or an isolated AccessKit experiment do not establish that integration. Report missing semantics honestly.

`cx.theme()`, `h_flex`/`v_flex`, `WindowExt::open_dialog`, `focus_ring`, `Disableable` and `Sizable` from upstream examples are not a promised GPUI 0.2.2 component API. Locate an actual project/component definition before using a helper; prefer existing project tokens and components rather than adding a library implicitly.

For a meaningful lifecycle/state regression, use `gpui-test`; for a measured responsiveness question, use `gpui-bench`. Neither substitutes for native semantic or platform verification.
