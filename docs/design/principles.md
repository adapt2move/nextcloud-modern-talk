# Design principles

These are acceptance targets for a future native client. See the [design status](README.md) before treating a sketch as evidence of implemented behavior.

| ID | Rule | Acceptance criterion |
| --- | --- | --- |
| **D-P01** | Use native macOS conventions. | GPUI content works with platform window controls, menus, text editing, focus, notifications and permission dialogs. Settings are reachable through the app menu and `⌘,`. A simulated title bar or sharing picker in HTML is never the native implementation. |
| **D-P02** | One conversation contains chat, files and calls. | Moving between the timeline, files, participants and a call preserves conversation and account identity. The list supports 1:1, group, public guest-access conversations and note-to-self. A voice room, webinar or presentation is a preset/attribute configuration, not an invented parallel room type. |
| **D-P03** | Separate identity, access and discovery. | Nextcloud user groups are invitation sources, not Talk conversation identities. Guest-link access, password protection, directory visibility and membership are distinct controls. A public link does not imply a discoverable conversation. |
| **D-P04** | Resolve each action from server evidence. | Creating, calling, sending, reacting, inviting, moderating and sharing have separate gates. Use advertised capabilities, returned presets, configuration and effective rights. Forced values remain locked with an explanation. Unknown restrictive state never silently becomes an ordinary editable group. |
| **D-P05** | Respect the user's entry intent. | An incoming direct call offers **Annehmen** and enters the direct call without conference prejoin. Opening a conversation, including a voice room, joins no call and captures no media. A deliberate conference join, including a voice-room shortcut, opens the device check and retains its conversation on cancel. Permission, consent and security gates apply to both; ringing alone captures no media. |
| **D-P06** | Make state readable. | Distinguish ringing, connecting, active, reconnecting, declined, ended and failed calls. Show microphone, camera, sharing and recording states with text and symbols. Presence or an active-speaker outline never implies verified identity or encryption. |
| **D-P07** | Keep the workspace calm and aligned. | Use one type family, consistent icon boxes and shared text baselines. Blue indicates primary actions and selection; semantic colors retain their meaning in both palettes. Content remains the strongest visual element. |
| **D-P08** | Make the full flow accessible. | Keyboard and assistive-technology users can authenticate, find a conversation, send a message, answer and leave a call. Focus stays visible, restores after a dialog and never triggers acceptance or media capture just by moving. Apply the measurable targets in [tokens.md](tokens.md#accessibility-targets). |
| **D-P09** | Keep credentials out of ordinary storage. | Authentication stores app credentials through macOS Keychain or the appropriate OS secret store. If unavailable, show retry/cancel and an explicit session-only option; do not silently write a plaintext credential. Explain that session-only mode requires signing in again after restart. |

## Platform behavior

Use the system browser for the Nextcloud login flow. Use the actual macOS content-sharing picker where supported; the drawn chooser in `D-S11` communicates selection and cancellation only. Respect system text editing, Full Keyboard Access, VoiceOver, Reduce Motion, Reduce Transparency and Increase Contrast. No native or assistive-technology integration is demonstrated by the HTML prototypes.

A dialog restores focus to its invoker. `Escape` dismisses a transient surface; it must not unexpectedly hang up an active call. Provide discoverable menu commands for shortcuts. The sketch's `⌘K` search is a proposed app shortcut, not evidence that all native menu commands exist.

These targets follow Apple's [keyboard guidance](https://developer.apple.com/design/human-interface-guidelines/keyboards) and [accessibility guidance](https://developer.apple.com/design/human-interface-guidelines/accessibility). Apple's [ScreenCaptureKit introduction](https://developer.apple.com/videos/play/wwdc2023/10136/) describes the system sharing picker; [Keychain services](https://developer.apple.com/documentation/security/keychain-services) provides secret storage on macOS.

## Domain evidence

The conversation terminology follows the pinned Talk [24.x constants](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/docs/constants.md) and [25.x Conversation API](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/conversation.md). Presets and their availability are server-driven; see the pinned [24.x preset factory](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/lib/Service/RoomPresetFactory.php) and [25.x capabilities](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/capabilities.md). These links support the model, not a claim of client compatibility testing.
