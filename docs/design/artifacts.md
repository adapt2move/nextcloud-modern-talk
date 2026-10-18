# Artifact provenance and review

Current package: **revision 06, 2026-10-18**. This record covers schematic design artifacts only, not native rendering, server compatibility, accessibility conformance or performance. The [round 1 remediation report](../reviews/design-remediation-r1.md) records author checks; independent re-review remains pending.

## Current inventory

| Path | Content / status |
| --- | --- |
| `assets/blue/`, `assets/sage/` | 20 current screen PNGs per palette, D-S01–D-S20 |
| `assets/<palette>/contexts/` | Two additional D-S09 captures: Teamküche and Designbesprechung; not separate screen IDs |
| `assets/superseded/<palette>/13-incoming-v04.png` | Historical incoming-call sketch; preview-first acceptance is obsolete |
| `assets/superseded/<palette>/*-v05.png` | Four pre-remediation views per palette: creation 14/15, discovery 17 and active voice 18 |
| `assets/fonts/` | Unmodified Manrope variable font and full OFL notice |
| `prototypes/blue/ui-sketch.html`, `prototypes/sage/ui-sketch.html` | Self-contained editable mockups with identical navigation scripts |
| `prototypes/index.html`, `prototypes/sage/index.html` | Current galleries; four account, nine conversation and seven call views, with counts derived from data |
| `prototypes/blue/palette-blue.css` | Retained palette source; colors also embedded in the blue mockup |
| `prototypes/talk-features.html` | Clearly labeled historical feature comparison; its original 18-view count is historical |
| `prototypes/talk-model-and-compatibility.html` | Historical source comparison; its former voice-room entry description is explicitly superseded |

There are **40 current screen PNGs, four contextual PNGs and ten historical PNGs: 54 total**. All 36 inherited PNGs remain byte-identical in either current or superseded locations. Revision 05 added four captures; revision 06 adds fourteen captures (eight replacements, two new conversation views and four contextual checks). Replaced images remain in `superseded/` and are not selectable in current galleries. ZIP bundles, raw upstream research snapshots and manifests containing private source paths are excluded.

The inherited material came from the project's original 18-view blue/sage design set. Repository HTML copies were adapted for relative links, schematic status and corrected flows. The supplied `design/` directory outside the repository remains untouched.

## Revision 06 — current

- **D-S20 `voice-room`:** Teamküche conversation with chat, participants and explicit call-join action. Sidebar navigation does not depict participation or media capture.
- **D-S09:** Teamküche joins, including the discovery shortcut, pass through the correctly named prejoin. The voice example keeps microphone/camera off before confirmation and states microphone-on / camera-off intent at final join, subject to required gates. Back/Escape returns to the selected conversation; Designbesprechung and Produktteam also preserve their contexts.
- **D-S18:** Only final voluntary join reaches the active Teamküche voice example. **Teilen*** stays in the same two-person call and explicitly marks the unsketched sharing flow. Leaving returns to Teamküche.
- **D-S14/15/17:** Preset/discovery wording now distinguishes opening a conversation from deliberate call entry. Both palettes have updated captures.
- **Galleries:** D-S20 is added without renumbering D-S01–D-S19. Visible counts come from the data; obsolete current revision labels are removed. D-S13 → D-S19 remains the incoming direct-call route without prejoin.

<a id="revision-05"></a>

## Revision 05 — historical

Revision 05 had 19 views per palette. It changed incoming direct-call **Annehmen** to D-S19, with exactly Mara and Alex and no conference device check. The representative local intent is microphone on after authorization/required gates, camera off. Four-person conferences remained separate. Those incoming/direct-call PNGs are retained unchanged in revision 06. The [independent round 1 review](../reviews/design-spec-review.md) accepted that artifact-level distinction and requested the subsequent voice/context/gallery corrections.

## Verification and limits

All fourteen revision 06 assets were saved with T3 `preview_snapshot` at **1280 × 900**, showing the full **1120 × 760** schematic window. These are browser-rendered HTML images. Both mockups loaded Manrope; all 20 states per palette rendered without horizontal page overflow at the reference size. Representative saved images were visually inspected; this is not a pixel-by-pixel audit of every inherited image.

Actual browser clicks exercised sidebar/discovery → Teamküche prejoin → cancel/join, illustrative voice sharing, Designbesprechung/Produktteam prejoin back, conference sharing cancel/confirm/stop, and direct **Annehmen** / illustrative sharing. The Teamküche call retained its two people; direct acceptance retained Mara/Alex and skipped prejoin. See the remediation report for the route evidence and exact scope.

Both gallery scripts displayed 20 total / 4 account / 9 conversation / 7 call views. D-S19 and D-S20 remained selectable; switching palettes in the combined gallery preserved selection and changed the image reference. The source HTML was loaded in memory into one owned T3 tab, without a server. Relative image references were checked on disk, not fetched by a served gallery page. Query-string initialization was source-checked; contextual rendering and real click routes were exercised independently of it. None of these checks demonstrates native navigation, capture, permissions or Talk integration.

Previous token calculations remain limited to specified pairs: white-on-blue-primary **5.01:1**, white-on-sage-primary **8.95:1**, and proposed secondary-text pairs above **5:1**. The inherited blue muted `#6D7C91` is **4.25:1** on white; [tokens.md](tokens.md) supplies improved targets for small text. Revision 06 does not assert an accessibility audit.

The bundled font, OFL notice and project EUPL text retain the hashes documented in [NOTICE.md](../../NOTICE.md). All four embedded font payloads match the bundled TTF; both editable mockups retain the complete font notice. No new third-party assets were introduced. The pinned primary distributions and their earlier verification are documented in NOTICE and the independent review.

No VM, service, build, native test, Nextcloud deployment, Git operation or publication occurred during this remediation. Device settings, preview, authentication, consent, capture and media remain illustrations. Product requirements still govern all OS permission, secret-storage, policy, consent, security and real interoperability work after the architecture/module/interface bootstrap.

## Repeat the focused review

1. Open either editable mockup locally with `?screen=chat` at 1280 × 900 and wait for Manrope.
2. Click Teamküche in the sidebar. Confirm D-S20, its composer, and no joined/capturing state. Choose **Anruf beitreten**, check Teamküche's D-S09, cancel, then repeat and confirm **Jetzt teilnehmen** → D-S18. **Teilen*** must keep that call and two people.
3. Use **Offene Gespräche entdecken** → **Anruf beitreten**. Confirm the same Teamküche prejoin; back opens D-S20 without joining.
4. Open Designbesprechung, choose **Teilnehmen**, then back. Confirm D-S16 and its composer. Repeat from Produktteam and confirm D-S05. Check conference picker cancel, confirm and stop without changing conversation.
5. Open `?screen=incoming`, choose **Annehmen**, and confirm Mara/Alex in D-S19 without D-S09. The direct sharing control remains illustrative.
6. Repeat in the other palette. Inspect both galleries' derived totals and select D-S19/D-S20; switch palettes in the combined gallery. Use [flows.md](flows.md) for contextual screenshots and remaining coverage limits.
