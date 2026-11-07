# Screen and continuity qualification

Run these tools only on an owned disposable agent-dev-box VM. They are opt-in
interop drivers, not production authentication or a substitute for native desktop
qualification. `normal_talk.rs` uses the normal account-owned call actor and media
engine. Its `native-screen` case supplies generated 720p RGBA; `portal-screen`
includes the desktop's actual Linux capture owner. `web-screen` operates the
unmodified official Talk client's sharing controls. Reports distinguish these
sources and contain counters and state only, without credentials, SDP or ICE
addresses. Keep reports outside the repository.

Prepare the pinned r35 lab with `tests/lab/dev.py up` as described in
[the lab guide](../../tests/lab/README.md). Build the driver on the VM:

```sh
python3 tools/cargo.py build --locked -p talk-adapters --features backend-test --example media-normal-talk
```

Use the returned private manifest and a new output filename:

```sh
python3 tools/media/normal_talk.py --manifest /run/LAB/manifest.json \
  --executable target/debug/examples/media-normal-talk \
  --scenario native-screen --participants 3 --output /tmp/native-screen.json
```

The participant count is two or three. Cases include `media`, `native-screen`,
`native-static-screen`, `portal-screen`, `web-screen`, explicit `reconnect`,
`network-resume`, `web-screen-resume` and `session-replaced` (replacement
requires HPB). Screen sending passes only with a distinct browser-decoded
1280×720 stream, not camera traffic. Generated media does not qualify physical devices.

The official web client requires HTTPS for screen capture. On the disposable VM,
install `libnss3-tools`, then run `tls_lab.py --state /run/LAB/state.json`. This
creates a one-day CA/key under the private lab runtime and trusts its public
certificate in the VM, the lab container and both Chromium NSS store locations.
It updates the media manifest to HTTPS. No TLS verifier is disabled. Destroy the
VM after qualification; do not copy its CA key or credentials into evidence.

For HPB, build Janus 1.3.0 into `/opt/call-media-janus` with WebSocket,
VideoRoom and SCTP support, and install coturn. `hpb_lab.py` names the exact
signaling 2.1.1 image digest. Start it as an owned detached VM job:

```sh
agent-dev-box exec --detach -- python3 tools/media/hpb_lab.py \
  --state /run/LAB/state.json --serve
```

The tool configures the existing lab over private stdin, starts bounded local
Janus/TURN services and an HPB container, and keeps their owner alive. Use its
reported private gateway as `normal_talk.py --hpb-target GATEWAY`. The harness
provides bounded HTTP/TLS forwarding while it runs. `network-resume` interrupts
that WebSocket endpoint (or the internal-signaling HTTP endpoint) for 1.5 seconds and requires observed reconnecting,
retained mute intent and decoded media after recovery. `session-replaced` adds
`--lab-state /run/LAB/state.json` and replaces the owned signaling process; it
requires a new call generation, retired old capture and fresh media.
Fault injection waits for native decoded AV and browser AV in both directions;
the native driver exposes bounded metadata progress in its private result file.
Mute and outage times are relative to that established media state. A startup
negotiation failure cannot be counted as a recovered call.
Internal outage probes allow a 50-second exchange window because the severed
server-side poll can remain active for its documented 30 seconds. The remote
screen stays shared until 44 seconds after media readiness in that probe, so recovery must observe
fresh screen frames before the explicit stop. REST success alone never stands
in for a resumed signaling read; the recorded phase must become active again.
Do not run two browser drivers concurrently on the same endpoints.

`trace_signaling.py` accepts the same arguments and observes bounded HPB message
metadata while forwarding unchanged bytes. It records hashed routing identities,
message kinds and error codes, never hello credentials, SDP or candidates.
Full HPB replacement with an existing Talk 25 web peer has a known automatic
recovery failure, documented in [known limitations](../../docs/limitations.md#servers-and-calls).
Run `session-replaced` without extra flags to retain that result. A separate
`--rejoin-web-peer-after-replacement` fixture explicitly operates the web peer's
leave/join controls after native fresh admission; its result must be reported
separately from automatic recovery.

For the Linux portal probe, use an unprivileged disposable user, its own private
manifest/credentials/TLS files, and an executable that user can read and run.
Install Sway, xdg-desktop-portal-wlr, PipeWire, WirePlumber and
`gstreamer1.0-pipewire`. Run `portal_session.sh` inside that user's
`dbus-run-session`, with a private `XDG_RUNTIME_DIR`, wrapping the browser driver
and its `portal-screen` case. The script creates a real headless Sway output and
lets the portal backend select that sole output; this is not evidence for a
physical compositor's interactive picker. Export that user's HOME/XDG data
paths before starting the bus. Portal/backend logs remain private.
`NMT_PORTAL_WLR` selects an explicit fixture backend binary. The SHM-only pixman
probe used upstream xdg-desktop-portal-wlr 0.7.1 with its initial format check
changed to fail only when neither SHM nor DMA-BUF is available. Its original
check incorrectly requires DMA-BUF even on a SHM-only output. This fixture patch
is not a shipping dependency change or qualification of an unmodified desktop.
The distro 0.8.1 and upstream 0.8.4 backend instead hit
[the double-frame protocol error](https://github.com/emersion/xdg-desktop-portal-wlr/issues/404).
Preserve the exact backend source/archive/patch hashes in external evidence.

Stop the adjunct with `hpb_lab.py --state /run/LAB/state.json --stop`, cancel its
owned VM job, then stop the base lab with `tests/lab/dev.py stop --state ...`.
Copy only reviewed redacted reports and check logs before destroying the VM.

To qualify the server-delivered ICE settings, build `media-native-peer`, then run
`ice_settings.py --manifest /run/LAB/manifest.json --executable
 target/debug/examples/media-native-peer --expect srflx --output /tmp/stun.json`
and repeat with `--expect relay`. Use the same Playwright Python environment as
the browser driver. The adjunct's bounded Binding-only UDP proxy supplies a
separate mapped port for the same-host STUN probe; coturn handles actual UDP
TURN allocations. Relay success requires selected relay pairs and decoded audio
and video at both ends, not just an allocation.
