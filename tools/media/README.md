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

Use `--scenario media --participants 3 --join-third-after-media` to join a third
browser participant after five seconds of established bidirectional media. The
native driver observes another 50 seconds after initial media readiness, including
time for an outstanding internal-signaling poll. A pass requires the same call
generation, continuing audio/video, two flowing remote cameras and media decoded
by the third browser. Add `--hpb-target` for the owned HPB adjunct. Generated media
does not reproduce every real participant's codec or network.

The `camera-only` scenario keeps the native microphone disabled, requires fresh
browser-decoded camera video plus received browser audio/video, and asserts that
no native audio capture was submitted. It isolates camera publication from a
missing or unavailable microphone; it does not emulate a physical headset.

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

When extending a driver into a sustained native-UI journey, keep X11 commands,
OCR and other blocking work off the asyncio thread that owns its forwards
(for example, use `asyncio.to_thread`). A busy observer on that thread also
stalls the client's network and can manufacture a recovery failure. Record
event-loop lag with the media counters. `LoopbackForward` closes connections
after 180 seconds, even with traffic, so a longer run through it exercises
repeated transport recovery. Browser instrumentation also admits only eight
peer connections per page lifetime; use fresh contexts between independent
calls and report a diagnostic-bound failure separately from a client failure.

`trace_signaling.py` accepts the same arguments and observes bounded HPB message
metadata while forwarding unchanged bytes. It records hashed routing identities,
message kinds and error codes, plus HTTP call/room mutation methods and relative
times. It never records HTTP paths, headers, bodies, hello credentials, SDP or
candidates. The HTTP observer skips bodies, stops observing unsupported framing
and shares the finite transcript capacity; it does not alter forwarded traffic.
Full HPB replacement with an existing Talk 25 web peer has a known automatic
recovery failure, documented in [known limitations](../../docs/limitations.md#servers-and-calls).
Run `session-replaced` without extra flags to retain that result. A separate
`--rejoin-web-peer-after-replacement` fixture explicitly operates the web peer's
leave/join controls after native fresh admission; its result must be reported
separately from automatic recovery.

For diagnosis only, `--refresh-web-peer-flags-after-replacement` reasserts the
owned synthetic web participant's AV call flags after native fresh admission,
without leaving or rejoining the web call. It is mutually exclusive with peer
rejoin and never counts as automatic native recovery. The report records the
intervention's HTTP status and sets `automatic_recovery_fixture` to false.
Use it to distinguish HPB's in-call subscription gate from a publisher or SDP
failure; the web client's obsolete peer routes may remain even after this gate
is restored. This option changes the peer fixture, not production client behavior.

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

## Connected GStreamer migration gates

The shipping engine has not switched to `webrtcbin`. The exact
`gstreamer-webrtc` 0.25.4 dependency is test-only. On an owned Linux media VM,
install the base/good/bad plugins, `gstreamer1.0-nice`, and the GStreamer WebRTC
development library (`libgstreamer-plugins-bad1.0-dev`). In addition to the
normal production-path media tests, run each migration gate separately:

```sh
python3 tools/cargo.py test --locked -p talk-adapters --lib \
  webrtcbin_sdp_ice_and_connected_codecs_exchange_fresh_av -- --ignored
python3 tools/cargo.py test --locked -p talk-adapters --lib \
  talk_e2ee_vp8_survives_gstreamer_payloader -- --ignored
python3 tools/cargo.py test --locked -p talk-adapters --lib \
  talk_e2ee_vp8_survives_alternative_rust_payloader -- --ignored
python3 tools/cargo.py test --locked -p talk-adapters --lib \
  webrtcbin_ice_restart_rotates_credentials -- --ignored
```

The first connects two native `webrtcbin` pipelines through real SDP/ICE and
requires continuing bidirectional decoded audio/video after an initial sample
window. It uses generated capture and does not qualify Talk/Janus, data channels,
TURN, publication gates or native devices for a replacement engine.

The second is a deliberately unmet acceptance gate, not an expected-failure
regression: it must pass if the replacement uses `rtpvp8pay`; a compatible
alternative may be qualified instead. It encodes
real VP8, seals it with the production Talk cipher, verifies decryption, and
feeds both plain and sealed frames through `rtpvp8pay`/`rtpvp8depay`. The sealed
frame is rejected by the current native payloader. GStreamer 1.28.4's
[`gst_rtp_vp8_pay_parse_frame`](https://github.com/GStreamer/gstreamer/blob/1.28.4/subprojects/gst-plugins-good/gst/rtp/gstrtpvp8pay.c#L272)
reads partition data beyond Talk's clear 10/3-byte prefix. Encrypting RTP packets
instead would not preserve Talk's whole-frame cipher format. A native-compatible
opaque-frame packetizer integration is still required. Do not remove the current
packetizer or fall back to unencrypted media to make this gate pass. The separate
alternative probe registers `gst-plugin-rtp` 0.15.4 statically and requires
`rtpvp8pay2` to preserve keyframes and delta frames byte for byte; it does not
validate the rest of a new production transport or its packaged native runtime.
That alternative passes the isolated sealed keyframe/delta-frame probe on the
exercised Linux runtime; the classic payloader does not.

The ICE-restart gate negotiates two descriptions and requests a new offer with
`ice-restart=true`. It fails if either ICE credential is unchanged, without
printing those secrets. The exercised native path reuses both credentials.
Replacing the peer instead is not equivalent to the existing in-place restart:
it needs explicit signaling, generation and recovery behavior plus interop tests.

The ordinary `opus_packet_duration_changes_keep_audio_and_video_alive` regression
uses the production transport, cipher, decoder, `audiobuffersplit` and audio
handoff. It changes real Opus packet durations through 10/20/40/60 ms during a
live video stream, with and without E2EE, and checks fresh decoded media in
every phase. The native appsink drops oldest blocks beyond its twelve-block cap;
packet duration does not enlarge an application PCM queue.

## TLS TURN and cancellation

On an owned disposable Linux VM with coturn, OpenSSL, iproute2 and util-linux,
run `python3 tools/media/turn_tls.py` as root after preparing the native build
prerequisites. The runner builds outside a private network namespace, then uses
a veth pair and a real TLS-only coturn listener inside it. It changes no host
firewall or trust settings, accepts no production credentials, and removes its
throwaway files and server. Output records fixed outcomes only.

Success requires a selected relay pair and continuing decoded audio/video in
both directions, with and without Talk E2EE. Initial decoded frames do not
qualify the final observation window. Certificate rejection and connection
lifetime tests run separately from the positive media case. These are generated
media and local-network checks; public TURN deployments and macOS/Windows are
not thereby qualified.

The local UDP hop from an interface-bound WebRTC socket to the loopback bridge
must be permitted by the host. For example, Kubernetes localnet firewall rules
can prevent relay allocation before TLS is even attempted. The fixture isolates
that hop rather than changing firewall policy. A verified TLS endpoint alone
must never be reported as relayed media.
