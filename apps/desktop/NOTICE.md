# Native desktop notices

Original project code is EUPL-1.2. See the repository `LICENSE` and `NOTICE.md`.

The macOS screen permission check uses the safe preflight API of
[`core-graphics` 0.24.0](https://docs.rs/crate/core-graphics/0.24.0/source/src/access.rs),
by The Servo Project Developers, under MIT OR Apache-2.0. Its framework FFI uses
internal unsafe code; application Rust remains subject to `unsafe_code = "forbid"`.
The existing dependency license collection applies to this direct dependency too.

`src/presentation/input.rs` is adapted from Zed Industries' GPUI 0.2.2
`examples/input.rs`, distributed under Apache-2.0. Its original license is
[assets/GPUI-LICENSE-APACHE](assets/GPUI-LICENSE-APACHE). Changes implement finite
UTF-8 admission, privacy-gated user-entry clipboard, native focus/appearance,
horizontal caret scrolling, corrected IME replacement offsets and grapheme tests.
Preserve this attribution and license when redistributing the derived source.

Manrope is Copyright 2018 The Manrope Project Authors
(https://github.com/googlefonts/manrope), under SIL OFL-1.1. The unchanged original
and notice are in [assets/fonts](assets/fonts). Regular (400), SemiBold (600) and
Bold (700) are static instances generated from that original; their hashes and
generation method are recorded in `static-manrope-provenance.json` and
`tools/ui/instantiate-manrope.py`. The OFL has no reserved font name in this copy.
The original font remains unchanged. The three instances are embedded in the
executable with `include_bytes!`; no development-machine font path is required.

Distributors must include the project license, this notice, the Manrope notice,
and applicable dependency notices. The dependency evidence in
`docs/design/native-ui` is a provenance record, not release/license approval.
Synthetic people and organization data are available only through an explicit
`--ui-scenario` development option and labeled in every application view.

Functional SVG icons are unmodified Lucide 0.468.0, pinned to commit
`f12b0de177fbc2a6795e99be065887e72b237123`. Preserve the unchanged
[Lucide notice](assets/icons/LUCIDE-LICENSE), which includes the ISC license and
Cole Bemis / Feather MIT attribution. The three brand SVGs remain original
project artwork under EUPL-1.2. [Icon provenance](assets/icons/PROVENANCE.md) and
`assets/icons/lucide-manifest.json` identify exact upstream paths and hashes.
SVG bytes are embedded; no runtime asset directory is needed.

Application sounds have separate upstream licenses. See [sound attribution](assets/sounds/README.md) and the included license texts; retain them in binary distributions.
