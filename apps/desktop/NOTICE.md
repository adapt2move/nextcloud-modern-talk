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
horizontal caret scrolling, UTF-16 replacement/selection, composition undo,
grapheme masking, wrapped-line caret affinity and bounded editing tests.
Preserve this attribution and license when redistributing the derived source.

The runtime embeds unmodified Google Sans v14.000 static Regular (400), Medium
(500) and Bold (700) fonts from the Google Sans Project, under SIL OFL-1.1.
Preserve [the OFL notice](assets/fonts/GoogleSans-LICENSE.txt),
[trademark guidance](assets/fonts/GoogleSans-TRADEMARKS.txt) and
[exact release provenance](assets/fonts/googlesans-provenance.json). These fonts
are not relicensed under EUPL. No development-machine font installation is needed.
Historical design mockups retain their separately attributed Manrope fonts.

Onboarding step numerals use the unmodified Google Sans Code v7.001 Roman
variable font at Regular (400), with its fixed-width `MONO` axis at the default
of 1. This font is separately licensed under SIL OFL-1.1; preserve
[its full license](assets/fonts/GoogleSansCode-LICENSE.txt),
[trademark guidance](assets/fonts/GoogleSansCode-TRADEMARKS.txt) and
[exact release provenance](assets/fonts/googlesanscode-provenance.json).
The body font remains Google Sans.

The patched GPUI 0.2.2 renderer remains Apache-2.0. Its original license,
published archive identity and modified-file inventory are retained in
[the vendored source](../../vendor/gpui/UPSTREAM.json). Distribution notices
include its upstream license and patch provenance, separately from project code.

Distributors must include the project license, this notice, the font notices,
and applicable dependency notices.
Synthetic people and organization data are available only through an explicit
`--ui-scenario` development option and labeled in every application view.

Functional SVG icons use Lucide 0.468.0, pinned to commit
`f12b0de177fbc2a6795e99be065887e72b237123`. Their bytes are unchanged except for
the filled favorite-star variant: its fill is `currentColor` and its upstream
geometry is unchanged. Preserve the unchanged
[Lucide notice](assets/icons/LUCIDE-LICENSE), which includes the ISC license and
Cole Bemis / Feather MIT attribution. The three brand SVGs remain original
project artwork under EUPL-1.2. [Icon provenance](assets/icons/PROVENANCE.md) and
`assets/icons/lucide-manifest.json` identify exact upstream paths and hashes.
SVG bytes are embedded; no runtime asset directory is needed.

Application sounds have separate upstream licenses. See [sound attribution](assets/sounds/README.md) and the included license texts; retain them in binary distributions.
