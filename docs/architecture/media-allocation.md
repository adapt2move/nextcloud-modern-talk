# Native media allocation targets

**Future full-profile allocation targets, not an implemented allocator or measured performance.** The alpha uses a maintained in-process engine with bounded application handoffs. This older two-helper reservation model does not describe that engine’s internals and is not an alpha resource guarantee. Read with [the engine and ownership decision](README.md#native-media). The full allocation profile remains unqualified. The limits of current functional and diagnostic evidence are recorded in §G. Units below are bytes, KiB = 1,024 bytes, MiB = 1,048,576 bytes.

## A. Admitted shape and ownership

The media/platform owner owns one application-wide reservation ledger in the Rust media supervisor and per-call subledgers spanning helpers, renderer imports and pending generations. Reservation is lazy with respect to physical allocation, but mandatory before allocation/import/copy. Capacity is never expanded to hide exhaustion.

| Resource per call/prejoin slot | Finite admission maximum |
| --- | --- |
| Local media | 1 microphone, 1 camera and 1 screen; preview uses the same camera backing, not a second capture pipeline |
| Remote media | 4 cameras, 1 screen, 8 audio publishers; stricter fresh policy wins; hidden cameras unsubscribe before another subscription is admitted |
| Video format | VP8 8-bit 4:2:0; CPU/native I420 or NV12, or BGRA8 for capture/conversion/presentation; no implicit 10/12-bit, HDR, 4:2:2, 4:4:4, arbitrary texture format, mip chain or multisampling |
| Camera | Positive even coded/display dimensions ≤1280×720; baseline delivery exactly 1280×720 at 30 fps within the future frozen tolerances |
| Screen | Positive even coded/display dimensions ≤1920×1080; baseline delivery exactly 1920×1080 at 10 fps within the future frozen tolerances |
| Codec layering | 1 active encoding/rendition per source in the initial profile; no hidden additional simulcast/SVC encoder or decoder |
| Audio | Opus 48 kHz mono, 20 ms frames initially; owned PCM at ≤48 kHz, ≤2 channels, signed 16-bit or float32; charge the larger 4-byte representation |
| Control objects | ≤12 PeerConnections, ≤24 RTP transceivers, ≤2 data channels/PeerConnection; all fit the shared 8 MiB control arena; SDP ≤64 KiB, ≤64 local and 64 remote candidates/PeerConnection, each ≤2 KiB; excess is rejected before object creation |
| Concurrency | ≤2 slots application-wide, including voluntary prejoin/preview; ≤1 slot/helper per account and one publishing call/account. A third preview/join is `Busy`. Existing reservations, including obsolete generations, count. |

A texture view or metadata-only rotation can reuse backing. A rotation, crop, format conversion, render target, readback or upload that creates storage requires its own charged slot. Native inputs with larger dimensions cannot be admitted by promising to downscale after their allocation. Other configurations remain Planned/unqualified; these initial limits do not erase product features.

## B. Checked layout and per-stream backing

Define `A(x,n) = checked_mul(ceil_div_checked(x,n), n)`. Use checked unsigned arithmetic for every addition, multiplication, alignment, plane offset/span and IPC conversion; reject negative strides, zero/odd/out-of-range dimensions, overflow and unsupported plane layouts before reservation. Conversions to native signed integers must also be checked.

For the selected YUV reservation calculation, allow at most 64 pixels of border on each side: `W=w+128`, `H=h+128`. This is a **chosen admissible padding envelope**, not an assertion about all libvpx/platform allocations. Strides are at most `A(W,64)` for luma/NV12 chroma and `A(W/2,64)` for I420 chroma. Use 16 KiB rounding for each plane as a conservative chosen accounting unit; verify actual allocator/IOSurface alignment and any metadata separately. No presumed OS page-size fact is needed.

```text
I420(w,h) = A(A(w+128,64)*(h+128),16384)
           + 2*A(A((w+128)/2,64)*((h+128)/2),16384)
BGRA(w,h) = A(A(4*w,64)*h,16384)
```

For admitted even dimensions, the corresponding NV12 bound is no larger than this I420 reservation. A platform reporting larger strides, borders or allocation size does not fit merely because its visible dimensions fit. Every plane must lie within its registered allocation; overlapping/disjoint planes must match the approved format. Charge actual shared allocation size, including required padding/auxiliary storage, against the reserved slot. Opaque/tiled allocations need a qualified upper-bound query or preallocation contract; dimensions×bytes-per-pixel alone cannot qualify them.

| Maximum frame class | Derived backing bytes | Selected slot charge |
| --- | ---: | ---: |
| Camera I420, padded 1408×848 | 1,818,624 | 2 MiB |
| Screen I420, padded 2048×1208 | 3,719,168 | 4 MiB |
| Camera BGRA, visible 1280×720 | 3,686,400 | 4 MiB |
| Screen BGRA, visible 1920×1080 | 8,306,688 | 8 MiB |

Each logical video stream reserves **12 YUV slots plus 8 BGRA slots**, independently of its two-frame application handoff queue:

- 8 YUV slots collectively cover native capture/encoder/decoder/reference storage; 2 cover waiting handoff frames; 1 conversion/processing input and 1 renderer-held source. These are maxima across those uses, not eight per native component.
- 8 BGRA slots collectively cover up to 3 native capture-source buffers, 1 conversion/upload scratch, 1 current presentation, 2 GPU in-flight uses and 1 additional retained presentation/handoff backing. The application still queues at most two waiting video frames total per stream, across formats; reserving backing does not authorize extra queued work.
- Aliases of one backing share one charge. Independent native planes/copies/intermediates are charged independently. A stage may leave a reservation unused; it cannot create a private overflow pool. Every reference surface, texture allocation and old-format/old-generation backing must fit this aggregate assignment, or the profile is blocked until the contract is independently revised.

Thus **camera backing ≤12×2 + 8×4 = 56 MiB**; **screen backing ≤12×4 + 8×8 = 112 MiB**. This intentionally covers simultaneous capture, conversion and presentation copies; zero-copy is an optimization to verify, not an arithmetic assumption. Decoder references are included even if they never reach a callback. These counts do not establish the reference or pool counts of the selected WebRTC engine and codecs.

Apple documents a minimum/default [ScreenCaptureKit source queue depth of three][Apple-queue]. Use that native source allocation within the three capture slots, while retaining the two-frame application handoff bound. [ADR-002](README.md#module-boundaries) explicitly bounds the application handoff queue separately from the native source queue. The native queue is fixed at three for this proposed ScreenCaptureKit path and charged to the same stream's capture slots; no source-owned overflow pool is permitted. This clarification remains subject to independent bootstrap review and native qualification. Core Video's [allocation-threshold failure][Apple-threshold] controls allocation through a specified pixel-buffer pool, not every allocation by capture, codecs, the renderer or the OS.

## C. Compressed video, audio and private state

**Compressed video: 8 MiB per logical video stream.** Reserve eight 1 MiB storage units: two for reassembly, two for transform input/output, two for encoder/decoder working bitstreams, and two for pacing/retransmission/packet storage. Every duplicate consumes capacity; retained packet references can alias only the same registered bytes. A complete compressed frame, including encryption overhead, is ≤1 MiB. Reserve up to 64 bytes of transform expansion before accepting plaintext; the eventual qualified transform must prove its own maximum. Missing/unknown expansion blocks it. Fragmentation and packet metadata have finite capacities in the same byte arenas; incomplete frames cannot accumulate outside them. Deadlines and age limits remain separately required: a byte ceiling is not a latency guarantee.

Ingress limits must apply **before RTP assembly/native buffer growth**, not just at `VideoDecoder::Decode`. Encoder output allocation must also be reserved before the encoder creates it. If upstream cannot do so through a factory/wrapper, its affected allocator path needs a reviewed patch or a proved fixed bound. Repeated oversize traffic closes the affected stream; never reallocate to fit it.

**Audio: 4 MiB per call.** Twelve PCM lanes cover one local input, eight remote streams, one mixed output, one AEC reverse/reference lane and one conversion lane. At worst, 100 ms of float32 stereo at 48 kHz is `48,000×2×4×0.100 = 38,400 bytes`. For each lane reserve two 64 KiB blocks: one queued region and one in-use/history/work region. `12×2×64 KiB = 1.5 MiB`, within a **2 MiB PCM pool**. The extra 0.5 MiB is finite padding/working slack, not more queue duration.

Add **1 MiB encoded/packet storage** and **1 MiB additional sample/history/processing storage**, for 4 MiB total. Each initial Opus packet is ≤4 KiB and describes at most 20 ms; nine publish/receive streams × five packets × 4 KiB = 180 KiB for 100 ms per stream before copies. Additional copies/reordering/FEC use the same 1 MiB, with aggregate queued duration still ≤100 ms per stream across encoded/jitter/PCM waiting stages. Historical filter/AEC state is separately charged sample storage, not an additional playback queue. Larger engine-private audio histories are not exempt: inability to fit or bound them blocks the profile. Codec/driver buffering that cannot be observed/controlled is a qualification blocker, not assumed zero.

**Control: 8 MiB per call.** Session descriptions, ICE candidate storage, bounded callback/IPC descriptors, handles, bounded future key records and data-channel queues share this arena. The object/count bounds in §A supplement the byte cap. Unchecked native parser/serialization copies are not free. Critical revoke state bypasses ordinary queues.

**Engine-private auxiliary allowance: ≤128 MiB per helper**, plus **≤8 MiB for watchdog private allocations**, is a selected *additional ceiling to qualify*, not a measured estimate or an existing WebRTC engine setting. It covers non-media-buffer allocation such as object metadata, bounded algorithm tables, scheduler/task storage and owned thread stacks. Video/reference surfaces, PCM/history, bitstreams and packet copies must remain in their explicit classes; they cannot be hidden here. Before enabling a profile, an allocation inventory must prove a reservation path or finite preallocation bound for every helper allocation and show that this allowance is enforced. Unknown WebRTC/GStreamer/codec/OS-framework private allocation behavior remains **unknown**, and blocks admission where the required preallocation proof is unavailable. A post-hoc footprint monitor, `RLIMIT_AS` assumption or watchdog kill is not that proof.

These caps exclude no cost from later physical-footprint measurement: binary/runtime mappings, allocator fragmentation, kernel/driver allocations, GPU implementation overhead and system services still require the accepted collector and attribution. They are not established as a finite process-footprint ceiling by this contract. If any excluded category is actually proportional media backing owned/retained by the client, reclassify it into the relevant pool before admission.

## D. Aggregate derivation and required workloads

Let `C` be local+remote logical camera streams and `S` local+remote screen streams. A local camera/screen preview is a lease on that same local stream. For one slot:

```text
surface_MiB = 56*C + 112*S
compressed_video_MiB = 8*(C+S)
audio_MiB = 4
control_MiB = 8
controlled_buffer_MiB = 64*C + 120*S + 12
```

These are upper reservation envelopes, even when some categories are inactive. Do not eagerly allocate them. Screen-sender and receiver workloads remain distinct despite equal reservation arithmetic.

| Profile | C | S | Surface MiB | Compressed MiB | Audio+control MiB | Controlled total MiB | Existing footprint p95 / peak target MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Explicit camera preview only | 1 | 0 | 56 | 8 | 12 | **76** | No separate accepted preview budget; no inherited pass |
| Direct audio | 0 | 0 | 0 | 0 | 12 | **12** | 250 / 320 |
| Direct video: local camera + 1 remote | 2 | 0 | 112 | 16 | 12 | **140** | 450 / 600 |
| Four-person HPB conference: local + 3 remote cameras | 4 | 0 | 224 | 32 | 12 | **268** | 700 / 900 |
| Same conference, local screen sender | 4 | 1 | 336 | 40 | 12 | **388** | 850 / 1,100 |
| Same conference, remote screen receiver | 4 | 1 | 336 | 40 | 12 | **388** | 850 / 1,100 |
| Maximal slot: local + 4 remote cameras; local + 1 remote screen | 5 | 2 | 504 | 56 | 12 | **572** | Separate workload needed |
| Two maximal slots | 10 | 4 | 1,008 | 112 | 24 | **1,144** | Separate workload needed; no doubling/inheritance of a pass |

With the additional auxiliary ceilings, maximal proposed media-controlled allocations are `2×(572+128)+8 = 1,408 MiB`, **conditional on the still-missing allocation proof**. This is neither a new accepted footprint budget nor proof that the product can run both maximum slots within any existing workload budget. No maximum-slot/two-call profile is admitted on the strength of this arithmetic.

The BGRA reservation subset is `32*C + 64*S MiB` (288 MiB for one maximal slot, 576 MiB for two). YUV backing may also be GPU-visible, so that subset is not a total-GPU estimate. Count shared IOSurface/Metal aliases once in the allocation ledger; report each collector's raw counters and known overlaps. Do not add overlapping process/GPU/unified-memory values. Independent GPU copies and intermediates remain charged to their stream slots; if a GPUI/platform import creates unbounded private copies, it is unqualified. Media allocations are not absorbed into the separate 32 MiB image / 64 MiB GPU image caches in ADR-002.

Original CPU, power, network, GPU, latency and footprint targets are unchanged. Native overhead, the main application and all helpers are included by Q.PERF-01 measurement. Internal group topology, hardware codecs, additional layers, stereo or lower quality get independently reviewed workload profiles. Any shortfall in the mandated cameras, screen direction, resolution, fps or speech invalidates the original workload; a safe degraded call does not establish a performance pass.

## E. Before-allocation enforcement and overload

1. **Reserve at admission.** Validate scope, policy, codec, format, counts and exact profile; reserve enough stream slots and auxiliary capacity globally before creating native objects or opening devices. Reject insufficient capacity with `Busy`/`UnsupportedMediaProfile`. Prejoin is explicit, unpublished and charged. Subscriptions/SDP cannot force creation beyond reservations.
2. **Validate at each allocation frontier.** Check frame headers and sizes before codec initialization/decode/reconfiguration, reserve before encode/reassembly/conversion, and configure/qualify native source dimensions and pools before capture starts. The pinned libvpx size guard must be generated as 1920×1080; enforce 1280×720 for camera before dispatch. Advertised SDP dimensions and `max_render_resolution` alone are insufficient. A callback-time size check only contains downstream use; it cannot prevent the already completed allocation.
3. **Own every allocation.** Ledger entries have backing identity, byte charge, owner, stream and generation, format/layout, retain count and fence state. Allocate only after a successful checked reservation. Release credits only after final release confirmation; a dropped queue entry may still have native or GPU owners. Imported memory must have been covered by a qualified producer reservation, not retroactively accepted after an oversized allocation.
4. **Bound overload.** Drop a superseded replaceable waiting frame and record it; never grow pools or queue delay. On sustained exhaustion stop/unsubscribe the affected video source and show reduced quality/unavailable media. Preserve separately authorized audio only while its own bounds and policy are satisfied. Audio overflow reports underrun/overrun, discards obsolete data within policy or stops the call; it cannot extend the 100 ms queue. Any adaptation invalidates an unchanged baseline workload. Remote oversize/unknown formats close the affected stream before unsafe allocation; security/policy loss follows the stronger call-stop rule.
5. **Reclaim on format/generation changes.** Stop old producers and admission, invalidate old leases, drain callbacks/codec references, then retire pools after renderer/GPU completion. Any overlap consumes the same slot and global caps; no double-bank exemption. If no capacity remains, wait boundedly/stop and require deliberate retry, rather than freeing live backing or silently borrowing another account's quota. Stuck old-generation allocations block replacement admission and remain visible to collectors.

The maintained engine does not implement this proposed global ledger. Native decoder/reference buffers, encoded-image growth, source pools and GPU imports still need a measured inventory or upstream enforcement. The alpha must therefore not claim this full allocation profile, helper isolation or a hard process-memory ceiling. Functional interoperability and application queue bounds are narrower results; they do not establish these targets.

## F. Static reproduction and future evidence

The calculations can be reproduced without importing or running project code:

```python
MiB = 1024 * 1024
align = lambda x, n: ((x + n - 1) // n) * n  # arbitrary-precision audit only
for w, h in [(1280, 720), (1920, 1080)]:
    W, H = w + 128, h + 128
    yuv = align(align(W, 64) * H, 16384)
    yuv += 2 * align(align(W // 2, 64) * (H // 2), 16384)
    bgra = align(align(4 * w, 64) * h, 16384)
    print(w, h, yuv, bgra)
for C, S in [(0, 0), (1, 0), (2, 0), (4, 0), (4, 1), (5, 2)]:
    print(C, S, 64 * C + 120 * S + 12)
print(2 * (64 * 5 + 120 * 2 + 12), 2 * (572 + 128) + 8)
```

Expected frame bytes are the four §B values; totals are `12, 76, 140, 268, 388, 572`, then `1144, 1408`. The production implementation needs checked fixed-width arithmetic; the arbitrary-precision audit is not a test of its safety.

Before profile admission, media/platform supplies the native allocation map, executable denial/overflow/lifetime tests and patched source/artifact identities. The independent verifier checks malformed streams, concurrent two-call/prejoin admission, old generations, captured/cached/decoder reference backing and renderer stalls. Performance qualification additionally requires the frozen workload and complete process/lifetime/peak/GPU evidence from the accepted measurement contract. No native memory, codec quality, power or stop-time result is supplied here.

[Apple-queue]: https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/queuedepth
[Apple-threshold]: https://developer.apple.com/documentation/corevideo/kcvreturnwouldexceedallocationthreshold

## G. Qualification limits

Ordinary calls use the in-process Rust `webrtc` engine and GStreamer codecs. Their functional interoperability checks and the older synthetic helper diagnostics do not qualify these budgets. Required allocator hooks still include RTP/encoded-image assembly before allocation, encoder/decoder private references and scratch, audio/jitter queues, task/timer/DTLS backing, native capture/conversion pools and shared-surface generation/fence retirement. Count limits and sampled RSS do not prove aggregate allocation bounds. Physical devices, macOS and whole-process collectors require separate qualification.

See the [native helper interface](../../native/media/README.md) for the diagnostic protocol. Keep diagnostic implementation details and rationale beside the owning code; this contract retains the intended production limits.
