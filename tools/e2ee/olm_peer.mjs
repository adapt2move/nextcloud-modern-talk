// A participant running Nextcloud Talk's own key-exchange logic (spreed
// src/utils/e2ee/encryption.js, same messages, same Olm library) over JSON lines.
// The Rust test in crates/talk-adapters drives it. Requires `npm ci` in this folder.
//
// Input lines:  {"cmd":"init","session":"..."} {"cmd":"joined","sessions":[...]}
//               {"cmd":"left","sessions":[...]} {"cmd":"message","from":"..","payload":{..}}
//               {"cmd":"rotate"} {"cmd":"quit"}
// Output lines: {"send":{"to":"..","payload":{..}}} {"key":{"peer":"..","key":"<b64>","index":n}}
//               {"local":{"key":"<b64>","index":n}}
import Olm from '@matrix-org/olm'
import { randomBytes, randomUUID } from 'node:crypto'
import readline from 'node:readline'

await Olm.init()
const out = (value) => process.stdout.write(JSON.stringify(value) + '\n')
const b64 = (bytes) => Buffer.from(bytes).toString('base64')

let sessionId = ''
let key = new Uint8Array(randomBytes(32))
let keyIndex = 0
const sessions = {}
const account = new Olm.Account()
account.create()
const identity = JSON.parse(account.identity_keys())

const data = (id) => (sessions[id] = sessions[id] || {})
const send = (to, payload) => out({ send: { to, payload } })
const encryptKey = (session) => session.encrypt(JSON.stringify({ key: b64(key), index: keyIndex }))

function startSession(id) {
  const d = data(id)
  account.generate_one_time_keys(1)
  const otk = Object.values(JSON.parse(account.one_time_keys()).curve25519)[0]
  account.mark_keys_as_published()
  d.startMsgId = randomUUID()
  send(id, { id: d.startMsgId, type: 'encryption.start', identity: identity.curve25519, key: otk })
}
function setPeerKey(id, decoded) {
  const k = Buffer.from(decoded.key, 'base64')
  if (!data(id).lastKey || !data(id).lastKey.equals(k)) {
    data(id).lastKey = k
    out({ key: { peer: id, key: decoded.key, index: decoded.index } })
  }
}
function sendKey(id) {
  const d = data(id)
  if (!d.session) return
  send(id, { id: randomUUID(), type: 'encryption.setkey', key: encryptKey(d.session) })
}

function handle(from, payload) {
  const d = data(from)
  switch (payload.type) {
    case 'encryption.start': {
      if (d.session) return send(from, { type: 'encryption.error', error: 'Session already created' })
      const session = new Olm.Session()
      session.create_outbound(account, payload.identity, payload.key)
      d.session = session
      send(from, { id: payload.id, type: 'encryption.finish', key: encryptKey(session) })
      break
    }
    case 'encryption.finish': {
      if (d.session || payload.id !== d.startMsgId) return send(from, { type: 'encryption.error', error: 'bad finish' })
      const session = new Olm.Session()
      session.create_inbound(account, payload.key.body)
      account.remove_one_time_keys(session)
      const decoded = JSON.parse(session.decrypt(payload.key.type, payload.key.body))
      d.session = session
      delete d.startMsgId
      if (decoded.key) setPeerKey(from, decoded)
      sendKey(from)
      break
    }
    case 'encryption.setkey': {
      if (!d.session) return send(from, { type: 'encryption.error', error: 'No session for setting key' })
      const decoded = JSON.parse(d.session.decrypt(payload.key.type, payload.key.body))
      if (decoded.key !== undefined && decoded.index !== undefined) {
        setPeerKey(from, decoded)
        send(from, { id: payload.id, type: 'encryption.gotkey', key: encryptKey(d.session) })
      }
      break
    }
    case 'encryption.gotkey': {
      if (!d.session) return
      const decoded = JSON.parse(d.session.decrypt(payload.key.type, payload.key.body))
      if (decoded.key !== undefined && decoded.index !== undefined) setPeerKey(from, decoded)
      break
    }
  }
}

const lines = readline.createInterface({ input: process.stdin })
for await (const line of lines) {
  const cmd = JSON.parse(line)
  try {
    if (cmd.cmd === 'init') {
      sessionId = cmd.session
      out({ local: { key: b64(key), index: keyIndex } })
    } else if (cmd.cmd === 'joined') {
      for (const id of cmd.sessions) if (id < sessionId) startSession(id)
    } else if (cmd.cmd === 'left') {
      for (const id of cmd.sessions) delete sessions[id]
    } else if (cmd.cmd === 'message') {
      handle(cmd.from, cmd.payload)
    } else if (cmd.cmd === 'rotate') {
      key = new Uint8Array(randomBytes(32))
      keyIndex++
      for (const id of Object.keys(sessions)) sendKey(id)
      out({ local: { key: b64(key), index: keyIndex } })
    } else if (cmd.cmd === 'ping') {
      out({ pong: true })
    } else if (cmd.cmd === 'quit') {
      break
    }
  } catch (e) {
    out({ error: String(e) })
  }
}
process.exit(0)
