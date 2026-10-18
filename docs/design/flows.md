# Flows and screen map

Stable IDs below can be referenced by product specifications and acceptance scenarios. All screens are schematic. A target not pictured remains a requirement; a pictured control is not proof of implementation or support.

## Screen index

The two palettes share **20 screen IDs** and behavior (four account, nine conversation and seven call views). Open either `prototypes/<palette>/ui-sketch.html?screen=<state>` locally to inspect an editable view.

| ID | State | View / scope | Blue | Sage |
| --- | --- | --- | --- | --- |
| D-S01 | `server` | Server address | [PNG](assets/blue/01-server.png) | [PNG](assets/sage/01-server.png) |
| D-S02 | `browser` | Await browser authorization | [PNG](assets/blue/02-browser.png) | [PNG](assets/sage/02-browser.png) |
| D-S03 | `ready` | Account confirmation / notifications | [PNG](assets/blue/03-ready.png) | [PNG](assets/sage/03-ready.png) |
| D-S04 | `expired` | Expired login, retry | [PNG](assets/blue/04-expired.png) | [PNG](assets/sage/04-expired.png) |
| D-S05 | `chat` | Unified conversation list and group timeline | [PNG](assets/blue/05-chat.png) | [PNG](assets/sage/05-chat.png) |
| D-S06 | `thread` | Replies in context | [PNG](assets/blue/06-thread.png) | [PNG](assets/sage/06-thread.png) |
| D-S07 | `search` | Search overlay | [PNG](assets/blue/07-search.png) | [PNG](assets/sage/07-search.png) |
| D-S08 | `settings` | Appearance / readability settings | [PNG](assets/blue/08-settings.png) | [PNG](assets/sage/08-settings.png) |
| D-S09 | `prejoin` | Voluntary conference device check | [PNG](assets/blue/09-prejoin.png) | [PNG](assets/sage/09-prejoin.png) |
| D-S10 | `call` | Four-person conference, separate from a direct call | [PNG](assets/blue/10-call.png) | [PNG](assets/sage/10-call.png) |
| D-S11 | `sharepicker` | Schematic substitute for the system sharing picker | [PNG](assets/blue/11-sharepicker.png) | [PNG](assets/sage/11-sharepicker.png) |
| D-S12 | `sharing` | Conference with sharing active | [PNG](assets/blue/12-sharing.png) | [PNG](assets/sage/12-sharing.png) |
| D-S13 | `incoming` | Incoming direct call from Mara, revised | [PNG](assets/blue/13-incoming.png) | [PNG](assets/sage/13-incoming.png) |
| D-S14 | `create34` | Synthetic NC34/Talk24 preset example | [PNG](assets/blue/14-create34.png) | [PNG](assets/sage/14-create34.png) |
| D-S15 | `create35` | Synthetic NC35/Talk25 preset example | [PNG](assets/blue/15-create35.png) | [PNG](assets/sage/15-create35.png) |
| D-S16 | `room` | Group meeting conversation, chat/files/call/guest access | [PNG](assets/blue/16-room.png) | [PNG](assets/sage/16-room.png) |
| D-S17 | `discover` | Discoverable conversations | [PNG](assets/blue/17-discover.png) | [PNG](assets/sage/17-discover.png) |
| D-S18 | `voice` | Active voice-room preset; two people in a group room | [PNG](assets/blue/18-voice.png) | [PNG](assets/sage/18-voice.png) |
| D-S19 | `direct-call` | Accepted direct call; Mara and Alex only | [PNG](assets/blue/19-direct-call.png) | [PNG](assets/sage/19-direct-call.png) |
| D-S20 | `voice-room` | Teamküche conversation, not joined; added in revision 06 | [PNG](assets/blue/20-voice-room.png) | [PNG](assets/sage/20-voice-room.png) |

D-S09 retains one screen ID across conversation contexts. Its default PNG is Produktteam; contextual captures are [Teamküche blue](assets/blue/contexts/09-prejoin-teamkueche.png) / [sage](assets/sage/contexts/09-prejoin-teamkueche.png) and [Designbesprechung blue](assets/blue/contexts/09-prejoin-designbesprechung.png) / [sage](assets/sage/contexts/09-prejoin-designbesprechung.png). These four supporting captures are not additional gallery screens. Use `?screen=prejoin&conversation=voice` or `&conversation=design` to open those contexts. The Teamküche check depicts devices still off and states the microphone-on / camera-off intent for final join; the retained video-conference sketch illustrates a deliberately authorized local preview, not permission to start one automatically.

