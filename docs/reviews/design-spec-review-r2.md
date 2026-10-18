# Independent design specification review — round 2

**Verdict: DR-001–DR-004 closed at design-artifact level.** Revision 06 resolves the four findings from the [independent first review](design-spec-review.md). The corrected material is consistent enough to serve as supporting input to the later Deep Modules architecture/module/interface bootstrap. No introduced material regression was found in the reviewed routes.

Reviewed **2026-10-18**, after reading the first review and [author remediation report](design-remediation-r1.md) in full. This is a focused independent closure decision, not approval of the concurrent product/quality work or an implemented application. Only this new report was written.

## Per-finding verdicts

Every interaction below was reproduced in **both blue and sage** using the original mockup handlers in one independently opened T3 browser tab.

| Finding | Verdict | Independently reproduced evidence |
| --- | --- | --- |
| **DR-001 — voice entry skips prejoin** | **Closed** | From D-S05, the **Teamküche** sidebar row opens **D-S20**, with Teamküche header, selected sidebar entry and composer, plus explicit not-in-call/devices-off copy. **Anruf beitreten** opens **D-S09 (Teamküche)**, showing **Bereit, wenn du es bist.**, devices still off, no microphone test, and the microphone-on/camera-off intent for final join. Back and Escape each restore D-S20. Only **Jetzt teilnehmen** reaches D-S18. Independently repeated the **D-S17 → Anruf beitreten → D-S09** shortcut, including back, Escape and final join. |
| **DR-002 — voice sharing becomes Produktteam** | **Closed** | In D-S18, clicking **Teilen***, exposed as **Teilen (nur Illustration)**, leaves Teamküche, Mara/Alex, count two and its composer unchanged. The visible footnote identifies the sharing flow as pending. No conference picker appears. Escape also leaves the active voice call intact; **Verlassen** returns to D-S20. This is the explicitly permitted minimal correction, not a newly implemented voice sharing sequence. |
| **DR-003 — prejoin back loses Designbesprechung** | **Closed** | **D-S16 → Teilnehmen → D-S09 → Zurück zum Gespräch** restores D-S16, Designbesprechung's composer and its selected sidebar row. Escape gives the same result. **D-S05 → Anruf starten → D-S09** similarly restores Produktteam/D-S05 on back or Escape. Teamküche cancellation restores D-S20. None of these cancellation paths displays an accepted call. |
| **DR-004 — stale counts/revisions** | **Closed** | Both current galleries display **20 total / 4 account / 9 conversation / 7 call** views, matching their datasets and actual thumbnail counts. D-S19 and D-S20 are selectable at **7 / 7** and **9 / 9** respectively. Current gallery titles/toolbars have no stale revision number. Markdown identifies revision 06 as current and revision 05 as historical; retained comparison HTML is explicitly historical. |

The discovery shortcut deliberately selects Teamküche as the conversation context, so cancelling opens its conversation rather than returning to the directory. That behavior agrees with [D-F04 and D-F06](../design/flows.md). The two people shown in Teamküche remain members of a group conversation with a voice-room preset; this does not turn it into a direct conversation.

## Regression and artifact evidence

- **Incoming direct acceptance remains separate:** in each palette, D-S13 **Annehmen** reaches **D-S19** with Mara Fischer/Alex Sommer, exactly two video tiles, count two, personal chat and **Nachricht an Mara**. No device-prejoin heading appears. The incoming card still states devices off before acceptance and microphone-on/camera-off intent on acceptance. Enter with the incoming card focused leaves it ringing. Direct **Teilen** stays in D-S19; Escape does not hang up. Its sharing sequence and the disclosed hang-up shortcut remain unchanged design limitations.
- **Ordinary conferences preserve context:** both Produktteam and Designbesprechung still pass through their correctly named D-S09 to the four-person D-S10. For each conversation in each palette, sharing-picker **Abbrechen**, Escape, confirmation and **Freigabe beenden** preserve its header, call-chat identity, composer and four-person context. Escape in the active conference leaves it active. Leaving returns to the corresponding conversation.
- **Gallery selection survives navigation:** selecting D-S19/D-S20 and switching blue → sage → blue preserves caption, selected thumbnail and category position while changing the image reference to the matching palette. Both galleries remember those selections when moving between account, conversation and call categories. Their datasets each contain 20 unique IDs; both editable mockups have the same navigation script.
- **Relative references resolve:** 132 local Markdown/HTML/CSS/gallery-data reference occurrences resolve on disk, including all 60 gallery image references. All five checked Markdown fragments resolve. This checks paths and fragments, not external websites or browser fetching from the original relative URLs.
- **Saved images match the corrected interpretation:** visually inspected eight exact saved PNGs: both palettes' `20-voice-room.png`, `18-voice.png`, `contexts/09-prejoin-teamkueche.png` and `contexts/09-prejoin-designbesprechung.png`. Their conversation identities, not-yet-joined Teamküche check, two-person active voice view and illustrative-sharing notice agree with the live mockups. These are representative checks, not a visual audit of every image.
- **Inventory/history agree:** 40 current screen PNGs, four contextual prejoin PNGs and ten historical PNGs, all 1280 × 900. The four contextual captures remain variants of D-S09, not extra gallery screens. Each of the eight newly historical `-v05` files matches the corresponding original current-image hash in the first review.

