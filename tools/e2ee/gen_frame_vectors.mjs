// Generates frame-cipher vectors with the exact WebCrypto steps of Nextcloud Talk's
// web client (spreed src/utils/e2ee/crypto-utils.js and JitsiEncryptionWorkerContext.js),
// so the native cipher is checked against the reference rather than against itself.
// Usage: node tools/e2ee/gen_frame_vectors.mjs > crates/talk-adapters/src/media/e2ee/fixtures/frame_vectors.json
import { webcrypto as crypto } from 'node:crypto'

const hex = (bytes) => Buffer.from(bytes).toString('hex')
const enc = new TextEncoder()

async function importKey(raw) {
  return crypto.subtle.importKey('raw', raw, 'HKDF', false, ['deriveBits', 'deriveKey'])
}
async function deriveKeys(material) {
  const encryptionKey = await crypto.subtle.deriveKey({
    name: 'HKDF', salt: enc.encode('TalkFrameEncryptionKey'), hash: 'SHA-256', info: new ArrayBuffer(),
  }, material, { name: 'AES-GCM', length: 128 }, false, ['encrypt', 'decrypt'])
  return { material, encryptionKey }
}
async function ratchet(material) {
  return crypto.subtle.deriveBits({
    name: 'HKDF', salt: enc.encode('TalkFrameRatchetKey'), hash: 'SHA-256', info: new ArrayBuffer(),
  }, material, 256)
}
const UNENCRYPTED = { key: 10, delta: 3, audio: 1 }

async function encode(keyBytes, keyIndex, kind, iv, frame) {
  const { encryptionKey } = await deriveKeys(await importKey(keyBytes))
  const header = frame.subarray(0, UNENCRYPTED[kind])
  const cipher = new Uint8Array(await crypto.subtle.encrypt(
    { name: 'AES-GCM', iv, additionalData: header }, encryptionKey, frame.subarray(UNENCRYPTED[kind])))
  const out = new Uint8Array(header.length + cipher.length + iv.length + 2)
  out.set(header)
  out.set(cipher, header.length)
  out.set(iv, header.length + cipher.length)
  out[out.length - 2] = 12
  out[out.length - 1] = keyIndex
  return out
}

const fill = (n, seed) => Uint8Array.from({ length: n }, (_, i) => (i * 31 + seed) & 0xff)
const key = fill(32, 7)
const material = await importKey(key)
const ratcheted = new Uint8Array(await ratchet(material))
const ratcheted2 = new Uint8Array(await ratchet(await importKey(ratcheted)))

const cases = []
for (const [kind, size, index] of [['audio', 40, 0], ['delta', 300, 3], ['key', 2000, 15], ['key', 10, 1], ['audio', 1, 2]]) {
  const frame = fill(size, size)
  const iv = fill(12, size + 1)
  cases.push({
    kind, key_index: index, iv: hex(iv), plain: hex(frame),
    encrypted: hex(await encode(key, index, kind, iv, frame)),
  })
}
// A frame encrypted with the key two ratchet steps ahead of the receiver.
const ahead = fill(120, 9)
const aheadIv = fill(12, 99)
cases.push({
  kind: 'delta', key_index: 5, iv: hex(aheadIv), plain: hex(ahead),
  encrypted: hex(await encode(ratcheted2, 5, 'delta', aheadIv, ahead)), ratchet_steps: 2,
})

console.log(JSON.stringify({
  key: hex(key), ratchet_1: hex(ratcheted), ratchet_2: hex(ratcheted2), cases,
}, null, 2))
