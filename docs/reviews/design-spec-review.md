# Independent design specification review — round 1

**Verdict: changes requested.** The incoming direct-call decision passes this artifact review. Resolve **DR-001–DR-003** before accepting the package as a consistent design input for the architecture/module/interface bootstrap. **DR-004** is a minor editorial correction. This verdict concerns specifications and artifacts; it does not approve native functionality, compatibility, accessibility conformance, or performance.

Reviewed on **2026-10-18**. Scope: all files in `docs/design/`, `LICENSE`, `NOTICE.md`, and the relevant account/call product requirements. Only this review file was written. No Git operation, VM startup, service, build, native test, Nextcloud deployment, or publication occurred.

## Findings

### DR-001 — High: voice-room entry bypasses the required conference prejoin

**Design IDs:** D-S05, D-S09, D-S17, D-S18; D-F04, D-F06-C; D-P05.  
**Product requirement:** [CALL-004](../product/calls.md#call-004).  
**Locations:** [blue mockup](../design/prototypes/blue/ui-sketch.html), lines 629, 710, 795–804; [sage mockup](../design/prototypes/sage/ui-sketch.html), lines 596, 677, 762–771; [flows](../design/flows.md#d-f06-create-and-discover-conversations), lines 77–81. The galleries repeat the direct-entry description.

From a fresh D-S05, clicking **Teamküche · Mara ist im Gespräch** immediately renders D-S18, including **Du bist im Gespräch** and **Mikrofon an**. D-S17's **Anruf beitreten** also targets `voice` directly. No D-S09 appears. The sidebar row looks like conversation navigation, while D-F06-C calls voice-room selection an explicit join. More fundamentally, CALL-004 explicitly requires voice-room shortcuts to use the same deliberate prejoin path as other conferences. The design flow and clickable example therefore disagree with the governing product specification.

**Concrete fix:** distinguish opening the voice-room conversation from joining its call. Route an explicitly labeled join through D-S09 configured for Teamküche, then to D-S18 only after **Jetzt teilnehmen**. Update D-F06, the preset/discovery wording, gallery descriptions, and any affected current screenshot. The native client can choose a more cautious entry than another Talk client's preset behavior; the pinned [VoiceRoom preset source](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/RoomPresets/VoiceRoom.php) does not override this project's CALL-004 decision.

**Recheck:** in both palettes, conversation selection must not depict an accepted call; explicit voice-room joining must visit D-S09 with the correct identity and media intent, and cancel must remain outside the call. D-S13 **Annehmen** must still bypass D-S09. This finding concerns the illustrated transition, not evidence that the HTML captures media.

### DR-002 — Medium: voice-room sharing changes the conversation and participants

**Design IDs:** D-S18 → D-S11/D-S12; D-F02-A, D-F05; D-P02.  
**Locations:** [blue mockup](../design/prototypes/blue/ui-sketch.html), lines 712, 817–827; [sage mockup](../design/prototypes/sage/ui-sketch.html), lines 679, 784–794.

Reproduction: fresh D-S05 → Teamküche → **Teilen**. The two-person Teamküche call becomes the four-person **Produktteam** conference beneath the sharing picker. The header, participant count, call chat, and composer all change. The shared picker always renders the generic conference template; its context logic accounts for Designbesprechung but not Teamküche. This contradicts the unified conversation model and creates an incorrect transition for implementers to copy.

**Concrete fix:** retain the originating conversation and participants throughout sharing, cancelling, confirming, and stopping. If voice-room sharing is intentionally not illustrated yet, make this control explicitly illustrative without navigating to an unrelated conference, as already done for direct-call sharing.

**Recheck:** no reachable sharing action changes Teamküche into Produktteam or Designbesprechung, or changes its two representative participants to four. Direct-call sharing must continue to preserve Mara/Alex and remain clearly pending if unimplemented.

### DR-003 — Medium: cancelling a meeting prejoin loses the originating conversation

**Design IDs:** D-S16 → D-S09; D-F04-B, D-F02-A.  
**Locations:** [blue mockup](../design/prototypes/blue/ui-sketch.html), lines 704, 827–828; [sage mockup](../design/prototypes/sage/ui-sketch.html), lines 671, 794–795; [D-F04-B](../design/flows.md#d-f04-voluntarily-join-a-conference).

Reproduction: D-S16 **Designbesprechung** → **Teilnehmen** → D-S09, correctly titled Designbesprechung → **Zurück zum Gespräch**. The result is D-S05 **Produktteam**, and the meeting context resets to `product`. D-F04-B promises return to the originating conversation. Unlike the direct-call hang-up shortcut, this deviation is not explicitly identified in that flow.

**Concrete fix:** preserve the originating conversation when opening prejoin and use it for cancellation/back navigation.

**Recheck:** Designbesprechung → prejoin → back restores D-S16, including its header and composer; Produktteam → prejoin → back restores D-S05. No join occurs on cancellation. This does not require implementing the separately acknowledged missing direct-conversation navigation.

### DR-004 — Low: current gallery counts and revision labels are stale

**Design IDs:** D-S01–D-S19, especially D-S19.  
**Locations:** [combined gallery](../design/prototypes/index.html), lines 1, 19, 21; [sage gallery](../design/prototypes/sage/index.html), lines 1, 10, 12; [revision 05 inventory](../design/artifacts.md#revision-05).

Both gallery datasets correctly contain **19** screens: four login, eight conversation, and **seven** call views. Both current navigation bars still say **Anrufe 6** and their toolbars say **18 Zielansichten**. Titles/toolbars still refer to revision 04 or 03, while the package documents revision 05. The combined gallery simultaneously advertises 19 views.

**Concrete fix:** derive visible counts from the screen data and align or remove the obsolete current revision labels. Historical documents may retain their original counts when clearly identified as historical.

**Recheck:** both galleries consistently show 19 total / seven call views, and the new direct-call view remains selectable.

## Independently checked evidence

- **Direct-call entry passes at mockup level in both palettes:** D-S13 **Annehmen** → D-S19; exactly two video tiles, Mara Fischer and Alex Sommer; count two; personal chat and composer; no prejoin heading. The incoming card states no microphone/camera before acceptance and microphone-on/camera-off on acceptance. D-F03-C/D and CALL-001/002/009/011 retain permission, consent, publishing-policy, and security gates.
- **No accidental Enter acceptance was observed:** pressing Enter while the incoming card itself held focus left D-S13 unchanged in both palettes. Source inspection found no media-capture or networking APIs. This proves neither native notification focus safety nor native device behavior.
- **Direct-call sharing stays in D-S19 with two participants.** Its button is intentionally illustrative, consistently with D-F05-C. D-F03-F explicitly discloses the shortcut back to the sample workspace after hang-up; personal conversation navigation remains pending.
- **Conference forward entry remains distinct:** the Produktteam join route reaches D-S09 with **Bereit, wenn du es bist.**; **Jetzt teilnehmen** reaches D-S10 with four illustrative participants. Designbesprechung entry correctly names that conversation in prejoin; its cancellation defect is DR-003.
- **Inventory and geometry:** both mockups expose 19 unique screen states and have identical navigation scripts. All 38 rendered states fit the 1280 × 900 reference page horizontally, with a 1120 × 760 schematic window. There are 38 current PNGs and two superseded PNGs, all 1280 × 900. All 36 inherited PNGs match the supplied original design set byte for byte, including the two originals moved to `superseded/`.
- **Representative visual inspection:** live blue/sage incoming views and the blue active direct-call view; exact stored PNGs `blue/09-prejoin.png`, `blue/13-incoming.png`, `blue/19-direct-call.png`, and `sage/19-direct-call.png`. Identity, two-person direct-call layout, camera-off presentation, and palette geometry agree. This is representative inspection, not a claim to have visually audited every PNG.
- **Index and links:** 126 local reference occurrences resolved, five Markdown fragment links resolved, and no duplicate literal HTML IDs were found. Both gallery datasets contain D-S01–D-S19 counterparts and existing image files. In the combined gallery, selecting D-S19 and switching sage/blue preserved the selection and switched to the corresponding asset path. Browser resource loading from the original relative URLs was not exercised; see the method limitation below.
- **Design/accessibility targets:** the principles are concise and correctly distinguish conversations, invitation-source user groups, presets, guest access, and discoverability. Capability/configuration/effective-permission gates take precedence over synthetic version fixtures. The font loaded in both mockups; the bundled variable font's weight axis spans 200–800. Recalculated token contrast is 5.01:1 for white/blue primary, 8.95:1 for white/sage primary, and 5.11:1 / 5.88:1 for the proposed secondary-text/subtle-surface pairs. The historical blue muted/white pair is 4.25:1, already acknowledged as design debt. The measurable text, control, focus, and scaling targets are reasonable uses of [WCAG 2.2](https://www.w3.org/TR/WCAG22/); the documents correctly avoid claiming native conformance and identify hit areas as project targets.

## License, provenance, and public-file checks

`LICENSE` is **13,792 bytes**, byte-for-byte identical to the pinned [SPDX EUPL-1.2 text](https://github.com/spdx/license-list-data/blob/b8d6af45ad2fcfed61bb85a8ad068aa4a77eadf9/text/EUPL-1.2.txt). Its hash matches NOTICE and the manifest below. The [SPDX entry](https://spdx.org/licenses/EUPL-1.2.html) identifies the license as EUPL-1.2. This is an artifact comparison, not a broader legal opinion.

The bundled Manrope font and complete copyright/OFL notice exactly match the pinned [Google Fonts distribution](https://github.com/google/fonts/tree/b31870aff700ab7a1d74fa0c6887d95beb9e0037/ofl/manrope). All **four** embedded font payloads—two mockups and two galleries—match the bundled TTF. Both self-contained editable mockups include the full notice; the repository also supplies the separate notice file. NOTICE explicitly preserves the font's OFL-1.1 license rather than applying EUPL to it.

The inspected design sources contain inline HTML/CSS/SVG/JavaScript and the identified font, with no external script imports. No Buzz source/assets or additional identifiable vendored third-party implementation were found. This is inventory/source evidence and the supplied provenance statement, not an exhaustive independent authorship audit. No credential or absolute private-path findings arose from the reviewed text, source, or representative screenshots. Example server and guest-link identities are visibly fictional.

## Pending gates, not failed native tests

The package clearly says it is a schematic design for a future Rust/GPUI client, with no embedded WebView. Missing native states remain requirements, including:

- OS secret-store success, locked/denied/unavailable states, safe session-only fallback, credential lifecycle, and account isolation. D-P09/D-F01 and AUTH-003–009 describe these without claiming implementation.
- Permission denial, receive-only/blocked participation, consent, lobby, security/E2EE negotiation, connecting/reconnecting/ended states, invitation races, and duplicate acceptance. CALL-001–014 must govern the later native implementation.
- Native sharing permissions/picker, capture lifecycle, microphone/camera publication, recording enforcement, real server capability changes, and both supported Nextcloud/Talk families.
- VoiceOver, full keyboard/focus behavior, 200% text scaling, dark/contrast appearances, narrow layouts, and all composited contrast pairs.
- Real interoperability and measured startup, resource use, latency, and media performance.

Apple's [Keychain services](https://developer.apple.com/documentation/security/keychain-services), [capture-authorization guidance](https://developer.apple.com/documentation/avfoundation/requesting-authorization-to-capture-and-save-media), and [ScreenCaptureKit presentation](https://developer.apple.com/videos/play/wwdc2023/10136/) are platform references, not evidence that this repository has implemented those integrations. Native/integration checks must follow the architecture/module/interface bootstrap; none was substituted with screenshot evidence.

## Review method and snapshot

One newly created T3-owned browser tab was used; no other tabs were operated. T3 rejected `file:` navigation, so the original self-contained HTML was transferred into the blank tab in memory. Initial states were selected through the mockup's existing `renderScreen` function; reviewed transitions used the original click handlers and browser keyboard input. Stored PNG bytes were displayed in the same tab without changing files. Relative file references were checked statically. Query-string initialization and a served/local-file gallery load are not independently verified by this method. The tab was closed afterward.

The agent-dev-box session remained unprovisioned (`new`), with **0 seconds VM time and EUR 0 estimated cost**; there was no VM to hibernate. No environmental blocker prevented this artifact review. The unresolved blockers are DR-001–DR-003, which should be corrected by their owners and independently rechecked.

The following **58 repository-relative inputs** were hashed at **2026-10-18T14:57:50.076574+00:00**. No Git snapshot existed. The same hashes were confirmed again during the review.

At the final check, a concurrent update to `docs/product/calls.md` was detected and the complete updated file was re-read at **2026-10-18 15:08:23 UTC**. Its current SHA-256 is `3df4c873af55bfcd8b0aaa0e0a8eac98677ecc6c8e318122cf9b4cd3d6743c98`. The additions concerning incoming-notification focus and advertised maximum-duration policy do not change these findings; CALL-004 still requires voice-room prejoin. The other 57 inputs still match the initial manifest below. Later changes require rechecking; this report does not silently approve subsequent concurrent edits.

<details>
<summary>Reviewed input SHA-256 manifest</summary>

```text
57fb42fbcd0b037ce528ed8f72f1ec095d67bc6825ecf1448ff39be1fe68a4b4  LICENSE
32f132d980dc38fb62a767b30f33dd98f87fb60ce4d24e3e16455ace6e408683  NOTICE.md
405c778396dddea1c4e00ee4869c646031e4e15b323a252b670b0e597793b4da  docs/design/README.md
a4de545e2f3c515284d05fb694b1f5abe96f874fe2ed0feba9299954e1560317  docs/design/artifacts.md
22ce0c029d9b527f2a1fe9dc8cd92bf52107d7bd4c04061b7222ad761b8c5eab  docs/design/assets/blue/01-server.png
a25579db888a7f2d8e80e75c2f3845d3a9b7d19513d393e0a2a3368519f3c44e  docs/design/assets/blue/02-browser.png
a82f1ad2f1f8ae067b7dd4845008af9b7c6968ddd6f4c54749f4cbe63ca82285  docs/design/assets/blue/03-ready.png
8680be481b4bfdd43814a9ba5577f243922d14f7401a2a5f0167e574849f530e  docs/design/assets/blue/04-expired.png
1ddc38706864b31c9347b0e0fda58871e1cc18ff9d8235852ebc39205f8b256b  docs/design/assets/blue/05-chat.png
db27b025903c13c891eacce17ea49dc228613a5bf56e30b976700b8837360619  docs/design/assets/blue/06-thread.png
4f807ef283c58f0aea790acb30b37a860e61a628bb065197dfa43b69f5e163fc  docs/design/assets/blue/07-search.png
548074d8112dd182d518e51f44040c6973290dcba48b69a066171738317eeb0c  docs/design/assets/blue/08-settings.png
9427c089d0b0a0fe8a8cac8c1d4fb191c60f2c6e0073c1773e9be58d69baa5ae  docs/design/assets/blue/09-prejoin.png
62d7407506af33c5f2a964c2280b6845dd668f7c97a0ef1f32f7d49a669937fd  docs/design/assets/blue/10-call.png
176c1bfb834af0d9ab497fd8a11a10f11e582b7c4a3c7cab1e5f317a9f6cc594  docs/design/assets/blue/11-sharepicker.png
716686b05d1805bc349cf88f5b438b0ceb770c5683d306a9309a137aa2e8b328  docs/design/assets/blue/12-sharing.png
d4126d56b6d568b99d22da5eb48994f3750c3da4905453ef6847f2d57487e31a  docs/design/assets/blue/13-incoming.png
8540e9e89064f346bb8833e384883290481fde8f8e91225cd107dc6904f57e9e  docs/design/assets/blue/14-create34.png
dfb9b3445ed704cf9206604b027ed8ffd73bc9182871f1a01446d22879ec1832  docs/design/assets/blue/15-create35.png
af1b8c6122ea4861dbda06fac450c373411f6307adb1fd698b5b7c3b4e326bce  docs/design/assets/blue/16-room.png
43b07c63e543e2f71542bc712c6d55a0a494c11dd0e3fb9251eeee7bfb27fe58  docs/design/assets/blue/17-discover.png
c508e47626dc855d8e929aca617bbb62e5224fc78f6612bda89f112e8f8ebda3  docs/design/assets/blue/18-voice.png
d8b57bb1c9cd61e3ae96905ed75aec2f4841de94971d1c87f0836aa93d57ce7a  docs/design/assets/blue/19-direct-call.png
58172e0c0fac2cda8a37b348164bb55e44b0e69051e557e92b1d3f6910141f7b  docs/design/assets/fonts/Manrope-LICENSE.txt
3ae11c49db0455a3cc33e37d380f20fdb8c7f8b41dc07625c177e3d87a9d6ae6  docs/design/assets/fonts/Manrope-Variable.ttf
e9530d0f1e6d4bafa9d70a2afa85bf5318b50bb7ff9a878210a6f7b161759c5b  docs/design/assets/sage/01-server.png
50b43a463534c58bbb3b6a297a559ed5f54b91926b4dbabbfc97cdba653a97df  docs/design/assets/sage/02-browser.png
870de8707e0a38bc92e1692c9f8f630b576ea92f55f18a72b54b53c77af10d4b  docs/design/assets/sage/03-ready.png
d77c148b0a2dad7f3fcfeccedccfab0d7c7f90461ab48116faa539577785a5c8  docs/design/assets/sage/04-expired.png
275bcda18dbcfa240eddae102c5dc92cc4d6dd61012515b6bf5504ed3669145c  docs/design/assets/sage/05-chat.png
6f0ca68193af4cccdbf113a4ad9584db6669be5ead703ffcd67cbc899e7c3dcd  docs/design/assets/sage/06-thread.png
a493ca53047278b3cafd868f4d84a71d0cc0bb4e45dbda2b5f8c250b3e6b01f6  docs/design/assets/sage/07-search.png
88e50f5145cff5efcd354cd9a0972798ebfac3907cd7db601aa2c04592d2a03f  docs/design/assets/sage/08-settings.png
8e8ca1885989ff49e2bfda6d19ea21455beb1706039cec88ab3d3a749c72de8b  docs/design/assets/sage/09-prejoin.png
fce33c5bd91d8a0a1cca59094acfc99814627b77aa6453e6f56ec087a4144250  docs/design/assets/sage/10-call.png
5f97ffef483bddfacddddcf253d065520d24257c38641beda5cf27eda6f97233  docs/design/assets/sage/11-sharepicker.png
30746635dcb246b32c48835e22c53dbbd04d9128d6e4227bd66561c6166f4fb5  docs/design/assets/sage/12-sharing.png
476ffa7944188fed0cfea6f2f344dd533722a18b6d0b7fcb111aaf14b904f7cc  docs/design/assets/sage/13-incoming.png
c5354834e8a64ca1e2a668ada493a4e1d4c98dc7810f3a6a6fb905a10533034d  docs/design/assets/sage/14-create34.png
07c44177142fea4d0f0de393766bf5e57d3713b89a11c51ee7c5fc0a8e2297c3  docs/design/assets/sage/15-create35.png
fd4b633428228918cde6d3a0a67e6a5bed340cd26ddeb08ede81298dd9b21f59  docs/design/assets/sage/16-room.png
4a83710a7ae56cb6c6cd593fbfc9556aa63587066692f6683098a3e8de527a28  docs/design/assets/sage/17-discover.png
cbc60af3a8553ea97dac7f59b5e3e0e487b11e5c69894226f406062a15d85fc6  docs/design/assets/sage/18-voice.png
fcf14a92231e1b0404bda62930cacc59507d9db83c39b6b9a3cf58e76c2c378f  docs/design/assets/sage/19-direct-call.png
355ad40dce623d35b1bf0adc29be9c38d63390b3620fb661994b84378246b446  docs/design/assets/superseded/blue/13-incoming-v04.png
8da6c8e94c66e18e545f02a02f054b567a4876fbd9c4a43cd237c3a57924cc97  docs/design/assets/superseded/sage/13-incoming-v04.png
1323959dacf858c3d1491dccf9bc47ee2c3fc758eea6a7f8e4f219eb3e97c299  docs/design/flows.md
28c627d2f95bb616fdd9d5c1263731364fa4d5a333b9f420588a9baa47a9bced  docs/design/principles.md
4595c73e1f0ce4f8e805b494eb7ee8561cea0cb8e2c29624c8d27cae0afe8f4b  docs/design/prototypes/blue/palette-blue.css
9c636f81ef90eeffc681361ba9f39f39f91698e5f9048ca9b0c565e4a23c29e0  docs/design/prototypes/blue/ui-sketch.html
ab71ac58301866f021291d2769882aa12ee6c438e72065988e93f51182567e5e  docs/design/prototypes/index.html
4601fb2d2e0342a815e4eb28d53d32861c9304878eee49aa9bb2a35ca35dac72  docs/design/prototypes/sage/index.html
171d28302f406c773c8653727c735a2d667155641aa41c6595bb22e34384f7d4  docs/design/prototypes/sage/ui-sketch.html
b9f76f2b35465b54a82ff3c99be264fd448b9b71a96d0e8ae419850f8fff06e1  docs/design/prototypes/talk-features.html
7c1f43691dbb9bde55c40a2f2ba2f0418db0f9f09b77f0183d2fac6fb224cd55  docs/design/prototypes/talk-model-and-compatibility.html
3dbc4fa127076c873d7fe514ca62d142b2111c372f527591c4fe3d57ae46165e  docs/design/tokens.md
0b2d79a18966e1733703f9137143b5a36748d6fdbd78ca4fc30b50e9b5a5c0f6  docs/product/accounts.md
d5e217440cb30bc545060079f4a916dae1cfa0bdb50d909c831d98155103e1c4  docs/product/calls.md
```

</details>