## D-F01 Connect account

`D-S01 → D-S02 → D-S03 → D-S05`; expired authorization branches to `D-S04 → D-S02`.

- **D-F01-A:** Show the server identity before opening the system browser. Password and 2FA entry remain in Nextcloud. Do not add a password form or embedded browser to the native client. This follows [Nextcloud Login Flow v2](https://docs.nextcloud.com/server/34/developer_manual/client_apis/LoginFlow/index.html#login-flow-v2).
- **D-F01-B:** Persist the app credential only through the OS secret store. On failure offer **Erneut versuchen**, **Abbrechen**, and explicit **Nur für diese Sitzung fortfahren**. Session-only mode holds the credential in process memory, marks the account accordingly and requires reauthentication after restart; cancellation discards the pending credential. No plaintext file, settings, log, clipboard or diagnostics fallback. If session-only operation cannot meet that contract, stop sign-in with a recoverable explanation.
- **D-F01-C:** A failed/expired authorization leaves a retry path without exposing tokens. Notification permission is optional and separate from authentication. Secure-storage failure and session-only mode need dedicated views; they are not drawn in `D-S03`.

## D-F02 Work within a conversation

`D-S05 ↔ D-S06 / D-S07 / D-S08`; select a meeting conversation to open `D-S16`, or Teamküche to open `D-S20` without joining its call.

- **D-F02-A:** Chat, file references, participants and calls share the same account-scoped conversation identity. Show the selected conversation consistently in header, composer, results and call chat.
- **D-F02-B:** Preserve the draft and reading position when opening replies, searching or returning from a call. Pending, failed and retryable message states must be distinct from delivered messages; these error states are not drawn.
- **D-F02-C:** “Notiz an mich” is a note-to-self conversation, not a colleague to call. Nextcloud user groups appear in invitation selection, not as an alternative conversation namespace. Respect per-action permissions and unavailable-feature explanations.

## D-F03 Accept a direct call

`D-S13 → Annehmen → required permission/consent/security gates, if any → connecting → D-S19`.

- **D-F03-A:** Identify the caller and account. Offer **Annehmen** and **Ablehnen**. The incoming example is a personal call from Mara, even if a different conversation was open underneath.
- **D-F03-B:** A single intentional acceptance enters the direct-call flow. **Bereit, wenn du es bist.** and `D-S09` are absent. The active target contains exactly Mara and Alex, with two participant tiles and a count of two; it does not reuse the four-person conference.
- **D-F03-C:** Ringing never activates microphone, camera or their previews. Permission prompts, recording consent and mandatory security requirements can delay/block joining or media activation. Declining a requirement cannot silently bypass it.
- **D-F03-D:** The illustrated media choice is microphone on after acceptance and permission, camera off until explicitly enabled. The incoming card states that choice. This is the representative case, not an implementation of account preferences or a rule that overrides a permission denial.
- **D-F03-E:** Decline, caller cancellation, an already-ended call and connection failure leave media off and show the actual outcome. Prevent duplicate acceptance. Merely receiving the call must not steal focus and turn an unrelated Enter key into acceptance. The active view must distinguish connecting from connected.
- **D-F03-F:** Hanging up releases capture and returns to the same personal conversation. The mockup shortcuts return to the example workspace; full conversation navigation and all exceptional states require native implementation.

## D-F04 Voluntarily join a conference

`D-S05 or D-S16 → explicit join/start action → D-S09 → Jetzt teilnehmen → required gates/lobby → D-S10`.

`D-S20 → Anruf beitreten → D-S09 (Teamküche) → Jetzt teilnehmen → required gates/lobby → D-S18`.

The explicit Teamküche shortcut in `D-S17` also opens `D-S09 (Teamküche)`. This selects Teamküche as the conversation context; cancellation opens `D-S20`, still outside its call.

- **D-F04-A:** Show **Bereit, wenn du es bist.** only after the user chooses to join/start the conference. Allow device selection and explicit microphone/camera choices before participation. A local preview requires a deliberate preview/test action and OS authorization; it is not published to other participants. Choosing a conversation or its voice-room preset is not a join or preview action. Initial media intent must honor server policy and user settings; the Teamküche example states microphone on at final join and camera off.
- **D-F04-B:** Cancel/back (including Escape in the sketch) returns to the originating conversation without joining and releases any preview capture. Check `D-S16 → D-S09 → D-S16` for Designbesprechung, `D-S05 → D-S09 → D-S05` for Produktteam, and `D-S20 → D-S09 → D-S20` for Teamküche; header, composer and selected sidebar entry must agree. Unavailable or denied devices have a recoverable explanation. Where policy permits, offer participation with those devices off.
- **D-F04-C:** Recheck permissions and meeting state when joining. Lobby, required consent and security policy are not bypassed by the prejoin button. Use authoritative participant counts rather than the sketch's fixed four-person conference or two-person voice-room example. [CALL-004](../product/calls.md#call-004) governs all voluntary entry paths.
- **D-F04-D:** Changing local mute/camera state never changes the conversation's guest access, discoverability or call-start permissions.

## D-F05 Share and stop sharing

`D-S10 → D-S11 → D-S12 → stop sharing → D-S10`.

- **D-F05-A:** A deliberate sharing action opens the supported native picker and any necessary system permission flow. Cancellation publishes nothing and returns to the same call. In both Produktteam and Designbesprechung, picker cancellation, confirmation and stopping preserve conversation, call chat and participant context. `D-S11` is a drawing, not an exact macOS dialog or a custom replacement to ship.
- **D-F05-B:** Identify what is shared and keep **Freigabe beenden** reachable. Stop capture on stop, revoked permission or call termination; do not leave an apparently active share indicator after capture fails.
- **D-F05-C:** Voice-room and direct-call sharing must retain the originating conversation and participants. Their sharing flows are not drawn. Teamküche **Teilen*** is visibly marked as an illustration and stays in `D-S18` with Mara and Alex, count two, and the Teamküche composer; no picker, confirmation or stop state is claimed. Direct-call **Teilen** likewise stays in `D-S19`. Only the four-person conference depicts `D-S11/D-S12`; those states must never substitute for either two-person call.

## D-F06 Create and discover conversations

`D-S05 → D-S14 or D-S15 → invitation/access setup → D-S16`; `D-S05 → Teamküche sidebar → D-S20`; `D-S17 → explicit Teamküche join → D-S09 (Teamküche) → D-S18`.

- **D-F06-A:** Resolve available presets from server evidence. `create34` and `create35` are example fixtures, not hardcoded version gates. Server-forced values stay visibly locked. A Nextcloud user group can supply invitees without becoming the conversation's identity.
- **D-F06-B:** Separate guest access, password, directory visibility and membership. Show the effect of each choice before creating/sharing a link. Do not infer discovery from a public link.
- **D-F06-C:** Opening the Teamküche conversation is navigation only (`D-S20`), with microphone/camera off and no joined-call state. Its separately labeled **Anruf beitreten** action and the discovery shortcut both require `D-S09`; only **Jetzt teilnehmen**, followed by required gates, leads to `D-S18`. Back from prejoin returns to Teamküche without joining. Apply permission, consent and security gates; no preset or version label bypasses this path. The two people shown in `D-S18` still belong to a group room; use `D-S19` for the direct-call case.
- **D-F06-D:** Read expiration, moderation and call availability from the conversation/server. “1 Stunde” in the voice-room example is illustrative, not a universal retention rule. See the pinned [VoiceRoom preset](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/RoomPresets/VoiceRoom.php).

## Coverage limits

Voice/direct sharing, configurable device intent and the full preview/test lifecycle are not implemented by these sketches. No dedicated target screens yet cover restricted/unknown capabilities, secret-store failure, OS prompts, lobby/consent/security failure, reconnecting calls, offline drafts, guest onboarding, note-to-self detail, moderation, full dark appearance, text scaling or narrow-window layouts. Product specifications must retain their requirements and gates despite these gaps. Nothing here claims full Talk parity or successful tests against either supported server family.