## License and font continuity

`LICENSE`, `NOTICE.md`, `Manrope-LICENSE.txt` and `Manrope-Variable.ttf` exactly match their first-review SHA-256 hashes. All four embedded font payloads still match the bundled TTF; both editable mockups retain the complete font notice, and Manrope loaded in both palettes.

The existing EUPL-1.2 and separate Manrope OFL-1.1 provenance assessment therefore has byte continuity. The pinned primary distributions remain recorded in [NOTICE](../../NOTICE.md) and the [first review](design-spec-review.md#license-provenance-and-public-file-checks). No broad legal research or new legal assurance is claimed in this round.

## Method, limits and handoff

The original self-contained HTML sources were transferred in memory into one new, owned T3 tab at **1280 × 900**. Existing `renderScreen` was used only to establish the initial conversation and incoming-call examples; the reviewed transitions used actual browser clicks and Escape/Enter input against the original handlers. No other agent's tab was operated. Saved PNGs were read directly from disk. The review tab was closed afterward.

The galleries' original relative images were not fetched from a served/local-file page in this in-memory setup. Paths were checked independently on disk; selection checks read the actual image references. Query-string context initialization was checked in source, not exercised by URL navigation. These method limits do not substitute for later packaged-application checks.

This acceptance concerns **mockup consistency only**. Native Rust/GPUI rendering, OS permissions, applicable recording consent/security gates, capture and preview lifecycle, secret storage/fallback, accessibility, backend interoperability and performance remain unimplemented/unverified here. The video-conference prejoin depicts an already deliberately authorized local preview; its full preview/test lifecycle remains an acknowledged target. [CALL-001/002/004/009/011](../product/calls.md) continue to govern subsequent work. None of those future gates is being reclassified as a defect in this authoring-stage review.

No outstanding blocker remains for **DR-001–DR-004**. Product/quality acceptance belongs to its separate reviewers; architecture/module/interface bootstrap must still precede full implementation. No VM, service, build, Nextcloud deployment, Git operation, publication or additional agent was started. A read-only dev-box status check reported an unprovisioned `new` session, **0 seconds VM time and EUR 0 estimated cost**; there was no VM to hibernate.

## Reviewed input snapshot

Initial hashes were captured at **2026-10-18T15:27:07.223764+00:00**. At **2026-10-18T15:33:04.214685+00:00**, all 72 design/license/prior-report inputs still matched. The concurrent `docs/product/calls.md` update was detected and the relevant CALL-001/002/004/009/011 sections were re-read; its hash below records that requirement context without approving the concurrent product changes.

The manifest contains all **31 changed/new design paths** relative to the first-review manifest (nine text/HTML files, fourteen current/contextual PNGs and eight historical copies), the two prior reports, the current call requirements, and four unchanged license/font continuity inputs: **38 files**. Subsequent changes are outside this verdict.

<details>
<summary>Reviewed input SHA-256 manifest</summary>

```text
57fb42fbcd0b037ce528ed8f72f1ec095d67bc6825ecf1448ff39be1fe68a4b4  LICENSE
32f132d980dc38fb62a767b30f33dd98f87fb60ce4d24e3e16455ace6e408683  NOTICE.md
4c2b9a18b8dd899d3da4f7fde554cf1a23183206612c5c30e862976f6ab60228  docs/design/README.md
0735a262627aa92f280cdae8efd4284bc88ede5e3d8ce00523fe5adef92934f2  docs/design/artifacts.md
b26737c09a53e29fa25e480f234fc5aecc2fdd7ea3dcec2558942250c35b8ccc  docs/design/assets/blue/14-create34.png
a726be715911484e80e2cc7425a56740c560f89025b571587e9ecec774ae6a3a  docs/design/assets/blue/15-create35.png
1be4f04f759ddc492f02884da85a033b69c467b7fbdad83aba25c26f9618efa2  docs/design/assets/blue/17-discover.png
eba0578060f86e16cb5114e17081a7ba1a449e78235cfcd318dca3ff889735ce  docs/design/assets/blue/18-voice.png
80c070a7575201c270bbd2ebfe3d24ee08d5635fff9f4a0a2902159117fa01fa  docs/design/assets/blue/20-voice-room.png
5a81cdc42ab14be835e2038f3578c07a32f7cb4bdea9f67501293e1c7fb87997  docs/design/assets/blue/contexts/09-prejoin-designbesprechung.png
38ccc6b663c727aaeaac39df58e8efc01e82dd7de75f97e73dd03e8a95b8d0f2  docs/design/assets/blue/contexts/09-prejoin-teamkueche.png
58172e0c0fac2cda8a37b348164bb55e44b0e69051e557e92b1d3f6910141f7b  docs/design/assets/fonts/Manrope-LICENSE.txt
3ae11c49db0455a3cc33e37d380f20fdb8c7f8b41dc07625c177e3d87a9d6ae6  docs/design/assets/fonts/Manrope-Variable.ttf
a27fe37ffd43cbace23e6a225cf19705e6c2598bbeee5ace4cec18150a576f58  docs/design/assets/sage/14-create34.png
a91ef00c9c86e8063165b33b15c0673fa5ce995b11fdfc15baabddbbfaa22d46  docs/design/assets/sage/15-create35.png
20be77a0276ad0bc4f98cb0f8bd6a4be02e7689b98545345b07070f1e9fe7170  docs/design/assets/sage/17-discover.png
e111b2615eb94a59276630dce469934ab2201d84233c88be53a94611d60d9849  docs/design/assets/sage/18-voice.png
bc1eeffdd16ea1d93a0d15ae4a7b9dcdb0cc31fd76b7d83e795f62cead67681e  docs/design/assets/sage/20-voice-room.png
4ad2f23ace0cccb503db29d761cfaa146a78c946391dd6c047bb6636994386a8  docs/design/assets/sage/contexts/09-prejoin-designbesprechung.png
ddc42d9a668bbfcd6d19842f9bb67fc5d5cd2334a45da1c95c533a90352cea1e  docs/design/assets/sage/contexts/09-prejoin-teamkueche.png
8540e9e89064f346bb8833e384883290481fde8f8e91225cd107dc6904f57e9e  docs/design/assets/superseded/blue/14-create34-v05.png
dfb9b3445ed704cf9206604b027ed8ffd73bc9182871f1a01446d22879ec1832  docs/design/assets/superseded/blue/15-create35-v05.png
43b07c63e543e2f71542bc712c6d55a0a494c11dd0e3fb9251eeee7bfb27fe58  docs/design/assets/superseded/blue/17-discover-v05.png
c508e47626dc855d8e929aca617bbb62e5224fc78f6612bda89f112e8f8ebda3  docs/design/assets/superseded/blue/18-voice-v05.png
c5354834e8a64ca1e2a668ada493a4e1d4c98dc7810f3a6a6fb905a10533034d  docs/design/assets/superseded/sage/14-create34-v05.png
07c44177142fea4d0f0de393766bf5e57d3713b89a11c51ee7c5fc0a8e2297c3  docs/design/assets/superseded/sage/15-create35-v05.png
4a83710a7ae56cb6c6cd593fbfc9556aa63587066692f6683098a3e8de527a28  docs/design/assets/superseded/sage/17-discover-v05.png
cbc60af3a8553ea97dac7f59b5e3e0e487b11e5c69894226f406062a15d85fc6  docs/design/assets/superseded/sage/18-voice-v05.png
57985abab7afa5b24fc921b7ff57b86b4e7265190249dfa20594838269fd34e3  docs/design/flows.md
b747cb415a5610ce5879d5ba0fb8672a6a289a4b1e22b6dd637aa71362654858  docs/design/principles.md
ecaa49181d3cde44f3d91887b5ac29b6289f8230c179261320cd4a3067b59124  docs/design/prototypes/blue/ui-sketch.html
1ad2ba434e2cdc89be09f724fc2d667d792082ae88c71cbd42dd06addfbfff1c  docs/design/prototypes/index.html
0dbdc741d945b63de9eb834afad1036bd7c459c02f56359f06e997bdda15e38c  docs/design/prototypes/sage/index.html
219ce85a460397b47385f3badddfe33613269d36e565a2a28fe9f9a4e97a6244  docs/design/prototypes/sage/ui-sketch.html
11e53b3122bb7ec15020467d8b720cc9584c2eb1732cee0ae71c3fa114cf590a  docs/design/prototypes/talk-model-and-compatibility.html
d0bf5bd5013a1764b4581bc8fd6b3daf4cc31a203f49d0f7bc4fb2f95d52a389  docs/product/calls.md
4bd5173d813fd6d7fedadd7dadad5f80253ff1017c37ee824fd9979a60b26ff1  docs/reviews/design-remediation-r1.md
94e47d6a743742f60dde7157023db2307f13b0b08144be80b4e9ace6360da57e  docs/reviews/design-spec-review.md
```

</details>

