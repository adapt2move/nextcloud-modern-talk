# Notices

## Project license

Original project code, documentation and original design artifacts are licensed under the **European Union Public Licence v. 1.2**, SPDX identifier `EUPL-1.2`, subject to its terms. Copyright remains with the respective project contributors. Separately licensed third-party material retains its own license and notices.

[LICENSE](LICENSE) is the unchanged English license text from the authoritative [SPDX license-list-data source](https://github.com/spdx/license-list-data/blob/b8d6af45ad2fcfed61bb85a8ad068aa4a77eadf9/text/EUPL-1.2.txt), retrieved on 2026-10-18. See also the [SPDX EUPL-1.2 entry](https://spdx.org/licenses/EUPL-1.2.html).

SHA-256 of the included license text: `57fb42fbcd0b037ce528ed8f72f1ec095d67bc6825ecf1448ff39be1fe68a4b4`.

## Google Sans and GPUI

The runtime embeds unmodified Google Sans v14.000 Regular, Medium and Bold from
the [Google Sans Project](https://github.com/googlefonts/googlesans/tree/5c8f512c5fb42a7be099b951a7c097f96209c78e),
under SIL Open Font License 1.1. Preserve the
[font license](apps/desktop/assets/fonts/GoogleSans-LICENSE.txt),
[trademark guidance](apps/desktop/assets/fonts/GoogleSans-TRADEMARKS.txt) and
[release provenance](apps/desktop/assets/fonts/googlesans-provenance.json).
The fonts retain their separate license.

Onboarding step numerals use Google Sans Code v7.001 Regular (400), from the
unmodified Roman variable font in the [official release](https://github.com/googlefonts/googlesans-code/releases/tag/v7.001),
pinned to commit `f9e36cea20d34ecc9b61c09609fe02e1438364fc`. The fixed-width `MONO`
axis remains at its upstream default of 1. Preserve its separate
[SIL OFL-1.1 license](apps/desktop/assets/fonts/GoogleSansCode-LICENSE.txt),
[trademark guidance](apps/desktop/assets/fonts/GoogleSansCode-TRADEMARKS.txt) and
[font provenance](apps/desktop/assets/fonts/googlesanscode-provenance.json).

The patched GPUI 0.2.2 dependency remains Apache-2.0. Local changes cover backdrop
rendering and the nullable macOS text-input range output pointer. Its
[upstream license](vendor/gpui/LICENSE-APACHE) and
[original source and patch inventory](vendor/gpui/UPSTREAM.json) accompany the
source. Binary notice collection keeps this attribution separate from EUPL
project code.

## Manrope

Historical design assets retain Manrope; the runtime now uses Google Sans.

The bundled [Manrope variable font](docs/design/assets/fonts/Manrope-Variable.ttf), is separately licensed under **SIL Open Font License 1.1**, SPDX identifier `OFL-1.1`.

Copyright 2018 The Manrope Project Authors (https://github.com/googlefonts/manrope).

The full, unchanged copyright and license notice is in [Manrope-LICENSE.txt](docs/design/assets/fonts/Manrope-LICENSE.txt). The font is not relicensed under EUPL. Preserve its notice when redistributing or extracting the font.

The font and its notice were verified byte for byte against [Google Fonts' Manrope distribution](https://github.com/google/fonts/tree/b31870aff700ab7a1d74fa0c6887d95beb9e0037/ofl/manrope) on 2026-10-18. The upstream filename is `Manrope[wght].ttf`; only the local filename was changed. Font SHA-256: `3ae11c49db0455a3cc33e37d380f20fdb8c7f8b41dc07625c177e3d87a9d6ae6`.

## Markdown parser

Native chat formatting uses [pulldown-cmark 0.13.5](https://github.com/pulldown-cmark/pulldown-cmark/releases/tag/v0.13.5)
under MIT, without its optional HTML renderer. URL values use the separately
licensed pinned `url` crate. Preserve their upstream licenses and copyright
notices in dependency notices; these libraries are not relicensed under EUPL.

## Linux screen portal bindings

Linux screen-sharing capability checks use [ashpd 0.13.13](https://docs.rs/crate/ashpd/0.13.13/source/) under the MIT license, with its existing zbus bindings. The bounded asynchronous wait uses [async-io 2.6.0](https://docs.rs/crate/async-io/2.6.0/source/) and [futures-lite 2.6.1](https://docs.rs/crate/futures-lite/2.6.1/source/), each under Apache-2.0 OR MIT. These dependency licenses and their copyright notices remain applicable; preserve the corresponding upstream license files in distribution notices.

## Lucide icons

The functional SVG icons are unmodified [Lucide 0.468.0](https://github.com/lucide-icons/lucide/tree/f12b0de177fbc2a6795e99be065887e72b237123), pinned to commit `f12b0de177fbc2a6795e99be065887e72b237123`. The unchanged [upstream notice](apps/desktop/assets/icons/LUCIDE-LICENSE) includes the ISC license and Cole Bemis / Feather MIT attribution. [The icon manifest](apps/desktop/assets/icons/lucide-manifest.json) records immutable upstream paths and SHA-256 values. The three application brand SVGs remain separate project artwork under EUPL-1.2.

## Design provenance and names

The interface derives from the project's original design work. Historical sketches remain in Git history; current guidance is in [the design reference](docs/design/README.md). People, messages and the example organization are fictional; geometric portrait placeholders do not depict real call footage.

Buzz was a visual inspiration only. No Buzz source code or assets are included in this design package. References to Nextcloud, Talk, GPUI, Apple or other projects identify interoperability targets or technical sources; they do not imply affiliation, endorsement or trademark rights.
