# Shell navigation and status

The application shell has three jobs: get the user to a conversation quickly, say what
needs attention, and report problems without raising its voice. This note records the
structure and the reasons. [Tokens](tokens.md) holds the measurable values.

## Sidebar

```
Suchen                      ⌘K        search and quick switcher
Aktivität                    4        the only top-level destination
Gespräche                  +  ⋯       list title, two menus
  Favoriten
  Direkte Gespräche
  Kanäle  …                           collapsible groups, unread kept visible
Account                      ⚙        compact footer
```

The sidebar used to carry eight navigation rows, a three-way tab filter and the room list.
Most rows were rarely used and all of them looked equally important. The new order
follows how often each thing is needed:

| Need | Where it lives | Why there |
| --- | --- | --- |
| Find or switch | Search field, `⌘K` / `Strg K` | Constant, keyboard-first, unchanged. |
| What needs me | **Aktivität**, one row with a count | Unread, mentions, threads and reminders answer one question. They share a page with a small filter control instead of four sidebar rows. |
| Talk to someone | The room list | The list is the main content. Groups collapse, and a collapsed group still shows its unread count. |
| Start something | **+** on the list title: "Neues Gespräch", "Offene Gespräche durchsuchen" | Creation and discovery are one verb. They appear only where a list grows. |
| Shape the list | **⋯** on the list title: "Alle Gespräche", "Nur Ungelesene", "Nur Erwähnungen", "Archiv" | Rare, reversible choices. A non-default filter stays visible as one removable chip under the title. |
| Federated invitations | A notice at the top of Aktivität, with a count in the Aktivität badge | They are pending business, not navigation. |

Aktivität counts rooms that ask for the user: direct and personal conversations with unread
messages, and direct mentions. This is the same rule that puts a number on a room row.
Muted and archived rooms stay quiet. The room list keeps its place when a room is read;
rows do not jump into an "Ungelesen" section, because a list that rearranges itself while
the user reads it is harder to scan than one with bold titles and badges.

The archive replaces the list in place and offers one way back from its title. The
conversation list never shows archived rooms outside it.

Aktivität filters, in order: Ungelesen, Erwähnungen, Threads, Erinnerungen. Threads and
Erinnerungen are absent when the server lacks the feature. Erwähnungen is disabled, with a
reason, until a mention state is known. The Activity page reuses the room
row (with an avatar tile and a last-message preview) and the existing thread and reminder
bodies; the former Reminders dialog is gone. Entering Erinnerungen refreshes the
reminders as the dialog did.

## Title bar status

Connection state lives in the application title bar, next to "NextCloud Talk":

- **Healthy:** nothing is drawn.
- **Reconnecting** ("Verbinde …"): the server is slow or briefly unreachable and retries
  run. A pill with an amber dot and one word.
- **Offline** ("Offline", with "· N" when changes are queued): the server is unreachable
  or accepted changes wait for it.

The pill appears only after the existing five-second grace period, so brief blips never
flash. A click opens a popover with the reason in one sentence, the number of queued
changes, how long the connection has been down, and "Jetzt erneut verbinden". It closes
on an outside click or on Escape, which returns focus to the pill. The popover never claims a
last successful sync: the presentation contract carries no such time. The duration is
the time since the shell observed the failure.

Failures that are not connectivity (access denied, rate limited, unexpected response)
are not shown in the title bar. They appear as an inline notice where they block work.

## Status and error system

One visual language covers every state that is not the normal flow. Messages that fail
to send use the same notice and action styles.

**Scope decides where it appears.** The smallest scope that is still honest wins.

| Scope | Where | Used for |
| --- | --- | --- |
| App | Title-bar pill and popover | Connectivity: reconnecting, offline, queue |
| View | A centered state in the empty content area | The whole room cannot show content: access withdrawn, never loaded, not yet available |
| Region | Inline notice at the top of the affected region | The sidebar list failed to load, the account denies access, a section is partial |
| Composer | A slim bar replacing the composer | Read-only room ("Nur Lesezugriff") |
| Item | An icon on the row with a tooltip | One room or message action failed |
| Transient | A dismissible notice | The result of an action the user just took |

**Roles carry meaning twice, by icon and words, never by color alone.**
Information, warning ("waiting") and error share one calm shape: a faint tinted wash with
no outline, a 16 pt role icon in the role color, a short neutral title and one muted
sentence that says what to do. Text keeps the neutral primary and secondary colors so
contrast does not depend on the tint. An optional text-weight action sits at the end of
the line, or under the text in a narrow container such as the sidebar. At most one action
is primary.

**Copy.** A title says what happened in a few words. The detail says what the user can do
or what stays safe ("Dein Entwurf bleibt erhalten."). No error codes, no blame, no
exclamation marks. Every string exists in German, English and Spanish.

**Confirmed content stays.** A refresh or a temporary failure never replaces a timeline or
a list that was already confirmed. A centered state appears only when there is nothing to
show. Retained content never grants authority: sending and mutating stay disabled while
access is unconfirmed.

### States in the room

| State | Presentation |
| --- | --- |
| Access withdrawn or link invalid | Centered lock state "Kein Zugriff auf dieses Gespräch." with "Erneut prüfen". The composer disappears; a retained draft stays stored. |
| Read-only | Timeline stays; a slim bar "Nur Lesezugriff · Du kannst hier nicht schreiben." mentions the kept draft only when one exists. |
| Never loaded, offline | Centered "Noch keine Verbindung", no action; the title bar explains the connection. |
| Never loaded, failed | Centered "Nachrichten konnten nicht geladen werden." with "Erneut versuchen". |
| Server denies or limits access | Inline notice under the header, role and icon chosen by cause (lock, clock, cloud), with "Erneut versuchen". |
| List failed to load | Compact notice in the sidebar with "Erneut laden" under its text. |

## Evidence scenarios

`sidebar-rich`, `sidebar-activity`, `sidebar-activity-mentions`, `sidebar-new-menu`,
`sidebar-list-menu`, `sidebar-archive`, `sidebar-load-failed`, `connection-reconnecting`,
`offline-with-queue`, `connection-details`, `chat-access-denied` and `chat-read-only` are
fictional and contact no server. Capture them with
`TALK_UI_MATERIAL_PREVIEW=1 python3 tools/ui/capture.py` in light and dark, blue and
sage. See [development](../development.md#native-ui-entrypoint).
