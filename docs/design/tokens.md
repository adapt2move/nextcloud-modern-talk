# Design tokens

**Target specification.** Palette values below describe the preferred blue direction and the sage comparison. Semantic roles and accessibility requirements take precedence over literal colors and small text in retained HTML/PNG sketches. Units are logical points for native UI; CSS pixels are only the mockup measurement reference.

## Palette roles

| Token | Blue, preferred | Sage, comparison | Use |
| --- | --- | --- | --- |
| `text.primary` | `#26354B` | `#252E2E` | Main text and headings |
| `text.secondary` | `#596B85` | `#566358` | Readable secondary text on light surfaces |
| `surface.content` | `#FFFFFF` | `#FFFFFF` | Conversation and form content |
| `surface.subtle` | `#F6F8FC` | `#F6F7F4` | Grouping and inset areas |
| `action.primary` | `#356FC1` | `#354F41` | Primary button, selection indicator; white button text |
| `selection.surface` | `#EAF1FB` | `#EDF2E9` | Selected content with a distinct outline/label |
| `chrome.start` | `#E3EDFB` | `#E9E7BF` | Schematic window/sidebar tint |
| `chrome.end` | `#CBDCED` | `#C6D6DF` | Schematic gradient end |
| `border.decorative` | `#E6ECF4` | `#E9ECE8` | Nonessential separators only |
| `stage.background` | `#1C293B` | `#202725` | Call stage |
| `stage.text` | `#EAF0F8` | `#E9EEEA` | Text on the dark stage |

Gradients are visual exploration. Native title bars and increased-contrast appearances follow the platform. Do not depend on a decorative separator to identify an input or a selected control.

| Semantic role | Foreground / light background | Required accompanying cue |
| --- | --- | --- |
| Information / selected label | `#315E98` / `#EAF1FB` | Action label, selected state, or information icon |
| Available / success | `#286343` / `#EDF7F0` | “Verfügbar” or a success message |
| Waiting / warning | `#795300` / `#FFF4D6` | “Verbindung wird wiederhergestellt” or the specific warning |
| Error / destructive action | `#9E3E32` / `#FFF0EC` | Error explanation, “Ablehnen”, “Auflegen”, or “Löschen” |
| Recording | `#9E3E32` / `#FFF0EC` | Recording icon plus “Aufzeichnung läuft”; consent is separate |

These roles are shared across palettes. Gray indicates unavailable/unknown state with a reason. An unread count combines a number and emphasis; speaker activity combines an outline with a speaking indicator. Do not recolor every status blue or use avatar color as a status.

## Typography and alignment

`D-T01`: Manrope is the design typeface. The bundled variable font supports weights 200–800; use 400 for body, 600–650 for controls, and 700 for headings. Use the platform font for system-owned UI and as a fallback. Validate fallback glyphs, emoji and international scripts without clipping. See the [font notice](../../NOTICE.md#manrope).

| Role / metric | Native target or reference |
| --- | --- |
| Body / message | 14 pt / 22 pt line height, weight 400 |
| Conversation-list title | 13 pt / 18 pt, weight 650 |
| Secondary label / timestamp | 12 pt / 18 pt; do not copy the sketches' 8–10 px labels as a shipping default |
| Screen heading | 20–28 pt / 1.25 line height, weight 700 |
| Spacing scale | 4, 8, 12, 16, 24, 32; related controls share an 8 or 12 pt gap |
| Icon box | 20 × 20 pt; consistent optical centering and stroke weight |
| Standard control | 36 pt high; at least 32 × 32 pt interactive hit area |
| Primary form action | 44–46 pt high; label and icon centered on the same control axis |
| Common radii | 8 pt controls, 12 pt content panels; native window corners are system-owned |
| Baseline alignment | Align sender and time on one baseline; body starts at the same x-coordinate; keep avatars in a fixed column |
| Screenshot reference | 1280 × 900 canvas; 1120 × 760 schematic window; 44 px title bar; 278 px sidebar |
| Call reference | 1096 × 704 inner sheet; 274 px chat panel; 64 px header; 90 px control area |

`D-T02`: Resize around content. A narrow window collapses secondary panels before truncating primary actions; a long name can wrap in details. Keep chat and call controls reachable at larger text sizes. The fixed-size mockup is not evidence of a responsive native layout.

## Accessibility targets

| ID | Acceptance target |
| --- | --- |
| **D-A01** | At least 4.5:1 text contrast and 3:1 for essential control/state graphics. Measure actual composited backgrounds, including selected and focus states. |
| **D-A02** | Full keyboard operation, visible unobscured focus, sensible reading/tab order, no keyboard trap, and focus restoration after dialogs. Receiving a call must not accept it through an unrelated keystroke. |
| **D-A03** | Expose names, roles, values, toggle states and selected conversation to VoiceOver. Announce meaningful connection/error changes without reading every timer or audio-meter update. |
| **D-A04** | Text can scale to 200% without loss of primary actions or message content. Honor the macOS contrast/transparency settings and retain status labels in monochrome. |
| **D-A05** | Use short, finite ease-out transitions: 110 ms for hover and switch feedback, 140 ms reveals for dialogs, settings pages, transient feedback and newly appended messages. Closing is immediate. Message entry preserves row geometry and the reader's anchor; only a reader already at the end follows new content after a brief delay. Respect Reduce Motion, including when the system preference is unknown. No flashing ringing effect. Audio cues have visible equivalents. |
| **D-A06** | Controls have at least 32 × 32 pt hit areas; use 44 pt for principal call actions where space permits. Tooltips supplement persistent labels; they are not the only accessible names. |

The contrast, keyboard, focus and scaling targets use [WCAG 2.2](https://www.w3.org/TR/WCAG22/) as measurable design criteria alongside macOS accessibility conventions. This is not a WCAG conformance claim for a native application. Hit-area sizes above are project targets.

**Known design debt:** original screenshots contain small metadata, some muted labels below target contrast, and decorative mock controls. Native focus, dark appearance, text scaling and VoiceOver need implementation and testing. The blue prototype's historical `--muted: #6D7C91` is not the approved small-text token.
