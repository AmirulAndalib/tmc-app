# Live server queries

**The app talks to game servers itself.** The website can only show you what its scanner last saw, from wherever the scanner lives; the app sends the game's own query datagram from the user's device, so the player count is current and the latency is the one that player will actually experience. A Sydney player and a Frankfurt player see different numbers for the same box, and only the app can tell either of them the truth.

## Where the protocol comes from

`App.srvQueryProtocols` on the website — the same list its own scanners use, so there is no second source to drift. It arrives on every server row as `server.query`, along with `timeoutMs` and two port fields.

## Which port gets queried, and why not the other two

**Explicit `queryPort` if the row has one, otherwise the GAME port.** That is the whole rule, and the authority for it is the scanner that writes these rows — `spy/internal/scanners/server.go`:

```go
qPort := port
if srv.PortQuery != nil && *srv.PortQuery > 0 {
    qPort = *srv.PortQuery
}
```

The two fields the API also sends are **not** inputs to that decision, and treating them as such is what made every Source server look dead:

- **`portOffset`** (`App.srvGamePortQueryOffset`) is dead config. Grep the whole scanner for it and you get one hit: the struct field it deserialises into. Nothing reads it. A game whose App row carries a stale non-zero offset is scanned by the site on its game port, so probing `game + offset` from here is a port nobody is listening on — silence, which renders as a timeout.
- **`swapGamePort`** (`App.srvSwapGamePort`) is post-scan bookkeeping, not port selection. In `spy/internal/protocols/a2s.go` it means "once A2S_INFO comes back, read the true game port out of the extended-info block and swap the stored `port`/`portQuery`". Reading it as "query the game port" gave the right number often enough to look correct, which is worse than being wrong outright.

The only per-protocol exceptions are the scanner's own hardcoded ones, and they apply only when the row has no explicit query port: **FiveM → 30120**, **SCUM → game + 2**, **Frostbite → game + 22000**. `resolve_port` carries all three with the Go file named. Frostbite's also declines the arithmetic above a game port of 43535 rather than wrapping past 65535 — a wrapped port is not just wrong, it probes a stranger's unrelated service on a low port number.

Getting this wrong is still the single most common reason a live server shows as dead — which is why the rule is copied rather than invented.

`server.query` is null when the owner hid the network details or the game declares no protocol. In both cases there is nothing the app is entitled to probe, and it must not fall back to guessing.

## Implemented protocols

Each enum name is the protocol's, and the games that speak it are listed per app on the website (`App.srvQueryProtocols`), not here.

