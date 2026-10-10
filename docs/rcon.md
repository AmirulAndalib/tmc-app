# RCON

`rcon::` speaks two dialects: **Source** RCON (which many other games' servers also speak) and **Frostbite** R-CON. Both parse through `net::reader` and both have the truncation test every parser in this crate has.

Three protocol details that are easy to get wrong:

- **A Source multi-packet reply has no end marker.** After the command, a second empty packet with a different id is sent; the server answers in order, so its echo is the end. Stopping at the first packet truncates `status` on a full server; waiting for more hangs on every short reply.
- **A successful Source auth sends TWO packets**, in an order implementations disagree about, so the handshake reads until it sees the auth reply rather than counting.
- **Frostbite's `size` counts the two header fields it is part of.** Omitting them leaves four bytes of each packet in the buffer, which presents as "the second command always fails".

## The one deliberate SSRF gap

Everywhere else an address goes through `net::addr::resolve_public`. **RCON does not**, and the module header says so outright: `192.168.1.10` and `127.0.0.1` are the *normal* answers here, because the feature is "administer my server" and most people's server is on their own network. A guard that refused them would refuse the feature.

Bounded instead by a per-host connection cooldown, a session cap, and a Security-level audit entry on every connect. It remains a widening, and [SECURITY.md](../SECURITY.md) lists it as one.

## The passwords

Encrypted on the device with XChaCha20-Poly1305, key in the OS credential store (`crypto::LocalCipher`), and **never sent to the website**. A server password is not derived from a TMC account, losing it costs somebody their server rather than their profile, and no feature on the site needs it.

What that buys is stated honestly in `crypto.rs`: it answers a **copied file** — a backup, a synced folder, a disk pulled out of a laptop. It does not answer code running as the user in this session, and nothing on any desktop platform does. What the design guarantees instead is that the webview is not such code: `rcon_secret` is `pub(crate)`, `rcon::exec_saved` is its only caller, and `RconServer` has no password field for a refactor to start returning.
