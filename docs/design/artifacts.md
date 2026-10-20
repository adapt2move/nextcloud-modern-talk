# Design artifact provenance

Revision 06 is an illustrative design reference. Browser-rendered mockups and PNGs do not establish native rendering, server compatibility, accessibility, performance or working authentication/media.

## Inventory

| Path | Content |
| --- | --- |
| `assets/blue/`, `assets/sage/` | D-S01–D-S20 schematic screen PNGs per palette |
| `assets/<palette>/contexts/` | Teamküche and Designbesprechung contextual prejoin examples |
| `assets/fonts/` | Unmodified Manrope variable font and full OFL notice |
| `prototypes/blue/ui-sketch.html`, `prototypes/sage/ui-sketch.html` | Editable illustrative mockups |
| `prototypes/index.html`, `prototypes/sage/index.html` | Galleries for the current screen set |
| `prototypes/blue/palette-blue.css` | Palette source |
| `prototypes/talk-features.html`, `prototypes/talk-model-and-compatibility.html` | Original comparison material; product specifications govern current scope |

The assets derive from the project's original blue/sage design set. Repository HTML uses relative paths and schematic labels. Preserve embedded font payloads, full OFL notices and the provenance in [NOTICE.md](../../NOTICE.md). Supporting assets are not implementation or release evidence.

## Flow interpretation

Opening Teamküche shows its conversation, without joining a call or activating capture. Conference entry uses the named prejoin and returns to the selected conversation on cancellation; direct and huddle audio entry follows the explicit header action. Direct incoming acceptance routes to the direct-call example without voluntary conference prejoin. Mandatory OS, consent, policy and security gates still apply in the product.

Use [flows](flows.md) and [tokens](tokens.md) for the intended design. German text in inherited UI illustrations is localized example copy; public documentation is English. Native behavior and product requirements remain authoritative over any obsolete sketch.
