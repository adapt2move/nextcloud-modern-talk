# Install and update

Download the package and matching checksum file from the
[GitHub releases](https://github.com/adapt2move/nextcloud-modern-talk/releases).
Alpha versions are marked as prereleases. Keep the included provenance and
third-party notices when redistributing a package.

## macOS

The app requires **Apple Silicon and macOS 15.7 or later**.

1. Open the downloaded DMG.
2. Drag **NextCloud Modern Talk** onto **Applications**. This link points to the
   system Applications folder, called **Programme** on a German Mac.
3. Eject the disk image.
4. Open NextCloud Modern Talk from Applications.

The media runtime is bundled. No Homebrew packages, GStreamer framework or other
runtime installation is needed.

### First launch blocked by macOS

Releases through v0.0.10 and non-publishing test packages have ad-hoc signatures.
Published updates require a persistent internal alpha or Developer ID Application
signature. Internal alpha signing supplies no Apple publisher trust; notarization
is not implemented. If macOS says the developer cannot be verified or Apple cannot
check the app, first try to open the installed app, then:

1. Open **System Settings → Privacy & Security**.
2. Scroll to Security and choose **Open Anyway** for NextCloud Modern Talk.
3. Confirm **Open** in the next dialog.

On a German Mac, the path is **Systemeinstellungen → Datenschutz & Sicherheit →
Dennoch öffnen**, followed by **Öffnen**. This grants an exception for this app;
do not turn off Gatekeeper or remove system-wide security checks. Follow
[Apple's instructions](https://support.apple.com/102445) only for a download you
trust. A message that the app is damaged or will harm the computer is a different
failure; report its exact wording rather than bypassing it.

If these options are unavailable on a managed Mac, contact your administrator.

### Sign-in and permissions

Sign-in opens your system browser. New accounts are saved automatically in the
system credential store. If that fails, the app explains the problem and offers
retry or an explicitly selected, unencrypted private file.

Setup checks microphone, camera and screen-sharing access in turn and starts a
supported system permission request for the current step. Granting access moves
to the next step. You can skip optional permissions and still use chat.

If macOS asks you to restart after allowing screen access, reopen the app. It
returns to the ready screen for the same account and checks the actual permission
states again. You can change permissions later in Settings.

### Notifications

On macOS, open **Settings → Notifications → Allow notifications** to request the
system permission. Opening that page only checks the current state. If permission
was denied, open macOS **System Settings → Notifications**, allow NextCloud Modern
Talk, then return to the settings page or choose **Check again**. Banner/Sound
choices in Talk are local preferences and cannot override a system denial or Focus.

### Closing the window and quitting

On macOS, the red close button or **Command-W** hides the window and keeps Talk
running for permitted notifications. Click the Dock icon to return to the same
conversation and draft. Choose **Talk → Quit app** or **Command-Q** to stop the
application completely. Notifications still depend on macOS permission, Focus
and the server's notification policy; they cannot arrive after explicit quit.
Native close/reopen and notification delivery still require qualification on a Mac.

### Legacy profile recovery

Version v0.0.8 associates local accounts with a mount device number that can
change after a remount. If the user and directory inode still match but that
device number differs, quit Talk completely and use the
[legacy repair tool](../tools/identity/repair_macos_profile.py) with Python 3:

```sh
python3 repair_macos_profile.py --repair
```

Without `--repair` it only checks the association. Repair takes the application's
exclusive profile lock, creates a private SQLite backup in the same app data
folder and rechecks the profile before changing only its root association.
It preserves account eligibility, pending work and credentials; it refuses a
different user, directory or profile marker. Reopen Talk after successful repair.
This is a compatibility repair for the current mount, not the persistent-volume
upgrade. The tool's guards and transactions are tested with synthetic POSIX
profiles; native macOS execution remains unqualified.

## Ubuntu

For **Ubuntu 24.04 x86_64**, install the downloaded `.deb`:

```sh
sudo apt install ./nextcloud-modern-talk-VERSION-linux-x86_64.deb
```

Replace `VERSION` with the downloaded version, then launch NextCloud Modern Talk
from the application menu. Browser login needs a desktop session with a
registered browser and portal service. The tar archive is an alternative for an
owned installation with the native libraries recorded in its provenance.

## Updates

Choose the prerelease channel in Settings to receive alpha updates. The app
checks at a randomized daily time; Settings also provides a manual check.
Downloads run in the background. Select **Update** to install and restart after
saving or discarding any message draft. The action stays hidden during calls,
and an update never ends a call implicitly.

System-installed Linux packages use PackageKit and may request administrator
approval. An unavailable installation route is reported in Settings.

Upgrade **v0.0.1 manually**: it has no in-app installer. On macOS, also upgrade
**v0.0.2 manually using a current DMG**: its updater predates the self-contained
bundle layout. macOS v0.0.3 and later support complete bundle updates.

Windows is compile-checked only; no Windows installer is supplied. See
[known limitations](limitations.md) for platform and feature coverage.

## Local diagnostics

The app keeps a small local event log to help diagnose chat and backend failures.
It records timestamps, app version, connection changes, history requests and send
outcomes. Sign-in records its start, browser result and storage outcome.
Account removal also records credential retirement, cleanup and local
record removal, including allowlisted storage-backend and failure categories.
Call gate transitions record `operation: "call_gate"` and an allowlisted
`call_gate` reason (for example `required_e2ee_unsupported` or
`start_permission_denied`). `policy_unavailable` adds `policy_evidence` with
the source or field that could not be confirmed (for example `signaling_settings`,
`effective_permissions` or `source_freshness`). Repeated identical
observations do not emit another entry. No raw capability payload is logged.
Process-local request numbers connect an operation to its result.
Message text, account and conversation names, server addresses, credentials and
raw backend responses are excluded. Nothing is uploaded automatically.

Call stops record `operation: "call_stop"`. Media failures distinguish closed,
overloaded, malformed and unsupported peers. The `detail` field names the fixed
operation that failed, such as `VP8 header`, `video decoder input` or `data channel`;
it contains no packet data or dependency error text. A media stop preserves its
cause even if a concurrent policy refresh succeeds.

Every run writes an `environment` line first (platform and detail level), records a
`panic` line with only the source file and line if the app crashes, logs call phase
changes, download results and the outcome of each desktop notification
(`submitted`, `permission_denied`, `helper_timed_out`, ...). Development builds and
every pre-1.0 release also log verbose lines (`"level":"debug"`), for example how long
each picture took to load (`asset`, with a fixed kind such as `detail_view` and
`elapsed_ms`). Set `NEXTCLOUD_MODERN_TALK_LOG=debug` to force this on or `=info` to
force it off. All of it is fixed text, never content.

| Platform | Log directory |
| --- | --- |
| macOS | `~/Library/Logs/NextCloud Modern Talk/` |
| Linux | `$XDG_STATE_HOME/nextcloud-modern-talk/logs/`, or `~/.local/state/nextcloud-modern-talk/logs/` when unset |
| Windows | `%LOCALAPPDATA%\NextCloud Modern Talk\Logs\` |

The current file is `diagnostics.jsonl`; `diagnostics.1.jsonl` and
`diagnostics.2.jsonl` contain earlier events. Each is limited to 2 MiB. Files
rotate when full or when a new app process starts. If reporting a bug, include
the approximate time and review the relevant files before attaching them. You
can delete them while the app is closed.

Logging uses a bounded background writer. If storage is unavailable or another
app process owns the log, chat continues without a new file. Diagnostic events
may be dropped under heavy load; the next successful write records the count.
