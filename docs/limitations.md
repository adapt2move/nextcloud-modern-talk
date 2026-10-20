# Known limitations

The alpha does not yet implement every feature of the official Talk clients.
Availability follows the server's capabilities, configuration and room policy.
For individual operations, see the [feature inventory](compatibility/feature-inventory.md).

## Servers and calls

- Exercised server baselines are NextCloud 34.0.5 / Talk 24.0.6 and NextCloud
  35.0.2 / Talk 25.0.6. HPB checks use Talk 25.0.6, signaling 2.1.1 and Janus 1.3.0.
- TURN over TCP/TLS and live TURN credential rotation are unavailable in the
  pinned engine. Relay-required calls do not fall back to host candidates.
  UDP relay checks do not establish connectivity through every enterprise network.
- Guest/federated media, required call E2EE, screen/system-audio sharing and
  conference-scale qualification remain outside the supported profile.
  A remote screen offer ends the unsupported call. Permission guidance is not
  a sharing engine.

## Messages and background activity

- Typing indicators, voice messages, video/native document viewers, account-wide
  content search and parts of moderation remain incomplete. Documents can open
  in the Cloud web view; poll drafts remain server-managed.
- Background discovery has bounded capacity. Calendar integration, all mention
  and thread-notification policies, and platform-native incoming-call actions
  remain incomplete. Unknown notification eligibility stays suppressed.
- Loaded history is retained in session memory for up to 32 rooms, with up to
  100 messages per room. It does not survive an app restart, and inactive rooms
  are refreshed when reopened rather than continuously mirrored in full.
  There is no persistent offline message cache or durable outgoing-message
  replay. An uncertain send is never silently retried.

## Platforms

- The macOS app is ad-hoc signed and is not notarized. See the
  [first-launch instructions](install.md#first-launch-blocked-by-macos).
- Mac CI checks compilation, package structure, native library paths and
  signatures. Native login, permissions, devices and update installation still
  need platform qualification. Windows is compile-checked only.
- Native credential storage may be unavailable or report cleanup pending.
  The explicitly selected private-file fallback is persistent but unencrypted.
- Synthetic media and successful compilation do not qualify physical devices,
  every OS, semantic accessibility or whole-process performance and energy use.

Report reproducible problems without credentials, private server details or
message contents. Security issues follow [SECURITY.md](../SECURITY.md).
