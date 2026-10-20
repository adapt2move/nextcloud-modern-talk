# Messages, threads, and retrieval

Status: **Planned**. Sources: [chat, reaction, thread, scheduling, and capability contracts](../compatibility/baselines-and-sources.md#protocol-sources). All writes require current membership, relevant permissions, and supported room/host behavior.

## MSG-001

**Read and paginate history (P1).** Render text, rich objects, system events, deleted/edited markers, timestamps, and reply context with stable message identities. Preserve scroll position when older messages arrive. Separate first load, older-history load, empty history, partial history, and failed pagination. Do not execute HTML or scripts from any message or object.

Acceptance: duplicate/out-of-order delivery produces one canonical message per ID without jumping the reader to the bottom. Unknown system events/rich objects show an intelligible fallback. A deleted/expired parent is identified as unavailable without exposing its cached text. The room-list preview is not assumed to contain complete reaction or parent data.

## MSG-002

**Compose and format (P1).** Provide multiline text, emoji, supported Markdown, paste, and explicit send. Respect the server message-length limit before sending and preserve text on rejection. Offer a discoverable send-key preference; the default is Enter to send and Shift+Enter for a newline, with IME composition never interpreted as send. Native rendering must safely handle lists, code, quotes, links, and supported emphasis; unsupported syntax stays readable.

Acceptance: formatting preview and sent text preserve meaning, Unicode, and line breaks. Links display their destination and open only on deliberate activation under the security policy. Copying code preserves its contents. A permission change disables send while retaining the draft; a read-only timeline does not show a writable composer.

## MSG-003

**Make delivery states truthful (P1).** A pending message progresses through preparing/sending, accepted by server, failed, or delivery-unknown. Server acceptance is not proof of a recipient read. Use a client reference to reconcile optimistic messages with received messages, without assuming the server deduplicates requests. Do not retry ambiguous sends automatically unless the endpoint's tested semantics make it safe.

Acceptance: disconnect after submission shows “Zustellung unklar”, refreshes/reconciles, and offers an explicit retry with duplicate risk explained if uncertainty remains. Confirmed pre-submission failure allows a safe retry. Repeated clicks do not send parallel copies. Reauthentication, room removal, or expired permissions never release queued work to the wrong destination.

## MSG-004

**Draft locally and handle offline composition (P1).** Keep a draft per account, conversation, and thread/reply context. Restore text, attachments still available, and recipient context when switching views. Persist only under the approved content-storage policy; visibly identify session-only drafts. Offline composition is allowed, but pending text is not silently scheduled to transmit on reconnect. Before sending, revalidate membership and message targets.

Acceptance: restart recovers a protected persisted draft only for the same account; session-only mode does not claim recovery. A deleted thread/root or revoked room yields a recoverable draft with explicit destination selection, never a silent post to the main timeline. Logout and cache-clear explain draft loss.

## MSG-005

**Reply and privately reply (P1 reply; P3 private reply).** Quote only supported replyable messages, preserve original references, and let the reader navigate to context. Private reply creates/opens the permitted direct conversation and clearly changes the audience before send; it must not leak a classified/restricted message or imply that the whole source room receives it.

Acceptance: `isReplyable=false`, deleted parents, unavailable context, and permission removal disable the relevant action with a reason. Private reply requires `private-reply` and source policy; its composer names the private recipient and source context before submission.

## MSG-006

**Use real threads (P3; safe reading/fallback P1).** When supported, create named threads, rename when authorized, reply within them, list recent/subscribed activity, change supported thread notification/subscription levels, and navigate between root context and thread. Thread drafts and unread counts are distinct from the main composer. Do not simulate a server thread using a local-only grouping while claiming parity.

Acceptance: thread send includes the correct server relationship and appears in the same thread on another supported client. A missing `threads` capability hides thread creation; received unfamiliar thread data remains readable without flattening a reply into an unrelated send. Removed roots and inaccessible threads produce a clear unavailable state.

## MSG-007

**React without conflating reaction and chat permission (P1).** Add/remove one's own supported reaction and inspect permitted reaction authors. Optimistic changes reconcile with server counts and the current user's state. Separate call reactions from message reactions.

Acceptance: chat denied plus reaction allowed still permits reactions; the reverse does not. A revoked `react-permission`, deleted message, or failed request rolls back the pending reaction. Multiple clients and repeated clicks converge without inflated counts.

## MSG-008

**Edit and delete within actual policy (P1).** Offer edits and deletions according to message type, actor, moderator rights, capability, and current server limits. Show an edit mode and retained draft on failure. Distinguish delete message, delete rich-object share, and delete underlying file; never imply that deleting a chat share deletes the file everywhere. Honor note-to-self editing extensions only when supported.

Acceptance: expired edit/delete windows or a concurrent deletion produce explicit failure and a refreshed message. A successful edit shows server edit metadata; a successful delete updates timeline, previews, local search, pins, and cached excerpts. Do not hard-code an unlimited deletion/edit window from the version number.

## MSG-009

**Mention, send silently, and show typing with privacy (P1 mentions; P3 silent send).** Autocomplete actual users and allowed room-wide mentions. Respect `mention-permissions`, `silent-send`, and typing privacy. Show the silent-send choice before submission; it is not the same as muting one's own notifications. Format federated/guest mention identifiers according to the server contract.

Acceptance: forbidden `@all` cannot be sent through a decorative mention control. Silent sends reconcile with their returned state; sharing media uses its own metadata/gates, not an assumed identical endpoint. Private typing preference emits no typing events, and typing expires on idle/disconnect without a permanent “schreibt …” label.

## MSG-010

**Read markers, unread state, and receipts (P1).** Mark read when the relevant conversation is actually presented and its content is visible under the client's reading policy; background prefetch, notifications, and search must not clear unread state. Distinguish unread counts, mentions, explicit “mark unread”, and optional common-read receipts. Respect read privacy and the server's session activity model.

Acceptance: a hidden/minimized conversation does not become read from a background fetch. Turning read receipts off suppresses receipt display/transmission as required by the server contract. A common-read marker is never labeled as an exact individual reader list. Reconnect reconciles markers without resetting the user to a guessed latest message.

## MSG-011

**Search with scope and context (P1).** Provide conversation and account-wide message search through supported APIs, showing scope, query, progress, result snippets, and navigation to message context. Keep server search separate from an explicitly labeled partial local-cache search. Do not claim complete offline search. Where supported, add sender/date/type filters only with defined server semantics.

Acceptance: no matches, forbidden search, remote host failure, and incomplete offline results are different states. A result deleted or expired before opening reports unavailable; it does not reintroduce cached content. Search cannot reveal another account or restricted room through snippets or history.

## MSG-012

**Pin messages (P3).** List pins, navigate to their context, and pin/unpin only when permitted and `pinned-messages` is advertised. Support a pin expiry and personal hide action when the applicable contract permits them; hiding a pin for oneself does not unpin it for everyone. Pins refer to canonical messages and do not override retention.

Acceptance: a pin creation failure does not leave a local pin presented as shared. A pinned message later deleted/expired becomes unavailable and is reconciled. A participant losing pin privileges retains only permitted read behavior.

## MSG-013

**Remind later (P3).** Set/change/cancel a server reminder for an accessible message, with timezone-aware date/time selection and a visible upcoming-reminders list where available. A reminder is personal, not a scheduled message to other participants. Use server-confirmed timestamps and status.

An authenticated personal message-reminder notification uses the local important-notices policy, with its own typed reason and account/conversation target. Resolve it from supported server identifiers, never translated subjects or a local timer inferred from a message timestamp. Disabling local delivery does not cancel the server reminder or mark its message read.

Acceptance: ambiguous timezone/DST selection is resolved visibly; a failed creation does not promise a reminder. An inaccessible target opens a neutral unavailable state without its protected excerpt. Client quit does not imply a confirmed server reminder was canceled.

## MSG-014

**Schedule messages on the server (P3).** With `scheduled-messages`, select date/time, inspect pending schedules, and edit/cancel according to actual API permissions. Clearly label scheduled versus sent. Do not replace server scheduling with an undocumented local timer that requires the client to remain running.

Acceptance: quitting after server confirmation preserves the schedule; a request timeout is reconciled before another schedule is created. A rejected schedule retains text. A server send failure, lost membership, or cancellation race appears as returned status without falsely claiming delivery or cancellation.

## MSG-015

**Translate and summarize optionally (P4).** Offer message translation and unread-chat summaries only with supported APIs/providers and room policy. Show destination/provider information where available and require deliberate activation; never send private content to a third-party model chosen by the client. Keep original text available and label generated output. Summarizing does not mark unseen messages read automatically.

Acceptance: provider disabled/unavailable, unsupported language, task timeout, insufficient permissions, and classified/sensitive restrictions produce clear states. Generated text is not treated as authoritative conversation history; links to source context remain available when allowed.

## MSG-016

**Copy and forward deliberately (P3).** Permit copying text/links or forwarding selected content only when its room/security policy permits exposure and a supported destination send/share contract exists. A destination picker names the account and conversation and previews the outgoing content before send. Forwarded content must not impersonate the original author; distinguish a new share/quote from the original message.

Acceptance: cross-account forwarding is never an automatic side effect of switching accounts. An inaccessible or classified source cannot be exported through a forwarding shortcut. Missing attachment rights or an unavailable target preserves the user's choice with a clear failure. A failed forward does not remove or mark the source as sent to the destination. Clipboard use is explicit and subject to the security contract.
