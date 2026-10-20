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

Native SVG geometry is inherited from the repository design prototype
`docs/design/prototypes/blue/ui-sketch.html` under the repository EUPL-1.2
scope. No separate upstream icon authorship is asserted. Simple supplemental
geometries were authored for D1; D1.1 restores the project prototype brand as
three palette-colored SVG layers and simplifies the sun badge. See `assets/icons/PROVENANCE.md` for the exact
source boundary. SVG bytes are embedded; no runtime asset directory is needed.

Application sounds have separate upstream licenses. See [sound attribution](assets/sounds/README.md) and the included license texts; retain them in binary distributions.
