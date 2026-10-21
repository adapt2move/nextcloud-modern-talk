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

The alpha has an ad-hoc code signature, without an Apple Developer ID signature
or notarization. If macOS says the developer cannot be verified or Apple cannot
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
Process-local request numbers connect an operation to its result.
Message text, account and conversation names, server addresses, credentials and
raw backend responses are excluded. Nothing is uploaded automatically.

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