| Protocol | Notes |
| --- | --- |
| `A2S` | Challenge handshake + split-reply reassembly |
| `MINECRAFT` | Protocol 1.7+: TCP handshake → status JSON |
| `MINECRAFT_SLP` | Pre-1.7 server list ping: `0xFE 0x01`, UTF-16BE reply |
| `QUAKE3` | `getstatus` |
| `GAMESPY1` | `\status\` |
| `GAMESPY2` | Binary, per-section request byte |
| `GAMESPY3` | Signed challenge — see below |
| `GAMESPY4` | Identical game-server query to v3; shares its implementation, tagged separately |
| `SAMP` | IPv4 only, by protocol design |
| `FIVEM` | HTTP `/dynamic.json` + `/info.json` |
| `FROSTBITE` | R-CON over TCP on game + 22000; roster columns keyed by tag name |
| `TEAMSPEAK3` | ServerQuery on a fixed 10011; the game port SELECTS a virtual server rather than being connected to |
| `HYTALE_NITRADO` | HTTPS status document on game + 3, self-signed |

## The four that stay on `TCP_ONLY`, and why it is not laziness

`DISCORD`, `SCUM`, `GTA_NETWORK` and `GTA_RAGE` are the last unimplemented entries, and each is unimplementable *here* for the same reason:

| Protocol | What `spy` actually asks |
| --- | --- |
| `SCUM` | `api.hellbz.de` |
| `GTA_NETWORK` | `multiplayerhosting.info` |
| `GTA_RAGE` | the RAGE:MP master list at `cdn.rage.mp` |
| `DISCORD` | nothing — the handler is a stub |

None of them talks to the server. That is fine for a scanner reading a list once on everybody's behalf, and wrong for this app three times over: **the latency would describe that third party's hosting**, identical for every row (`spy` says so itself in three comments, and records no latency for any of them); it would **tell a third party every server a user scrolls past**; and on a phone it would be one HTTPS request per row per tick. So they measure a real TCP handshake to the real box instead, and their player counts come from the API — which got them from the scanner, which read those lists once.

`net/query/hytale.rs`'s module header carries this, because it is the one of the five that DID qualify.

Everything else in the enum is recognised but falls through to `TCP_ONLY`, which still yields a real latency figure. A Server Live Query plugin can cover any of them without an app update.

## Rules for anything under `net/query/`

These parsers read bytes from an unauthenticated machine that may be actively hostile, and the release profile sets `panic = "abort"` — so **a panic in a parser is a remote kill switch**. Consequently:

- **No indexing, no slicing.** Everything goes through `net::reader::Reader`, which bounds-checks every read and returns `Err` past the end.
- **Every length from the wire is checked against what remains.** A server claiming a 4 GB string is the normal case to handle, not an edge case.
- **Every loop is bounded** — fragments, entries, rules, roster length.
- **A truncated-input test is mandatory.** Each protocol has one that feeds every prefix of a valid reply through the parser; it must never panic.
- **`resolve_public` is the only way to get a `SocketAddr`.** Resolve once and connect to *that* — checking a hostname and reconnecting by name is a DNS rebinding hole, and the guard is what stops the browser being an SSRF tool against the user's own LAN.

Three protocol details that are easy to get wrong and are commented at the site:

- **A2S's `0x41` challenge.** Skip it and every modern server looks unreachable.
- **A2S's challenge is bound to the source PORT.** Use `net::transport::UdpSession` so the retry goes out on the socket that asked; a fresh ephemeral port is ignored by several server builds. It is also what makes split replies readable, since the continuation datagrams arrive unprompted on that socket.
- **GameSpy v3's challenge is a SIGNED decimal string.** Parse it unsigned and roughly half of all servers look unreachable.
- **A split A2S reply's `total` and `number` are two SEPARATE bytes.** GoldSrc packs them into one (index high nibble, total low) and has no split-size field; Orange Box and later use two bytes plus a size. Reading the packed form while also consuming a size field parses fragment 0 plausibly — `total = 2, index = 0` — and decodes every LATER fragment to index 0 as well, so they overwrite each other and every split roster comes out "incomplete". The authority is `go-a2s`, which is the library `spy` queries A2S with.

## The registry

`LiveQueryProvider` holds one timer and one registry. Cards register via an `IntersectionObserver` while on screen; every tick sends the whole visible set to Rust in a single `query_servers` call, which applies the user's concurrency cap. Fifty cards each polling would be fifty timers, fifty IPC round trips and fifty simultaneous sockets — which a phone's radio and most home routers handle badly.

Deregistration has a 5-second grace so a flick-scroll does not cancel work already in flight, and polling stops entirely while the window is hidden.

**The cadence is the user's**: `latencyIntervalMs`, one second by default, set under Settings → App → Servers and clamped to 250ms–5min *in Rust* (`LATENCY_INTERVAL_MS_MIN`/`MAX`) because it decides how often a third party's game server is sent a datagram. The provider publishes it as `intervalMs` so the single-server panel polls on the same number rather than on a constant of its own. A tick that arrives while one is in flight is remembered, not stacked, so a fast interval degrades to "as fast as the batch completes" rather than to overlapping batches.

Three properties of the scheduling that are easy to undo by accident:

- **A row's FIRST probe comes from `watch`'s kick, not from the interval.** The provider mounts at app start, so its warm-up has long since fired by the time anyone opens the browser; keying first measurements off the refresh interval left a fresh screenful showing ellipses, which reads as the feature being broken rather than slow. It is also what keeps a slow interval usable: a row is measured a quarter-second after it scrolls into view whatever the setting.
- **The batch is sorted unmeasured-first.** Rust caps a batch at 64 and *drops* the rest, so an unsorted batch during a fast scroll spends the cap on rows that already have a number and strands the ones that do not.
- **A failed `query_servers` call is applied as a failed probe for every row in it.** Otherwise a broken IPC boundary — an unregistered command, a drifted schema — is indistinguishable on screen from a browser full of slow servers.

## How a latency reading is drawn

| State | Shown | Meaning |
| --- | --- | --- |
| measured | `32ms`, coloured | <90 green · <150 yellow · <200 orange · ≥200 red-orange |
| timeout | `TO` in red | A probe went out and nothing came back |
| waiting | `…` | No probe has resolved yet |
| none | `—` | The owner hid the address; there is nothing to probe |

The ladder lives in `LATENCY_TIERS` /`latencyTone` and the four states in `latencyState`, both in one place so the card, the table and the server panel cannot disagree. `TO` exists because "500ms" and "no answer" are different facts and a dash for both is how a server browser earns a reputation for lying.

## Latency history

`net::latency::LatencyStore` keeps up to 120 samples for up to 512 servers, in memory only — the series is interesting while the browser is open, is entirely reconstructible by re-measuring, and persisting it would mean a disk write per scroll tick. That is a COUNT, not a window — at the default one-second cadence it is two minutes of history, and proportionally more at a slower one. **Failed probes are recorded as `None`, not dropped**: an intermittently-dead server must not graph as perfectly stable.

Graphs are hand-rolled SVG (`components/latency-graph.tsx`). No chart library: these render once per card in a scrolling grid, and a general-purpose chart's layout pass and scales get paid fifty times for a sixty-point line with no axes. A dropped probe **breaks the line** rather than interpolating across it, and the Y scale has a 40ms floor so a rock-steady server does not render as a mountain range.

"Sort by ping" in the server browser is client-side and could only be: latency is a fact about this device's route, so the API cannot order by it.
