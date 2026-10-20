# Provenance and compatibility

These skills and the revised AGENTS.md are original project instructions, selecting engineering ideas and verified API facts from the sources below. No upstream prose, diagrams or code examples are vendored. The project's [EUPL-1.2 license](../../../../LICENSE) remains unchanged; this does not relicense the external sources.

## Guidance sources and license scope

- AprilNEA's [gpui skill and rule directory](https://github.com/AprilNEA/gpui-skills/tree/3961d8cb27c345c04b592e79794eff0d278865ed/gpui), commit `3961d8cb27c345c04b592e79794eff0d278865ed`. The [skill frontmatter](https://github.com/AprilNEA/gpui-skills/blob/3961d8cb27c345c04b592e79794eff0d278865ed/gpui/SKILL.md) and [README](https://github.com/AprilNEA/gpui-skills/blob/3961d8cb27c345c04b592e79794eff0d278865ed/README.md) declare MIT; the pinned root and gpui directory contain no license file with a notice/grant. The original rules stay outside the project. This adaptation uses independently checked facts and original wording, avoiding distribution of incompletely attributed upstream text.
- Zed Industries' [.rules](https://github.com/zed-industries/zed/blob/56934ec0560bce8929025497411de7f5322fb6ca/.rules), commit `56934ec0560bce8929025497411de7f5322fb6ca`, informed AGENTS.md's scoped engineering, local rationale, error handling and GPUI lifecycle guidance. Its [README](https://github.com/zed-industries/zed/blob/56934ec0560bce8929025497411de7f5322fb6ca/README.md#licensing) states a GPL-3.0-or-later default with marked Apache-2.0 components. No Apache exception was found for `.rules` or the two skill files in their headers/ancestor directories; GPUI's Apache crate license is not applied to those documents. They are attributed research sources, not redistributed files.

The Zed test and benchmark sources are attributed in the corresponding [test](../../gpui-test/references/sources.md) and [benchmark](../../gpui-bench/references/sources.md) notes. Future copying of third-party text/code needs its actual license and notices; these references are not a grant to copy it under the project license.

## What was deliberately trimmed or corrected

April's real rule files on entity ownership, weak references, task/debounce lifetimes, subscriptions, notification, focus and dialogs were inspected. Repeated Rust/builder/style examples and unavailable index entries were omitted, as were application/component-library helpers presented without their defining dependency. The retained guidance corrects unconditional notify/detach advice, distinguishes task cancellation from completed cleanup, and separates a focus ring from semantic accessibility. No copied rule hierarchy or dangling `rules/` links remain.

AGENTS.md takes no README review banner, hard-rule override, Zed-specific layout/script rule, Sentry workflow, release-note format or unconditional detach policy from Zed. Product contracts and the user's scope govern this project. Implementation reasons stay by code; needed historical evidence stays outside normal contributor documentation.

## Exact dependency sources

The [GPUI 0.2.2 archive](https://static.crates.io/crates/gpui/gpui-0.2.2.crate), SHA-256 `979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`, matches [Cargo.lock](../../../../Cargo.lock). Its manifest declares Apache-2.0. Archive VCS metadata says `69e2130295c2649963eb639fc70b4f2ee8ea1624`, `dirty: true`; the published bytes are the source of truth. [Workspace](../../../../Cargo.toml) and [toolchain](../../../../rust-toolchain.toml) establish the project pins. No experimental overlay was used.

| Guidance | Versioned source and symbols |
| --- | --- |
| Strong/weak lifetime and fallible re-entry | [app/entity_map.rs](https://docs.rs/crate/gpui/0.2.2/source/src/app/entity_map.rs): `Entity`, `WeakEntity`, `downgrade`, `upgrade`, `update` |
| Notify, typed subscriptions, async context signatures | [app/context.rs](https://docs.rs/crate/gpui/0.2.2/source/src/app/context.rs): `notify`, `observe`, `subscribe`, `spawn`, `spawn_in`; [app.rs](https://docs.rs/crate/gpui/0.2.2/source/src/app.rs): foreground/background accessors |
| Task ownership and cancellation limits | [executor.rs](https://docs.rs/crate/gpui/0.2.2/source/src/executor.rs): `Task` wrapping async-task; locked [async-task 4.7.1 task.rs](https://docs.rs/crate/async-task/4.7.1/source/src/task.rs): drop versus awaited cancellation (the latter is not exposed by GPUI's wrapper) |
| Subscription drop/detach | [subscription.rs](https://docs.rs/crate/gpui/0.2.2/source/src/subscription.rs): `Subscription` |
| Virtualized ranges and height invalidation | [elements/uniform_list.rs](https://docs.rs/crate/gpui/0.2.2/source/src/elements/uniform_list.rs); [elements/list.rs](https://docs.rs/crate/gpui/0.2.2/source/src/elements/list.rs): `ListState::splice`, `reset` |
| Focus signatures, element identity and keyed-state lifetime | [window.rs](https://docs.rs/crate/gpui/0.2.2/source/src/window.rs): `Focusable`, `FocusHandle`, `use_keyed_state`; [elements/div.rs](https://docs.rs/crate/gpui/0.2.2/source/src/elements/div.rs): `id`, `track_focus`, `key_context`, `on_action` |
| Native text-input seam | [input.rs](https://docs.rs/crate/gpui/0.2.2/source/src/input.rs): `EntityInputHandler`, selection, marked text and UTF-16 boundaries |

API and metadata checks were static. No example was compiled and no application, UI, accessibility or performance result follows from them.
