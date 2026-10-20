# Embedded native icons

All functional SVG icons come verbatim from [Lucide 0.468.0](https://github.com/lucide-icons/lucide/tree/f12b0de177fbc2a6795e99be065887e72b237123), pinned to commit `f12b0de177fbc2a6795e99be065887e72b237123`. They retain the official 24 × 24 geometry, two-unit stroke, rounded joins and `currentColor` source. The existing GPUI SVG renderer applies the native palette; no icon font, web runtime or additional Rust dependency is used.

[lucide-manifest.json](lucide-manifest.json) maps each embedded filename to the exact upstream path, immutable source URL and SHA-256. Aliases such as `chat` → `message-circle`, `more` → `ellipsis`, `phone-down` → `phone-off` and `check-circle` → `circle-check` preserve the application vocabulary; file bytes are unchanged. The manifest also hashes the verbatim [Lucide license](LUCIDE-LICENSE), including its Feather attribution. Lucide's ISC terms and the stated MIT attribution for Feather portions remain separate from the application's EUPL-1.2 license.

## Application artwork

`brand.svg`, `brand-front.svg` and `brand-knockout.svg` remain inherited project artwork under EUPL-1.2, separate from Lucide. They transcribe the prototype `.brandmark:before/:after` geometry in [the Blue sketch](../../../../docs/design/prototypes/blue/ui-sketch.html) and its Sage counterpart. The rounded outline and rotated foreground square are separate SVG masks so each uses its native palette role. `brand.svg` subtracts the foreground and its clearance through an alpha mask so its outline works on every background. No Lucide authorship is asserted for these files.

All SVGs are embedded with `include_bytes!`. The fixed in-memory `AssetSource` performs no runtime filesystem or network lookup.
