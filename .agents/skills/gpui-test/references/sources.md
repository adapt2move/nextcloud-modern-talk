# Sources and version checks

This is original project guidance informed by Zed's [gpui-test skill](https://github.com/zed-industries/zed/blob/56934ec0560bce8929025497411de7f5322fb6ca/.agents/skills/gpui-test/SKILL.md), pinned at `56934ec0560bce8929025497411de7f5322fb6ca`. Upstream workflow ideas were checked against the published dependency instead of copying its text or examples. Zed's root [licensing statement](https://github.com/zed-industries/zed/blob/56934ec0560bce8929025497411de7f5322fb6ca/README.md#licensing) defaults to GPL-3.0-or-later with marked Apache components; no separate license marker was found for the skill. The [shared provenance note](../../gpui-guidelines/references/sources.md) explains the original-authoring boundary. No upstream license is relabeled EUPL.

## Published sources

- [gpui 0.2.2 archive](https://static.crates.io/crates/gpui/gpui-0.2.2.crate), SHA-256 `979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`.
- [gpui-macros 0.2.2 archive](https://static.crates.io/crates/gpui-macros/gpui-macros-0.2.2.crate), SHA-256 `bcb02dd63a2859714ac7b6b476937617c3c744157af1b49f7c904023a79039be`.

Both checksums match [Cargo.lock](../../../../Cargo.lock). Both crate manifests declare Apache-2.0. Their VCS metadata names `69e2130295c2649963eb639fc70b4f2ee8ea1624` with `dirty: true`; the published archives, not that Git revision or Zed main, are authoritative here.

| Retained claim | Exact source |
| --- | --- |
| Attributes, argument types, generated Rust test and direct `rand` reference | [gpui-macros `src/test.rs`](https://docs.rs/crate/gpui-macros/0.2.2/source/src/test.rs), argument parser and `test` expansion |
| SEED/ITERATIONS, duplicate first seed, retry behavior | [gpui `src/test.rs`](https://docs.rs/crate/gpui/0.2.2/source/src/test.rs), `run_test` and `calculate_seeds` |
| Test-support feature closure and gated exports | [Cargo.toml](https://docs.rs/crate/gpui/0.2.2/source/Cargo.toml), [src/gpui.rs](https://docs.rs/crate/gpui/0.2.2/source/src/gpui.rs), [src/app.rs](https://docs.rs/crate/gpui/0.2.2/source/src/app.rs) |
| Executor field and context pumping | [src/app/test_context.rs](https://docs.rs/crate/gpui/0.2.2/source/src/app/test_context.rs), `TestAppContext` and `run_until_parked` |
| Timers, waiting hints/backtrace, clock advancement | [src/executor.rs](https://docs.rs/crate/gpui/0.2.2/source/src/executor.rs), `BackgroundExecutor`; [test dispatcher](https://docs.rs/crate/gpui/0.2.2/source/src/platform/test/dispatcher.rs), `advance_clock` and `run_until_parked` |
| Package/target, locked selection and test filters | [desktop manifest](../../../../apps/desktop/Cargo.toml), [Cargo test manual](https://doc.rust-lang.org/cargo/commands/cargo-test.html), [libtest options](https://doc.rust-lang.org/rustc/tests/index.html) |

The unsupported diagnostic-variable finding comes from searching both complete published source trees. `ZED_HEADLESS` does exist in the Linux/FreeBSD compositor selector, but selects a headless backend, not native window validation; it is intentionally absent from test recipes. This is a static compatibility check: no project test, snippet or feature configuration was compiled or executed.
