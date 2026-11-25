// Installed before Talk starts. Observe its remote receivers and visible video
// elements. Nothing captured here is fed back into the call or saved to disk.
(() => {
    const peers = new Set();
    const tracks = new WeakMap();
    // Talk 25 installs its frame decryptor using operation "decode". Keep only
    // transform identity; the worker, keys and encoded frames stay with Talk.
    const decoders = new WeakSet();
    const canObserveTransform = typeof window.RTCRtpScriptTransform === 'function'
        && typeof Object.getOwnPropertyDescriptor(RTCRtpReceiver.prototype, 'transform')?.get === 'function';
    if (canObserveTransform) {
        window.RTCRtpScriptTransform = new Proxy(window.RTCRtpScriptTransform, {
            construct(target, args, newTarget) {
                const transform = Reflect.construct(target, args, newTarget);
                if (args[1]?.operation === 'decode') decoders.add(transform);
                return transform;
            }
        });
    }

    function hasTalkDecoder(receiver) {
        if (!canObserveTransform) return null;
        const transform = receiver.transform;
        if (transform === null) return false;
        return decoders.has(transform) ? true : null;
    }

    let nextTrack = 1;
    let context;
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 8;
    const pixels = canvas.getContext('2d', {willReadFrequently: true});

    window.RTCPeerConnection = new Proxy(window.RTCPeerConnection, {
        construct(target, args) {
            const peer = new target(...args);
            if (peers.size >= 8) throw Error('receiver peer limit');
            peers.add(peer);
            peer.addEventListener('connectionstatechange', () => {
                if (peer.connectionState === 'closed') peers.delete(peer);
            });
            return peer;
        }
    });

    function observe(track) {
        if (tracks.has(track)) return tracks.get(track);
        const state = {id: nextTrack++};
        if (track.kind === 'audio') {
            context ??= new AudioContext();
            const source = context.createMediaStreamSource(new MediaStream([track]));
            const analyser = context.createAnalyser();
            analyser.fftSize = 2048;
            analyser.smoothingTimeConstant = 0;
            const mute = context.createGain();
            mute.gain.value = 0;
            source.connect(analyser).connect(mute).connect(context.destination);
            context.resume();
            track.addEventListener('ended', () => {
                source.disconnect();
                analyser.disconnect();
                mute.disconnect();
            }, {once: true});
            state.analyser = analyser;
            state.bins = new Float32Array(analyser.frequencyBinCount);
        }
        tracks.set(track, state);
        return state;
    }

    function hasTone(state) {
        if (context.state !== 'running') return false;
        const {analyser, bins} = state;
        analyser.getFloatFrequencyData(bins);
        const bin = Math.round(1000 * analyser.fftSize / context.sampleRate);
        const peak = Math.max(...bins.subarray(bin - 1, bin + 2));
        const rest = Math.max(...bins.subarray(2, bin - 3), ...bins.subarray(bin + 4));
        return peak > -48 && peak > rest + 10;
    }

    function visibleLuma(track) {
        for (const video of document.querySelectorAll('video')) {
            if (!video.srcObject?.getVideoTracks?.().includes(track) || video.paused ||
                !video.videoWidth || video.readyState < 2 ||
                !video.checkVisibility({opacityProperty: true, visibilityProperty: true})) continue;
            const box = video.getBoundingClientRect();
            const left = Math.max(0, box.left), top = Math.max(0, box.top);
            const width = Math.min(innerWidth, box.right) - left;
            const height = Math.min(innerHeight, box.bottom) - top;
            if (width < 16 || height < 16) continue;
            const exposed = [[0.25, 0.25], [0.5, 0.5], [0.75, 0.75]].some(([x, y]) =>
                document.elementFromPoint(left + width * x, top + height * y) === video);
            if (!exposed) continue;
            pixels.drawImage(video, 0, 0, 8, 8);
            const rgba = pixels.getImageData(0, 0, 8, 8).data;
            let sum = 0;
            for (let i = 0; i < rgba.length; i += 4) {
                sum += (rgba[i] + rgba[i + 1] + rgba[i + 2]) / 3;
            }
            return Math.round(sum / 64);
        }
        return null;
    }

    window.deviceCallSample = async () => {
        const rows = [];
        for (const peer of peers) {
            if (peer.connectionState !== 'connected') continue;
            for (const receiver of peer.getReceivers()) {
                const track = receiver.track;
                if (!track || track.readyState !== 'live') continue;
                const state = observe(track);
                const stats = await receiver.getStats();
                for (const row of stats.values()) {
                    if (row.type !== 'inbound-rtp') continue;
                    const codec = stats.get(row.codecId)?.mimeType?.toLowerCase();
                    const decoder = hasTalkDecoder(receiver);
                    if (codec === 'audio/opus') {
                        rows.push({id: state.id, decoder, packets: row.packetsReceived, tone: hasTone(state)});
                    } else if (codec === 'video/vp8') {
                        rows.push({id: state.id, decoder, frames: row.framesDecoded, luma: visibleLuma(track)});
                    }
                }
            }
        }
        return rows;
    };
})();
