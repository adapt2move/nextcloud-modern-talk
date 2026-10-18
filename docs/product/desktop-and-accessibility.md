# Desktop behavior, accessibility, and resilience

Status: **Planned**. macOS is primary. Other OS adapters are architectural targets, not supported downloads. Server notification/presence/settings facts are referenced in [sources S03, S05, S08, S15](../compatibility/baselines-and-sources.md#protocol-sources).

## DESK-001

**Native desktop shell (P1).** Provide account switcher, conversation sidebar, message/content area, contextual details, settings, and discoverable call controls. Preserve navigation context and drafts when resizing or opening a thread. Narrow windows must still provide navigation and an accessible leave-call action. A future compact/floating call window (P4) must retain account identity, mute/leave/share-stop controls, focus behavior, and the same media session; it is not a claim of universal OS picture-in-picture support. macOS window/menu conventions and system-browser handoffs are native integrations, not HTML surfaces.

Acceptance: the application can complete login, find a conversation, send a permitted message, and reach settings without a WebView. Mock/demo data is visibly marked and cannot be mistaken for a live account. Layouts do not hide actionable error states behind decorative overlays.

## DESK-002

**Notifications and privacy (P1 messages; P2 calls).** Request OS notification permission in context. Combine room notification levels, call-notification preferences, server notification state, OS Focus/Do Not Disturb, and local sound choices without claiming control over OS delivery. Default lock-screen content is privacy-preserving; user-enabled previews remain subordinate to sensitive/classified rules. Prevent duplicate banners/sounds from multiple transport paths.

Acceptance: a denied OS notification permission shows guidance while in-app unread/ringing state still works. Notification activation opens the correct account/room/message; deleted or inaccessible targets show unavailable. No background fetch marks content read. A stale call notification is revalidated before acceptance; foreground/background acceptance shares [CALL-001–003](calls.md#call-001). If notifications cannot arrive while the app is quit, say so explicitly; do not promise background delivery until an actual tested mechanism exists.

## DESK-003

**Presence and status (P1 read; P3 editing).** Display supported server user status, custom status text/emoji/expiry, profile/avatar, and call participation separately. Allow supported status/profile/avatar settings only through their core or advertised temporary-avatar APIs and current limits, without overriding server/OS intent silently. Keep read receipts, typing privacy, notification mute, and presence as distinct preferences. Honor session inactive state when content is not actively being read.

Acceptance: unavailable presence is unknown rather than offline. Going offline does not claim the user's status changed server-side. Reconnection re-fetches status; expiry follows server-confirmed time. Choosing Do Not Disturb does not claim to mute every other client automatically.

## DESK-004

**Preferences with clear ownership (P1; advanced P3/P4).** Distinguish device-local settings from server/account settings. Cover theme/system appearance, text scaling, list density, language, send key, sounds, message/call notifications, privacy, attachment destination, cache controls, media defaults/devices, supported backgrounds, and call layout limits. A setting has saving/saved/failed states; unsupported controls are omitted or explained.

Acceptance: changing an account preference does not alter another account. Server-enforced values remain visible and locked. A failed write restores confirmed state and offers retry. Camera/microphone default preferences never authorize capture while ringing. Removing an account does not erase intentionally saved downloads.

## DESK-005

**Keyboard and assistive technology (P1 onward).** All shipping actions are keyboard reachable with visible focus and meaningful accessible names, roles, values, and status announcements. Native GPUI integration must expose an accessibility tree verified with macOS VoiceOver. Focus enters dialogs predictably, remains within modal interactions, and returns to the invoking control. Announce new content without continuously interrupting reading.

Acceptance: VoiceOver and keyboard-only users can add an account, change conversations, compose/edit/reply, use menus, handle permissions/errors, accept/decline a direct call, join/leave a conference, choose devices, and stop screen sharing. Icon-only controls have names; keyboard shortcuts never fire while composing IME text. Escape/cancel behavior cannot accidentally end a call or discard a draft without its defined action. A screenshot of an accessible-looking interface is not evidence of accessibility.

## DESK-006

**Readable visual and motion behavior (P1).** Support system light/dark appearance, increased contrast, reduced motion, and text scaling. Do not convey unread, muted, recording, permission-denied, or connection state by color alone. Keep critical controls visible at larger text sizes and small windows. Use contrast and target-size criteria from the quality plan, measured on actual native rendering.

Acceptance: reduced motion removes nonessential animation without hiding progress. At 200% text size, core workflows remain available without text covering their controls. Selection/focus and disabled/error states stay distinguishable in both palettes. Animated images/reactions do not force autoplay when reduced motion or user preference disables it.

## DESK-007

**Offline and reconnect (P1 messaging; P2 media).** Show account-specific offline/stale state and last successful synchronization where useful. Permit reading policy-approved cached content and composing drafts. Do not silently queue permission-changing/destructive actions or automatically send newly composed offline text. On reconnect, refresh credentials/session, capabilities, room membership/policies, and content before making actions available.

Acceptance: one offline account does not block another. Duplicate events reconcile; missing events trigger bounded full refresh. A room removed while offline cannot accept a queued send. Server `modifiedSince` is not treated as a deletion feed: periodic/full reconciliation handles deletion/disinvite. Call recovery follows [CALL-014](calls.md#call-014), with no automatic unmute or resurrection of ended calls.

## DESK-008

**Sleep, app lifecycle, and conflicts (P1/P2).** Closing a window, quitting, logging out, system sleep, and crashing have distinct effects. Explain active calls/transfers/drafts before explicit quit where interruption matters. Do not claim that a closed window disconnected a background account. A crash restart restores only permitted persisted state and never resumes capture automatically.

Acceptance: wake revalidates server session and OS devices; stale notifications and call attempts cannot reconnect blindly. One session becoming obsolete does not delete another device's account. Quit ends local capture and transient session credentials; server-scheduled messages/reminders remain server-owned.

## DESK-009

**Actionable errors and redacted diagnostics (P1).** Use concise user language with retry/change-settings/relogin/leave where appropriate. Provide optional diagnostic details containing client version, supported server versions, resolved feature states, and redacted error categories. No primary passwords, app passwords, cookies, poll tokens, private URLs, room passwords, SIP PINs, message text, or full participant lists belong in default diagnostic export. Follow [security.md](../quality/security.md).

Acceptance: user-visible errors do not contain raw backend dumps. Export is deliberate and reviewable; redaction is tested with synthetic secrets. No telemetry/analytics or diagnostic upload occurs by default. Native integration, performance, and reliability statements cite measured evidence, not design goals.

## DESK-010

**Locale, time, and platform conventions (P1).** Support Unicode/bidirectional text, localized dates/times/plurals, and German/English UI resources. Use stable server IDs independent of translation. Present schedules, reminders, meeting times, and timestamps with the relevant timezone and explicit handling of ambiguous local times. Use platform shortcut names, including Command on macOS.

Acceptance: different UI languages do not change protocol payload identifiers. A right-to-left message does not spoof the surrounding actor/account identity. Date-only, relative-time, and DST transitions are tested with deterministic clocks. No hard-coded mock identity, company, or private server appears in shipped UI.

## DESK-011

**Application updates and OS scope (P4 distribution work; honest status P1).** Report the actual client version and a trusted project release location. A future updater requires the distribution/security contract, integrity verification, and rollback/failure behavior before activation. Additional OS clients must verify native secret storage, capture, notification, and accessibility integrations separately.

Acceptance: source availability or successful cross-compilation is not labeled platform support. No arbitrary update executable is downloaded/run from a server-supplied message. A server “upgrade required” response explains the need without bypassing it or treating it as an authentication failure.
