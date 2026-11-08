# Desktop and accessibility

Use native GPUI and platform conventions. macOS is primary, Linux is a development and packaging target, and Windows is compile-only. See [known limitations](../limitations.md) for qualification; native behavior cannot be established by fixtures or compilation.

## DESK-001

Keep search, Activity and conversation navigation small, with details in contextual dialogs. Resize, sidebar toggles and thread navigation preserve selection/drafts; critical errors and call leave/stop controls remain reachable in narrow windows. Demo data is visibly fictional; browser handoffs are explicit.

## DESK-002

Combine server/room eligibility, local category choices, sensitive policy and OS permission/Focus without promising delivery or bypassing Do Not Disturb. Four independent banner/sound categories cover messages, important notices (conference participation and server reminders), incoming direct calls and optional Calendar. Calendar defaults off and needs verified source integration. Calls retain
separate capacity/lifecycle under message overload. Deduplicate; ordinary message banners need background eligibility. OS permission checks/tests do not prompt; requests are deliberate. Saved settings are not observed grants. Activation revalidates the exact account/room/message/call; inaccessible targets expose nothing. State quit/polling limits honestly. Fixed openly licensed sounds retain
attribution; no custom picker or separate Focus-bypassing notification player is required.

## DESK-003

Keep presence/custom status/profile, call participation, read privacy, typing and notification mute distinct. Unknown/stale status is not offline. Changes require supported server APIs and confirmed results; no false offline status on network loss.

## DESK-004

Separate device-local preferences from account/server policy. Preserve explicit choices, enforce locked server values and omit/explain unsupported controls. Correlate bounded asynchronous writes by revision; failure restores confirmed state with explicit Retry and cannot replace newer edits. Accounts in Settings show connection and storage/recovery separately. Permission setup is optional:
explicit entry authorizes one supported request per step, never capture. Chat works without grants. Privacy/settings checks and OS handoffs do not grant access. Device checks require permission; camera preview is explicit and disposal completes before a new capture owner starts. Settings or navigation cannot silently unmute/change source.

## DESK-005

All shipping actions need keyboard access, visible focus and meaningful semantic names/roles/values/status. Modal focus stays contained and returns to its invoker; Escape and shortcuts respect IME and cannot accidentally accept a call. Qualify the actual GPUI accessibility tree with VoiceOver for critical chat/call/account flows; keyboard support and screenshots alone do not establish semantic
accessibility.

## DESK-006

Support light/dark, contrast, reduced motion and 200% text without hiding controls. Unknown motion preference suppresses decoration. No essential state relies on color or hover alone. Text selection/copy obeys current privacy; hidden/replaced content loses export authority. Keep caret, focus, text input and accessibility distinct. Decorative transitions never delay input, change delivery/reading
authority or drive idle redraw loops. Shared dialogs and controls keep consistent spacing and focus.

## DESK-007

Start eligible accounts from bounded cache and retain history/drafts through offline periods. Explicit submissions enter [MSG-003's queue](messaging.md#msg-003); typing alone does not. Show one quiet account-specific connection pill after the five-second grace period, with reason, queued count and time since observed failure, never a guessed last-sync time. Reconnect refreshes authority before
dispatch; full room reconciliation handles removals absent from incremental updates. One offline account cannot block another; cached state never renews authority.

## DESK-008

Close, hide, quit, sleep, crash and logout have distinct lifetimes. On macOS close hides the retained window and background services; activation restores context, while hidden content cannot advance reads. Stop local device checks/playback on hide; established calls have their own owner. Explicit quit drains bounded work and stops capture. Other platforms close with their last window. Wake
revalidates policy and devices; restart never captures automatically. Server schedules/reminders remain server-owned.

## DESK-009

Errors offer scoped retry/settings/relogin/leave as appropriate. Bounded private rotating diagnostics contain typed/redacted facts, never credentials, message bodies, private URLs, room passwords, SIP PINs or full participant lists. Logging failure cannot block chat; no default telemetry/upload. Native/performance claims need evidence.

## DESK-010

Provide complete en/de/es UI, errors, tooltips, notifications and accessibility names, with device-local System/English/Deutsch/Español and English fallback. Language switching preserves account, call, draft and focus; content and protocol IDs are not translated. Support Unicode/bidirectional text and composition. Format local dates/plurals and resolve DST ambiguity explicitly; shortcuts follow
the OS.

## DESK-011

Use this project's GitHub Releases and chosen stable/prerelease channel for bounded, verified updates. Persist a randomized daily check schedule and allow explicit checks. Offer Update only for a matching verified installable package; hide/block it during ringing/joining/active/ending calls and recheck atomically before install. Drain work, revalidate staged bytes and preserve a usable
installation on failure. macOS packages include the native runtime and notices; signing/notarization/platform support retain their actual limits. Never execute chat/server-supplied updater URLs or end calls to update.
