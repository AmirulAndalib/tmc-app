# CLAUDE.md — tmc-app

Guidance for working in this repository. Specific to `tmc-app`; separate from
`../website-city/CLAUDE.md` and `../tmc-global/CLAUDE.md`.

## What this is

The official **The Modding Community** app: Rust + Tauri 2 + React 19, shipping
to **Windows, macOS, Linux, Android and iOS from one codebase**.

It covers what the website covers — browsing and viewing assets, mods, servers,
server maps, articles, communities, collections and members — plus the things a
browser tab cannot do:

- **One-click installs** through a declarative plugin system
- **Sandboxes** — named mod profiles, deployed by hard link, symbolic link or
  direct copy, with the game folder recoverable either way
- **Real latency pings** measured from the user's own device
- **Live server queries** over game-specific protocols
- **A download queue** with pause, resume, priorities and bandwidth limits
- **RCON**, with the passwords encrypted on the device and never uploaded
- **Finding the games** — Steam, Epic, GOG and the rest, read from what they
  already wrote down
- **Local-first settings**, kept deliberately separate from account settings

It is **not** a wrapper around the website. There is no landing page, no
marketing header, no footer. It opens on a browser and stays there.

## Sibling repositories

| Repo | Relationship |
| --- | --- |
| `../website-city` | The Next.js site. **Owns the API this app speaks** (`/api/app/v1`) and the auth flow. Changes there usually need a matching change here. |
| `../spy` | The Go scanner that populates every server row. **The authority on query ports and protocol quirks** — when this app and the site disagree about how to reach a server, `spy` is what settles it. |
| `../tmc-global` | `@modcommunity/shared` — design tokens and a few UI primitives, consumed from GitHub Packages. `npm run shared:local` swaps in the sibling checkout. |
| `../website-processing` | Astro landing site. No relationship to this app beyond sharing the design system. |

## Architecture, and the one rule everything follows

> **Credentials and the filesystem live in Rust. The webview gets data.**

There is no `fetch` in the frontend and no `get_token` command. A mod
description is untrusted text rendered inside a webview that can call `invoke`;
the defence is that there is nothing worth stealing on that side of the bridge.
Every layer below assumes an attacker has already achieved script execution in
the webview and asks what they can reach.

**One command takes a path from the webview**: `fs_list_dirs`, which backs the
app's own folder picker and returns directory *names* only — no files, no reads,
no writes. Its module header states exactly what that widens. Nothing else in
`commands/` accepts a path.

```
React (src/)                       ← untrusted content renders here
  │  invoke() via src/lib/ipc      ← every call parsed through a zod schema
  ▼
tmc-app  (src-tauri/src/)          ← the Tauri shell, and nothing else
  ├── commands/   the IPC surface
  ├── state.rs    AppState, assembled once
  └── paths.rs    platform paths, from Tauri's resolver
  │
  ▼
tmc-core (src-tauri/core/src/)     ← NO Tauri dependency; tests anywhere
  ├── auth.rs     PKCE + device grant; tokens never leave this process
  ├── secure.rs   refresh token + named secrets → OS credential store
  ├── crypto.rs   XChaCha20-Poly1305 for what has to be kept, not hashed
  ├── api.rs      the ONLY HTTP client; attaches the bearer, handles refresh
  ├── net/        address guard, transports, latency history, game protocols
  ├── rcon/       Source and Frostbite consoles; the one deliberate SSRF gap
  ├── download/   the queue: priorities, resume, rate limits
  ├── detect/     where the launchers say the games are
  ├── plugins/    manifest → jail → step executor, all declarative
  ├── deploy/     merge tree → link/copy → ledger; how mods reach the game
  ├── library/    subscriptions, sandboxes, staging, deployment
  ├── launch.rs   the only place a process is spawned
  ├── deeplink.rs what a `tmc://` link may mean, which is "show a page"
  └── logging.rs  the audit trail Settings → Logging reads
  │
  ├──► website-city  /api/app/v1   (catalogue, auth, sandboxes)
  ├──► game servers directly       (live queries, latency, RCON)
  └──► wherever a mod's files are  (the download queue)

tmc-usvfs (src-tauri/usvfs/)       ← the virtual filesystem, Windows-only halves
  ├── tree.rs     the merged view a game is shown instead of its own folder
  ├── shm.rs      publishing that tree where an injected process can map it
  ├── hooks.rs    the import-table patch, inside somebody else's game
  └── inject.rs   suspended launch + remote LoadLibrary
```

**`tmc-usvfs` is a third crate rather than a module** for one concrete reason:
it is the only part of the tree that can be `cargo check`ed for Windows from a
Linux box. `tmc-core` cannot — bundled SQLite needs a C cross-compiler — so
code that reaches into another process would otherwise be verified by reading
it. The split is what makes `cargo check -p tmc-usvfs --target
x86_64-pc-windows-gnu` possible, and that command has already caught four real
bugs.

**The workspace split is load-bearing.** `tmc-core` holds the plugin jail,
the protocol parsers and the token lifecycle — the three things you most want to
compile and fuzz on a CI runner with no display stack, no WebKit and no dbus.
`cargo test -p tmc-core` needs none of them. Anything you add that does not
genuinely need a window belongs on that side of the line.

### Source map

**`src-tauri/core/src/` — `tmc-core`**

| File | Owns |
| --- | --- |
| `api.rs` | HTTP. `api_base()` — compile-time, or `TMC_API_BASE` in a debug build. Never a setting |
| `auth.rs` | PKCE, device-grant state, in-memory access token |
| `secure.rs` | Keychain / Credential Manager / Secret Service, file fallback on mobile. Keyed per API base |
| `settings.rs` | App-local settings (`settings.json`), clamped on read |
| `anchor.rs` | **What a jail anchor may be.** Guards `gameDirs` / `downloadDir` |
| `crypto.rs` | The device key, and what it does and does not buy |
| `deeplink.rs` | The closed list of what a `tmc://` link may ask for |
| `logging.rs` | Append-only JSONL audit log + `audit!` macro |
| `net/addr.rs` | **The public-address guard.** Resolve once, connect to that |
| `net/transport.rs` | Bounded UDP/TCP exchanges — every read has a deadline and a cap |
| `net/reader.rs` | The non-panicking byte cursor every parser is built on |
| `net/ping.rs` | TCP connect timing, for games with no protocol |
| `net/latency.rs` | Rolling per-server history, bounded on both axes |
| `net/query/` | The game protocols — see below |
| `plugins/manifest.rs` | The manifest format and its validation |
| `plugins/jail.rs` | **The path jail.** Called a jail, not a sandbox — see below |
| `plugins/steps.rs` | The install/uninstall executor |
| `plugins/apps.rs` | Per-game rules: where mods go, how to launch, sandbox presets |
| `plugins/query.rs` | The declarative parser for Server Live Query plugins |
| `plugins/theme.rs` | Theme token validation |
| `plugins/registry.rs` | Installed plugins, approvals, fingerprint drift, the signature gate |
| `plugins/signature.rs` | **Who vouched for a plugin.** Ed25519 over the canonical manifest, against the user's own trust store |
| `deploy/merge.rs` | The virtual tree: who wins each path, and what conflicts |
| `deploy/link.rs` | One syscall each, and the platform reason it might fail |
| `deploy/ledger.rs` | What landed, what it displaced, and how to undo both |
| `deploy/engine.rs` | Strategy selection, the diff against last time, placement |
| `library/db.rs` | The device's SQLite store — subscriptions, sandboxes, queue, RCON |
| `library/sync.rs` | Reconciling against the account, on a watermark |
| `library/install.rs` | Materialising one subscription into the main game folder |
| `library/sandbox.rs` | Sandboxes: environment, strategy, options, load order |
| `library/deploy.rs` | Staging a mod into a sandbox, then deploying the sandbox |
| `download/rate.rs` | The token bucket behind both bandwidth limits |
| `download/mod.rs` | The queue itself |
| `download/store.rs` | The queue, across a restart |
| `detect/steam.rs` | `libraryfolders.vdf`, then every `appmanifest_*.acf` |
| `detect/epic.rs` | Epic's `.item` manifests, and Heroic/Legendary on Linux |
| `detect/gog.rs` | Galaxy's SQLite, read-only and `immutable=1` |
| `detect/folders.rs` | Xbox, Ubisoft, EA, Battle.net, and a game's own hints |
| `detect/vdf.rs` | Valve KeyValues, bounded on every axis |
| `rcon/source.rs` | Valve's protocol, which is also Minecraft's |
| `rcon/frostbite.rs` | Battlefield's, with `login.hashed` |
| `rcon/store.rs` | Saved servers. Passwords encrypted; none of it leaves the device |
| `launch.rs` | The only place a launch is RESOLVED. Produces a plan; runs nothing |

**`src-tauri/usvfs/src/` — `tmc-usvfs`**

| File | Owns |
| --- | --- |
| `tree.rs` | The merged view: resolution, directory listings, case rules. Platform-independent, tested everywhere |
| `shm.rs` | The blob format, published atomically and parsed defensively — it is read inside a game |
| `hooks.rs` | IAT patching and the redirecting `CreateFileW`. Windows only |
| `inject.rs` | `CREATE_SUSPENDED` + `CreateRemoteThread(LoadLibraryW)`, and the quoting `CommandLineToArgvW` demands |

**`src-tauri/src/` — `tmc-app`**

| File | Owns |
| --- | --- |
| `lib.rs` | Builder, plugin registration, the command list |
| `commands/` | The whole IPC surface. Nothing privileged happens outside it |
| `commands/fs.rs` | Directory listing for the app's folder picker. Names only, never contents |
| `commands/sandbox.rs` | Sandboxes, staging, deploying. Never takes a directory |
| `commands/downloads.rs` | The queue. No command here takes a URL or a destination — `download_release` takes ids |
| `commands/detect.rs` | Scan, and separately apply. A scan configures nothing |
| `commands/rcon.rs` | Consoles. No command returns a password |
| `commands/library.rs` | Sync, install, uninstall, launch |
| `spawn.rs` | The only place in the app that starts a process. Which mechanism is the PLAN's decision, not the caller's |
| `state.rs` | `AppState`, assembled once — shared locks and caches depend on that |
| `paths.rs` | Every path, from Tauri's resolver — never `$HOME` |

**`src/`**

| Path | Owns |
| --- | --- |
| `lib/ipc/` | `call()` + schemas + `ipc.*` + `messageOf`. The only place `invoke` is imported |
| `lib/api/contract.ts` | **Mirror** of website-city's contract. `npm run contract:sync` |
| `lib/api/client.ts` | `api.*`, every response zod-parsed |
| `lib/api/labels.ts` | How a game is named on screen — always its full name |
| `lib/api/offline-cache.ts` | What a cold launch shows before the network answers. Allow-listed, public data only |
| `lib/api/env.ts` | Which site this build talks to, for display and for the site's own links |
| `lib/auth/provider.tsx` | Login state and the poll loop |
| `lib/settings/provider.tsx` | App settings + account settings, kept apart |
| `lib/hooks/use-app-icons.ts` | Game artwork for screens whose data is local |
| `lib/hooks/use-breakpoint.ts` | Layout decisions, keyed on the window |
| `lib/hooks/use-platform.ts` | The few decisions that genuinely are per-OS, not per-window |
| `lib/hooks/use-live-query.tsx` | The live-server registry: one timer, one batch |
| `lib/hooks/use-deep-link.tsx` | Turns a `tmc://` link into a route. Never into an action |
| `lib/downloads/provider.tsx` | The queue, pushed from Rust and coalesced |
| `lib/library/provider.tsx` | The subscription sync loop |
| `lib/external.ts` | Which content kinds are handed to the system browser |
| `components/shell.tsx` | Sidebar ≥768px, bottom tabs below |
| `components/titlebar.tsx` | The app's own window frame — see "Cross-platform" |
| `components/update-banner.tsx` | "There is a newer version", and nothing more. Never installs anything |
| `components/folder-picker.tsx` | The in-app folder chooser, over `commands/fs.rs` |
| `components/game-icon.tsx` | A game's artwork, with a deterministic initials fallback |
| `components/item-thumb.tsx` | An item's cover in a list, from the LOCAL library row |
| `components/launch-dialog.tsx` | The confirmation shown before anything starts, shared by both launchers |
| `components/markdown.tsx` | The safe renderer for untrusted bodies |
| `components/select.tsx` | **The app's own dropdown.** A native `<select>`'s popup cannot be themed |
| `components/speed-graph.tsx` | A download's recent speed, hand-rolled SVG |
| `components/gallery.tsx` | Screenshots, and the lightbox behind them |
| `components/reviews.tsx` | The review list and its distribution bars |
| `components/report-button.tsx` | Reporting, into the website's own moderation queue |
| `components/latency-graph.tsx` | Sparkline + full chart + the latency ladder, hand-rolled SVG |
| `components/server-live.tsx` | The live strip on a server card |
| `components/server-table.tsx` | The server browser's default view: table, expandable rows |
| `components/browse-filters.tsx` | The filter panel: collapsible groups, kind-aware, URL-backed |
| `components/server-panel.tsx` | The live panel on a server's page |
| `routes/sandboxes.tsx` | The mod manager: what is in a sandbox, and whether it is applied |
| `routes/downloads.tsx` | The queue, and everything to do when one is stuck |
| `routes/rcon.tsx` | The server console |
| `routes/` | Browse, view, library, installs, account, settings panes |

## Authentication

**OAuth 2.0 Device Authorization Grant (RFC 8628) with PKCE.** The user signs in
on the real website in their real browser; the app never sees a password.

```
app                                    website-city
 │ POST /auth/device {client, challenge}
 │ ─────────────────────────────────────▶  AppDeviceGrant (PENDING)
 │ ◀───────────────────────────────────── {deviceCode, userCode, uri}
 │
 │ opens the SYSTEM browser at /login/device?code=XXXX-XXXX
 │                                          user signs in, approves
 │                                          appDevice.approve → APPROVED
 │ POST /auth/token {deviceCode, verifier}
 │ ─────────────────────────────────────▶  atomic APPROVED → CLAIMED
 │ ◀───────────────────────────────────── {accessToken, refreshToken, user}
```

Non-obvious properties, all load-bearing:

- **PKCE is not optional.** A stolen `deviceCode` is inert without the verifier,
  which never leaves the app's memory. A wrong verifier **destroys** the grant.
- **The grant is claimed atomically** (`updateMany` with `status: 'APPROVED'` in
  the WHERE). The app polls on a timer *and* on the deep link, so two claims
  racing is the normal case, not an edge case.
- **Refresh tokens rotate, and reuse kills the family.** A retired token coming
  back means two parties hold one lineage; every device is revoked and
  `reuseDetected` is set. `ApiClient::refresh` holds a mutex so the app never
  does this to itself.
- **The access token is memory-only.** One hour, treated as stale 60s early.
- **`tmc://` is a wake-up, not a credential.** Any app can claim a custom
  scheme, so nothing is read out of the URL — it only triggers an immediate
  poll.
- **System browser, never a webview.** An embedded login hides the address bar
  and the password manager, which is what makes it indistinguishable from
  phishing.

Website-side files: `src/server/auth/app-token.ts`,
`prisma/models/app-device.prisma`, `src/app/api/app/v1/auth/*`,
`src/server/api/routers/app-device.ts`, `src/app/[locale]/login/device/page.tsx`.

## The API contract

`website-city/src/types/app-api/contract.ts` is the **source of truth**;
`src/lib/api/contract.ts` is a **verbatim copy**. When the server side changes,
copy the file across — do not hand-edit the app's copy.

Every response is parsed through these schemas before it reaches a component, so
a drift is a loud error on the first request instead of an `undefined` three
screens deep. That is the entire reason the duplication is acceptable.

Why a separate REST API rather than the website's tRPC:

- The website's routers return **Prisma payloads** shaped by what a page renders.
  An installed app is not redeployed with the server, so its wire format has to
  be versioned and narrow.
- Every content type has a **different** row shape. The app draws one grid; the
  normalisation into `ContentSummary` has to happen once, server-side.
- Rust calls it too.

Endpoints: `/auth/device`, `/auth/token`, `/auth/refresh`, `/auth/revoke`, `/me`
(GET + PATCH), `/browse`, `/content/:kind/:id`, `/facets`, `/version`.

### The browse filters mirror the website's, deliberately

`BrowseQuerySchema` is a copy of `ServerBrowserPublicGetsInput` plus the
mod/asset equivalents, field for field, so a filter someone used on the site is
reachable here under the same name and with the same meaning. Where the two
could disagree, **the website wins** — `hideFull` drops servers declaring no
player limit because the site's SQL does, even though that is arguably wrong,
because a filter that quietly returns more than the site did is worse than one
that matches.

Two things this endpoint pins that the app cannot ask for, both server-decided
exactly as they are on the website:

- **`hasPenalties: false`.** A live penalty hides an item everywhere. Missing
  here, the app was the one surface still listing `SPY_FLAG`'d servers — the
  redirect farms and fake-player-count boxes. Applied to `/browse` *and*
  `/content/:kind/:id`, since reaching a row by id never passes a list filter.
- **`SRV_STALE_ONLINE_SEC`.** `online` has no expiry, so a server that stopped
  being scanned asserts its last reading forever.

Not every declared field is implemented: **`timeRange` is accepted and
ignored**, on this endpoint and always has been. The website windows aggregates
in-process over a capped candidate set, which an infinite-scroll cursor cannot
do — the ranking moves as the window moves, so rows repeat and vanish between
pages. `downloads` falls back to `createdAt` for the same reason.

## Live server queries

**The app talks to game servers itself.** The website can only show you what its
scanner last saw, from wherever the scanner lives; the app sends the game's own
query datagram from the user's device, so the player count is current and the
latency is the one that player will actually experience. A Sydney player and a
Frankfurt player see different numbers for the same box, and only the app can
tell either of them the truth.

### Where the protocol comes from

`App.srvQueryProtocols` on the website — the same list its own scanners use, so
there is no second source to drift. It arrives on every server row as
`server.query`, along with `timeoutMs` and two port fields.

### Which port gets queried, and why not the other two

**Explicit `queryPort` if the row has one, otherwise the GAME port.** That is
the whole rule, and the authority for it is the scanner that writes these rows —
`spy/internal/scanners/server.go`:

```go
qPort := port
if srv.PortQuery != nil && *srv.PortQuery > 0 {
    qPort = *srv.PortQuery
}
```

The two fields the API also sends are **not** inputs to that decision, and
treating them as such is what made every Source server look dead:

- **`portOffset`** (`App.srvGamePortQueryOffset`) is dead config. Grep the whole
  scanner for it and you get one hit: the struct field it deserialises into.
  Nothing reads it. A game whose App row carries a stale non-zero offset is
  scanned by the site on its game port, so probing `game + offset` from here is
  a port nobody is listening on — silence, which renders as a timeout.
- **`swapGamePort`** (`App.srvSwapGamePort`) is post-scan bookkeeping, not port
  selection. In `spy/internal/protocols/a2s.go` it means "once A2S_INFO comes
  back, read the true game port out of the extended-info block and swap the
  stored `port`/`portQuery`". Reading it as "query the game port" gave the right
  number often enough to look correct, which is worse than being wrong outright.

The only per-protocol exceptions are the scanner's own hardcoded ones, and they
apply only when the row has no explicit query port: **FiveM → 30120**,
**SCUM → game + 2**, **Frostbite → game + 22000**. `resolve_port` carries all
three with the Go file named. Frostbite's also declines the arithmetic above a
game port of 43535 rather than wrapping past 65535 — a wrapped port is not just
wrong, it probes a stranger's unrelated service on a low port number.

Getting this wrong is still the single most common reason a live server shows as
dead — which is why the rule is copied rather than invented.

`server.query` is null when the owner hid the network details or the game
declares no protocol. In both cases there is nothing the app is entitled to
probe, and it must not fall back to guessing.

### Implemented protocols

| Protocol | Games | Notes |
| --- | --- | --- |
| `A2S` | CS2, TF2, Rust, ARK, Garry's Mod, Squad | Challenge handshake + split-reply reassembly |
| `MINECRAFT` | 1.7+ | TCP handshake → status JSON |
| `MINECRAFT_SLP` | pre-1.7 | `0xFE 0x01`, UTF-16BE reply |
| `QUAKE3` | CoD, Wolfenstein, Xonotic, OpenArena | `getstatus` |
| `GAMESPY1` | Unreal Tournament, early Battlefield | `\status\` |
| `GAMESPY2` | Battlefield 2, UT2004 | Binary, per-section request byte |
| `GAMESPY3` | Minecraft's query port, many others | Signed challenge — see below |
| `GAMESPY4` | Games listing v4 | Identical game-server query to v3; shares its implementation, tagged separately |
| `SAMP` | SA-MP, open.mp | IPv4 only, by protocol design |
| `FIVEM` | GTA V, RedM | HTTP `/dynamic.json` + `/info.json` |
| `FROSTBITE` | BF3, BF4, Bad Company 2, Hardline | R-CON over TCP on game + 22000; roster columns keyed by tag name |
| `TEAMSPEAK3` | TeamSpeak 3 | ServerQuery on a fixed 10011; the game port SELECTS a virtual server rather than being connected to |
| `HYTALE_NITRADO` | Hytale on Nitrado | HTTPS status document on game + 3, self-signed |

### The four that stay on `TCP_ONLY`, and why it is not laziness

`DISCORD`, `SCUM`, `GTA_NETWORK` and `GTA_RAGE` are the last unimplemented
entries, and each is unimplementable *here* for the same reason:

| Protocol | What `spy` actually asks |
| --- | --- |
| `SCUM` | `api.hellbz.de` |
| `GTA_NETWORK` | `multiplayerhosting.info` |
| `GTA_RAGE` | the RAGE:MP master list at `cdn.rage.mp` |
| `DISCORD` | nothing — the handler is a stub |

None of them talks to the server. That is fine for a scanner reading a list
once on everybody's behalf, and wrong for this app three times over: **the
latency would describe that third party's hosting**, identical for every row
(`spy` says so itself in three comments, and records no latency for any of
them); it would **tell a third party every server a user scrolls past**; and on
a phone it would be one HTTPS request per row per tick. So they measure a real
TCP handshake to the real box instead, and their player counts come from the
API — which got them from the scanner, which read those lists once.

`net/query/hytale.rs`'s module header carries this, because it is the one of
the five that DID qualify.

Everything else in the enum is recognised but falls through to `TCP_ONLY`, which
still yields a real latency figure. A Server Live Query plugin can cover any of
them without an app update.

### Rules for anything under `net/query/`

These parsers read bytes from an unauthenticated machine that may be actively
hostile, and the release profile sets `panic = "abort"` — so **a panic in a
parser is a remote kill switch**. Consequently:

- **No indexing, no slicing.** Everything goes through `net::reader::Reader`,
  which bounds-checks every read and returns `Err` past the end.
- **Every length from the wire is checked against what remains.** A server
  claiming a 4 GB string is the normal case to handle, not an edge case.
- **Every loop is bounded** — fragments, entries, rules, roster length.
- **A truncated-input test is mandatory.** Each protocol has one that feeds
  every prefix of a valid reply through the parser; it must never panic.
- **`resolve_public` is the only way to get a `SocketAddr`.** Resolve once and
  connect to *that* — checking a hostname and reconnecting by name is a DNS
  rebinding hole, and the guard is what stops the browser being an SSRF tool
  against the user's own LAN.

Three protocol details that are easy to get wrong and are commented at the site:

- **A2S's `0x41` challenge.** Skip it and every modern server looks unreachable.
- **A2S's challenge is bound to the source PORT.** Use `net::transport::
  UdpSession` so the retry goes out on the socket that asked; a fresh ephemeral
  port is ignored by several server builds. It is also what makes split replies
  readable, since the continuation datagrams arrive unprompted on that socket.
- **GameSpy v3's challenge is a SIGNED decimal string.** Parse it unsigned and
  roughly half of all servers look unreachable.
- **A split A2S reply's `total` and `number` are two SEPARATE bytes.** GoldSrc
  packs them into one (index high nibble, total low) and has no split-size
  field; Orange Box and later use two bytes plus a size. Reading the packed form
  while also consuming a size field parses fragment 0 plausibly — `total = 2,
  index = 0` — and decodes every LATER fragment to index 0 as well, so they
  overwrite each other and every split roster comes out "incomplete". The
  authority is `go-a2s`, which is the library `spy` queries A2S with.

### The registry

`LiveQueryProvider` holds one timer and one registry. Cards register via an
`IntersectionObserver` while on screen; every tick sends the whole visible set
to Rust in a single `query_servers` call, which applies the user's concurrency
cap. Fifty cards each polling would be fifty timers, fifty IPC round trips and
fifty simultaneous sockets — which a phone's radio and most home routers handle
badly.

Deregistration has a 5-second grace so a flick-scroll does not cancel work
already in flight, and polling stops entirely while the window is hidden.

**The cadence is the user's**: `latencyIntervalMs`, one second by default, set
under Settings → App → Servers and clamped to 250ms–5min *in Rust*
(`LATENCY_INTERVAL_MS_MIN`/`MAX`) because it decides how often a third party's
game server is sent a datagram. The provider publishes it as `intervalMs` so
the single-server panel polls on the same number rather than on a constant of
its own. A tick that arrives while one is in flight is remembered, not stacked,
so a fast interval degrades to "as fast as the batch completes" rather than to
overlapping batches.

Three properties of the scheduling that are easy to undo by accident:

- **A row's FIRST probe comes from `watch`'s kick, not from the interval.** The
  provider mounts at app start, so its warm-up has long since fired by the time
  anyone opens the browser; keying first measurements off the refresh interval
  left a fresh screenful showing ellipses, which reads as the feature being
  broken rather than slow. It is also what keeps a slow interval usable: a row
  is measured a quarter-second after it scrolls into view whatever the setting.
- **The batch is sorted unmeasured-first.** Rust caps a batch at 64 and *drops*
  the rest, so an unsorted batch during a fast scroll spends the cap on rows
  that already have a number and strands the ones that do not.
- **A failed `query_servers` call is applied as a failed probe for every row in
  it.** Otherwise a broken IPC boundary — an unregistered command, a drifted
  schema — is indistinguishable on screen from a browser full of slow servers.

### How a latency reading is drawn

| State | Shown | Meaning |
| --- | --- | --- |
| measured | `32ms`, coloured | <90 green · <150 yellow · <200 orange · ≥200 red-orange |
| timeout | `TO` in red | A probe went out and nothing came back |
| waiting | `…` | No probe has resolved yet |
| none | `—` | The owner hid the address; there is nothing to probe |

The ladder lives in `LATENCY_TIERS` / `latencyTone` and the four states in
`latencyState`, both in one place so the card, the table and the server panel
cannot disagree. `TO` exists because "500ms" and "no answer" are different facts
and a dash for both is how a server browser earns a reputation for lying.

### Latency history

`net::latency::LatencyStore` keeps up to 120 samples for up to 512 servers, in
memory only — the series is interesting while the browser is open, is entirely
reconstructible by re-measuring, and persisting it would mean a disk write per
scroll tick. That is a COUNT, not a window — at the default one-second cadence
it is two minutes of history, and proportionally more at a slower one.
**Failed probes are recorded as `None`, not dropped**: an intermittently-dead
server must not graph as perfectly stable.

Graphs are hand-rolled SVG (`components/latency-graph.tsx`). No chart library:
these render once per card in a scrolling grid, and a general-purpose chart's
layout pass and scales get paid fifty times for a sixty-point line with no axes.
A dropped probe **breaks the line** rather than interpolating across it, and the
Y scale has a 40ms floor so a rock-steady server does not render as a mountain
range.

"Sort by ping" in the server browser is client-side and could only be: latency
is a fact about this device's route, so the API cannot order by it.

## The plugin system

**A plugin is data, never code.** No script engine, no WASM, no `exec` step.
An installer describes file operations; a query plugin describes a datagram and
how to read the reply; a theme describes colour tokens. Everything a plugin can
express is something this crate implements and audits.

That is a real limitation and it is the point: the alternative is third-party
code running with full filesystem access next to the user's saves and
credentials, and nothing bolted on afterwards recovers from that.

### Three types

| Type | Declares | Runtime |
| --- | --- | --- |
| **Installer** | Ordered `install` / `uninstall` steps | `plugins/steps.rs` |
| **Server Live Query** | A hex request + a field list | `plugins/query.rs` |
| **Theme** | A map of CSS custom properties | `plugins/theme.rs` |

### The step vocabulary

`download`, `extract`, `copy`, `move`, `mkdir`, `remove`, `writeText`,
`patchJson`. **Adding a step that runs a program, resolves an absolute path, or
reads an environment variable breaks the model** — do not.

Every path is a `PathRef { root, path }` where `root` is one of `gameDir`,
`pluginData`, `downloads`. The type cannot express an absolute path.

**`path` is relative to the ROOT, not to any grant.** A plugin granted
`{gameDir, "mods", write}` writes to `{ root: gameDir, path: "mods/foo.jar" }`.
Grants are *filters* over the resolved path, so several narrow grants under one
root compose as a union and stay narrow — the alternative, anchoring each grant
at its own subdirectory, cannot say which grant a `PathRef` meant and used to
merge them into the widest one.

### The defences, and what each one catches

| Control | Catches |
| --- | --- |
| Lexical rejection in `join_relative` | `../`, absolute paths, drive prefixes, UNC, NUL, trailing dot/space (Windows), `:` (ADS) |
| Post-join containment | Component sequences that combine badly |
| Canonicalised deepest existing ancestor | A symlink already on disk pointing out of the jail |
| `enclosed_name` + re-check on zip entries | Zip-slip |
| Tar link/special-file refusal | `link → /etc` followed by `link/passwd` |
| Extract byte + entry caps | Zip bombs |
| Streaming download counter | A server lying in `Content-Length` |
| `https`-only + host allow-list, re-checked after placeholder substitution | A template smuggling in a host |
| SHA-256 verification, file deleted on mismatch | A tampered download reaching a later step |
| `is_public()` in `net/addr.rs` | SSRF into the LAN or a cloud metadata endpoint |
| Theme token + colour allow-list | `url()` beacons, `display:none` on the uninstall button |
| Grants as filters over a root-relative path | Two grants for one root merging into the widest |
| Manifest fingerprint | An update silently widening permissions |
| Ed25519 signature over the canonical manifest | An anonymous bundle, when `requireSignedPlugins` is on |

### Approval

`plugin_inspect` returns a fingerprint (SHA-256 of the canonical manifest), the
full permission list and the signature state; `plugin_approve` takes the
fingerprint back. That closes the window between the user reading the
permissions and clicking approve.

`Registry::rescan` runs every launch and re-hashes each installed manifest. A
drifted plugin is **disabled** and flagged `needsReapproval`; the toggle refuses
to re-enable it, because the toggle is not where permissions are shown.

### Signatures

The fingerprint answers *"is this the same plugin the user approved?"*. It
cannot answer *"did anybody the user trusts write it?"* — a hash of whatever is
in the folder is perfectly good for a bundle that anything at all dropped there.
`plugins/signature.rs` is the second question: Ed25519 over the manifest's
**canonical bytes**, the same serialisation the fingerprint hashes, so a bundle
reformatted in transit keeps its signature and a bundle whose declared
permissions changed loses it. The detached signature is `plugin.sig`, hex.

**Three states, not two.** `Unsigned`, `Trusted(keyId)` and `Untrusted` — the
last being a signature that matches nothing the user trusts, which is either a
publisher whose key has not been added yet or a tampered bundle. Folding it into
"unsigned" is how the interesting case disappears, so the UI and the refusal
messages keep them apart.

**The trust store is the user's, and no key is compiled in.** Keys live in
`plugins/trusted-keys.json`, and TMC's own publishing key will be a row in it
like anybody else's — the honest shape while there is no registry to distribute
plugins through, and still correct once there is. Bundling a key nobody signs
with would make `requireSignedPlugins` a switch that refuses everything.

**`requireSignedPlugins` now does something.** It was a stored boolean with
nothing behind it, which is worse than not having it — somebody turns it on,
believes they are protected, and installs accordingly. With it on, `active()`
refuses an unsigned or untrusted plugin before the executor can be reached.

Two properties of the gate that are easy to lose:

  * **It re-verifies rather than reading `record.signature`.** The stored verdict
    is a claim about a check that ran at some earlier time under a trust store
    that may since have changed, and this is the last gate before something
    writes to a game folder.
  * **Trusting a key rescans immediately.** "Add the publisher's key" is advice
    that has to work when followed, not after a restart — and removing a key has
    to stop those plugins at once, which is the entire reason to remove one.

**Producing one** is `cargo run -p tmc-core --example plugin-sign` — `keygen`,
`sign <dir> <secret>`, `verify <dir> <public>`. It exists so the feature is
usable without reimplementing it from a doc comment, which would mean guessing
at the one detail that matters. Its two properties are worth checking by hand
after any change to `Manifest`: a manifest minified and key-sorted still
verifies, and one extra `gameDir` write grant does not.

What a signature buys is bounded and stated in the module header: it says a
holder of that key produced *these* permissions and *these* steps. It says
nothing about whether they are safe. A signed plugin is confined by exactly the
same jail as an unsigned one.

Examples live in `examples/plugins/` and are **validated by two Rust tests**:
one parses every manifest, the other builds each installer's real jail and
resolves every step path through it. The second is the one with teeth — parsing
only proves the JSON is well-formed, while a manifest whose grants and step
paths disagree is the mistake an author copying the example would inherit.

### App plugins: the rules for one game

Distinct from the bundles above, and deliberately:

| | Registry plugin | App plugin |
| --- | --- | --- |
| Where | `plugins/<id>/plugin.json` | `plugins/app/<slug>/*.json\|yaml` |
| Identified by | a reverse-DNS id the author picks | the GAME it handles |
| Answers | "what can this plugin do?" | "where do this game's mods go, and how do its sandboxes work?" |

A bundle is something a user installed; an app plugin is a *rule for a game*,
the app ships one per supported game, and there are dozens of tiny ones. The
directory name is the game's **URL slug**, lower-cased. `disabled/` is skipped
at any depth, which is the whole mechanism for turning a rule off without
deleting it.

| File | Declares |
| --- | --- |
| `manage_mod.json` / `manage_asset.yaml` | Install and uninstall steps for one content kind |
| `launch.json` | How to start the game |
| `sandbox.json` | Deployment strategies, presets, the game's own options, and detection hints |

**The safety model is unchanged.** Each file compiles to a synthetic `Manifest`
and runs through the same jail and the same executor, so it can express nothing
a registry plugin cannot. What it adds is only the *selection* — which rule
applies to which game, kind and file.

## The library

The device's answer to the account's subscriptions: which of them this machine
holds, which are on disk, and how they got there.

**SQLite, not a JSON file** (`library/db.rs`). Everything else the app persists
— settings, the plugin registry — is a small file rewritten whole, and that is
right for those: read at launch, written on a click, never contended. The
library is not like that. The sync loop writes it on a timer while the UI reads
it on every render and the installer writes single rows mid-install, and a
whole-file rewrite under that pattern loses one writer to another exactly when
somebody is watching a progress bar.

Schema changes are **stepwise** — each migration takes the database from `n-1`
to `n` and runs only if it has not. Re-running one batch happens to be safe
today because every statement is `IF NOT EXISTS`, and stops being safe the first
time a step needs an `ALTER TABLE`.

**It polls** (`library/sync.rs`), because a subscription can be created in a
BROWSER and there is no push channel to an installed desktop app that is
reliable across three desktop platforms, two mobile ones, corporate firewalls
and sleeping laptops. A watermark makes that cheap: each response carries a
`revision`, the next request sends it back, and a device open for an hour has
made sixty requests and transferred one row.

**A full sync still happens** every `FULL_SYNC_EVERY` passes and always on
launch, because a delta cannot express a DELETION — there is nothing left to
poll. Anything the device holds that the server did not send has been
unsubscribed elsewhere.

**An upsert from the server never touches the device-local columns.** That is
what stops a resync forgetting what is installed, which is the single easiest
way to turn a working library into an endless reinstall loop.

**The plan is executed in Rust, not returned to the webview.** A frontend that
decided what to install is one an injected script can talk into installing
something.

## Sandboxes, and how mods reach the game

A **sandbox** is what other managers call a profile (Mod Organizer, Vortex) or
an instance (CurseForge, r2modman): a named set of mods with its own load
order, its own deployment method and its own launch settings.

> The word is why `plugins::jail` is called a jail. It used to be
> `plugins::sandbox::Sandbox` — the path jail — and two `Sandbox` types in one
> crate, one of them a security boundary, is a mistake waiting for somebody to
> reach for the wrong one.

**On the wire a sandbox is an `AppInstall`.** The cloud model predates the word
and `/api/app/v1/installs` is shipped, so renaming the endpoint would 400 every
request from every installed copy of the app. The app says "sandbox"; the API
says "install"; they are the same thing.

### What lives where

By the same test the settings use — *would this be wrong to apply on a
different machine?*

| Fact | Where | Because |
| --- | --- | --- |
| name, mods, order, options, launch flags | the account (`AppInstall`) | signing in on a second machine should reproduce it |
| which folder it deploys into | the device | that path exists on one machine |
| which release of each mod is staged | the device | so does that |
| the deployment ledger | the device | it describes files on one disk |

The cloud half is **optional per sandbox**. `cloudSync` off means it is never
sent anywhere and a reconcile cannot delete it — which is the whole answer to
"I do not want my mod list on your server", and it has to be per-sandbox rather
than global because the useful case is "sync my Minecraft profiles, not the one
I use for testing".

### Two steps, not one

```text
  stage   — run the game's install rule with the sandbox's own staging folder
            standing in for the game directory
  deploy  — mirror every staged file into the real game folder, by whichever
            mechanism the sandbox is set to
```

Splitting them is what makes the whole feature work:

  * **Every install rule written before sandboxes existed still works.** A rule
    that copies to `mods/{fileName}` writes to
    `<staging>/<sandbox>/<mod>/mods/foo.jar`, and deployment puts
    `mods/foo.jar` in the game folder. The rule never learns which strategy is
    in use, and it should not — "where does a Minecraft mod go" and "how do
    files reach the game folder" are different questions.
  * **Switching sandboxes is a link operation, not a download.** Staging
    survives an undeploy.
  * **Nothing half-downloaded is ever in the game folder.**

`pluginData` is scoped per sandbox and per mod (`jail_for_scoped`). Two
sandboxes staging the same mod run identical steps with an identical
`{fileName}`; a shared scratch directory meant the second run's download landed
on the first's, and a sandbox pinned to an older release quietly got the newer
file.

### The four strategies

| Strategy | Game folder | Cost | Fails when |
| --- | --- | --- | --- |
| `direct` | modified | a full copy | never; it is the fallback |
| `hardlink` | link pointers | nothing | staging is on another drive |
| `symlink` | link pointers | nothing | Windows without Developer Mode |
| `usvfs` | untouched | nothing | not Windows, or not built with `usvfs-hooks` |

#### USVFS: the one that deploys nothing

Mod Organizer's approach, in `src-tauri/usvfs/` (`tmc-usvfs`): the mods stay in
staging and the game is *told* they are there, by a DLL injected before its
entry point runs that patches `CreateFileW` and `GetFileAttributesW` in its
import table and answers them from a merged in-memory tree.

```
  app                              game process
  ───                              ────────────
  build the merged tree
  publish it            ──blob──▶
  launch suspended
  inject the hook DLL   ──────▶    DllMain → read the tree, patch the IAT
  resume                ──────▶    every open of a virtual path is answered
                                   from staging
```

**Deploying is not a file operation**, and everything downstream follows from
that:

  * the merged tree is serialised to a blob inside the sandbox's own staging
    folder, and **that publish IS the deploy**;
  * **the ledger comes back empty**, which is the correct record rather than a
    gap — it names files in the game folder and there are none, so `purge`
    correctly does nothing and `verify` correctly reports a healthy folder;
  * **a sandbox switching TO it still purges what the last strategy left**,
    which is the one write to the game folder a virtual deploy performs. Without
    it the user gets both: every file twice, with the virtual copy winning only
    where the tree happens to cover it;
  * **there is no fallback in either direction.** Whether the game folder is
    modified at all is the reason somebody picks this, so silently linking
    instead would dirty a folder chosen to stay clean, and silently going
    virtual would leave a game that was never told about the mods.

**The revision is a content hash, not a counter or a clock.** The injected DLL
compares it to notice that a blob it already read has changed; a counter would
also tick for a redeploy producing an identical tree.

**It is gated twice, at deploy and at launch** (`deploy::usvfs_unavailable`,
`spawn::launch_with_vfs`), on Windows AND on `tmc-core`'s `usvfs-hooks` feature,
which is **off**. The two are halves of one decision: a build with only one on
could publish a tree no launch would carry, or inject with nothing to inject.
The refusals are separate strings because they are separate facts — "this is
Windows-only" can never change for a Mac user, and "this build has it off" can.

Why the gate exists at all is a confidence split, stated in `usvfs/src/lib.rs`:

| Half | State |
| --- | --- |
| `tree` — the merged view, resolution, listings, case rules | **Tested**, on every platform |
| `shm` — publishing and reading the blob atomically | **Tested**, including every malformed input |
| `hooks` — IAT patching, the redirecting `CreateFileW` | **Type-checked against `x86_64-pc-windows-gnu`. Never run against a game.** |
| `inject` — suspended launch, remote `LoadLibrary` | **Type-checked.** Argument quoting and environment building are tested; the launch is not |

`cargo check -p tmc-usvfs --target x86_64-pc-windows-gnu` is how that
type-checking happens and it is worth keeping working — it found four real bugs
the first time it ran, including `IMAGE_NT_HEADERS64` living in
`Diagnostics::Debug` while being gated behind the `Win32_System_SystemInformation`
feature. `tmc-core` cannot be cross-checked the same way (bundled SQLite needs a
C cross-compiler), which is exactly why the injector is its own dependency-light
crate.

**What it cannot do, even when it works:** calls through `GetProcAddress` are
not redirected (the pointer never came from an import table), direct `ntdll`
syscalls are not redirected, writes are not redirected, and every kernel-level
anti-cheat treats injection as an attack — correctly. A game declaring
`antiCheat: "kernel"` in its `sandbox.json` is refused this strategy before it
reaches the engine.

**A URI launch rule and a virtual deploy are refused together.** A game started
through `steam://` is started by Steam, and there is no process of ours to
inject into; the alternative starts the game with none of the sandbox's mods and
reports success, which is the hardest kind of bug to diagnose because the folder
is stock and there is nothing to find.

**Capability is probed, not assumed.** `deploy::link::probe` creates one file
and links it, twice, and reports what worked. Every rules table for this is
wrong somewhere — a Linux box with staging on an exFAT USB drive, a macOS
volume with links disabled, a Windows machine with Developer Mode on.

**Falling back between the two LINK strategies is automatic and reported.
Falling back to copying is not.** Copying is not a worse link, it is a different
decision: it writes gigabytes, it modifies the game's own files, and its failure
mode is a game folder that needs restoring rather than unlinking. A sandbox that
asked for links and can have neither gets an error naming both fixes.

### Three properties that hold across every strategy

  * **Nothing is overwritten.** A file already in the game folder that the app
    did not put there is MOVED to the backup store before its place is taken,
    and a purge puts it back.
  * **Nothing is removed unless it is still ours.** Every removal re-checks the
    file against the ledger row that claims it — a symbolic link must still
    point at its staging file, a hard link must still share its identity, a copy
    must still have its recorded size and mtime. Anything else is left alone and
    reported, so a config the user edited after deploying survives.
  * **Staging is never modified.** Deployment only ever reads from it, which is
    what lets two sandboxes share one downloaded copy of a mod.

### The ledger

`deploy::ledger` is the only record of which files in somebody's game folder
belong to us, and undeploying by rescanning cannot replace it: a rescan can see
that `Data/textures/sky.dds` exists, and cannot see whether the app put it there
or whether it shipped with the game. Guessing wrong either strands a modded file
forever or deletes a base-game asset.

It is written even when a deploy reported errors — a partial deploy put files on
disk, and losing the record of them creates exactly the orphans the ledger
exists to prevent.

### Conflicts

The merge tree decides one winner per path before anything touches the disk, by
priority, with ties broken on the mod key so the answer is stable across runs.
Losers are **reported**, never merged: nothing here understands any game's file
formats, and a manager that silently produces a file neither author wrote is how
"it works for me" bug reports are made.

Sorting happens inside `merge::build`, not in the caller. A tree built from an
unsorted list is wrong in a way that looks right until somebody reorders their
list and nothing changes.

### `sandbox.json`

The PDF blueprint's "strategy matrix", plus the two things it left implicit:

```json
{
  "manifestVersion": 1,
  "sandbox": {
    "deploy": {
      "defaultStrategy": "symlink",
      "supportedStrategies": ["symlink", "hardlink", "direct"],
      "antiCheat": "kernel",
      "modTargets": [{ "type": "loader_mod", "relPath": "mods" }],
      "notes": ["Shown verbatim in the sandbox's settings"]
    },
    "presets": [{ "id": "fabric", "label": "Fabric", "loader": "fabric" }],
    "options": [{ "key": "memoryMb", "label": "Memory", "type": "int", "max": 65536 }],
    "detect": { "steamAppIds": ["271590"], "markers": ["GTA5.exe"] }
  }
}
```

  * **`options`** is a *form description* and nothing more. It cannot express a
    condition, a computation or a dependency between fields — a settings schema
    that can do those is a program, and the plugin model rests on plugins not
    being programs. A game needing one needs a second preset instead.
  * **Values are clamped against the schema on every write**, and a key the
    schema does not declare is dropped. The options reach the cloud and come
    back, so "the server said 900 GB of heap" is answered here rather than by a
    game that will not start.
  * **`antiCheat: "kernel"`** is load-bearing. EAC, BattlEye and Vanguard all
    watch for a game folder whose files are not where they should be, so a game
    declaring it gets Direct as its default and a warning on every other option
    — the cost of guessing wrong is somebody's account, not a failed install.

Options reach a command line only through `optionArgs` in the game's own
`launch.json`. `LaunchOptions` carries them in a flattened `extra` map; an
object or an array contributes nothing, and there is still no shell.

## Downloads

Every file the app fetches goes through `download::DownloadManager` — a mod's
release archive, a sandbox's staging, whatever a plugin's `download` step names.
It is a subsystem rather than a function because a modpack is not one download,
it is four hundred, over a home connection, on a laptop that will be closed
halfway through.

```text
  enqueue ──▶ queued ──▶ running ──┬──▶ done
                 ▲         │       ├──▶ failed ──▶ (retry) ──▶ queued
                 └─────────┴───────┴──▶ paused ──▶ (resume) ──▶ queued
```

Two guarantees worth naming:

  * **The destination filename only ever appears after the last byte and the
    checksum.** Progress lives in a `.tmcpart` file beside the target, so
    nothing downstream can pick up a partial file and treat it as complete.
  * **A `200` in answer to a range request means the server ignored it**, so
    the file restarts rather than being appended to. Appending produces a file
    that is too long, passes every length check, and fails its checksum with an
    error nobody can explain.

**Downloading one named release goes through the queue too.** An item's page
lists its version history, and both its buttons — the header's "Download 1.4"
and each row's — used to hand the URL to the system browser. That is a strange
thing for an app whose whole download story is the queue being bypassed, and
somebody fetching an older release because the newest one broke their save is
exactly the person who wants it pausable. `download_release` takes a kind and
two ids, **never a URL and never a path**: Rust re-fetches the detail, picks the
file, and puts it in the user's download folder. So the rule at the top of
`commands/downloads.rs` still holds — there is no `download_start(url, path)`,
and nothing an injected script could point at a file of its choosing.

**A plugin's `download` step goes through the queue** when the executor has a
handle to one, with a deterministic id derived from the plugin and the
destination — so re-running a failed install is the same row rather than a
second writer for one file. The host allow-list, the size cap and the checksum
are enforced on both paths; the queue moves where the bytes are read, not which
checks run.

**The queue gets its own HTTP client, not the API's.** The API client attaches a
bearer to everything it sends, and a mod archive comes from a CDN that has no
business seeing one — a redirect to a third-party mirror would hand out an
access token.

Bandwidth is a token bucket (`download::rate`), composed as global × per
download, both live-adjustable. A request larger than the bucket can hold is
served by taking the balance negative rather than refused, because a downloader
whose chunk size it does not control would otherwise hang.

The device reports a SNAPSHOT of its queue to `/api/app/v1/downloads` so the
website can show it. That row never drives anything — there is deliberately no
endpoint that tells a device to pause or cancel — and it carries no URLs and no
filesystem paths.

## Finding the games

`detect::scan` reads what the launchers already wrote down rather than asking
somebody to type a path.

| Source | Reads |
| --- | --- |
| Steam | `libraryfolders.vdf`, then every `appmanifest_*.acf` |
| Epic | `Data/Manifests/*.item`, plus Heroic/Legendary on Linux |
| GOG | `galaxy-2.0.db`, and the plain `GOG Games` folder |
| Xbox, Ubisoft, EA, Battle.net | their fixed folder layouts, one level deep |
| a game's own hints | `sandbox.json`'s `detect.paths` |

**Detection suggests; it never configures.** Applying a result goes through
`anchor::validate_root` exactly as a hand-typed path does — a folder is not more
trustworthy for having been found automatically, and a game directory is a jail
anchor.

**It reads no environment variable.** The platform directories arrive as
`DetectRoots`, filled by the Tauri crate from its path resolver.

Matching a folder to a TMC game, in descending order of confidence: the
launcher's own id (exact, language-independent, survives a rename), then a
marker file, then the normalised display name. A name match must also find the
marker; an id match is not second-guessed.

Two details that are easy to get wrong and are commented at the site: Steam's
`installdir` is NOT the display name (`Grand Theft Auto V` lives in `GTAV`), and
GOG's database is opened `mode=ro&immutable=1` so a running Galaxy neither
blocks the read nor hands back a torn one.

## RCON

`rcon::` speaks two dialects: **Source** (which is also Minecraft's, Rust's,
ARK's, Squad's and Palworld's) and **Frostbite** (Battlefield 3/4/Hardline/BC2).
Both parse through `net::reader` and both have the truncation test every parser
in this crate has.

Three protocol details that are easy to get wrong:

  * **A Source multi-packet reply has no end marker.** After the command, a
    second empty packet with a different id is sent; the server answers in
    order, so its echo is the end. Stopping at the first packet truncates
    `status` on a full server; waiting for more hangs on every short reply.
  * **A successful Source auth sends TWO packets**, in an order implementations
    disagree about, so the handshake reads until it sees the auth reply rather
    than counting.
  * **Frostbite's `size` counts the two header fields it is part of.** Omitting
    them leaves four bytes of each packet in the buffer, which presents as "the
    second command always fails".

### The one deliberate SSRF gap

Everywhere else an address goes through `net::addr::resolve_public`. **RCON does
not**, and the module header says so outright: `192.168.1.10` and `127.0.0.1`
are the *normal* answers here, because the feature is "administer my server" and
most people's server is on their own network. A guard that refused them would
refuse the feature.

Bounded instead by a per-host connection cooldown, a session cap, and a
Security-level audit entry on every connect. It remains a widening and it is
listed as one below.

### The passwords

Encrypted on the device with XChaCha20-Poly1305, key in the OS credential store
(`crypto::LocalCipher`), and **never sent to the website**. A server password is
not derived from a TMC account, losing it costs somebody their server rather
than their profile, and no feature on the site needs it.

What that buys is stated honestly in `crypto.rs`: it answers a **copied file** —
a backup, a synced folder, a disk pulled out of a laptop. It does not answer
code running as the user in this session, and nothing on any desktop platform
does. What the design guarantees instead is that the webview is not such code:
`rcon_secret` is `pub(crate)`, `rcon::exec_saved` is its only caller, and
`RconServer` has no password field for a refactor to start returning.

## Deep links

`tmc://` is registered on desktop and declared in the mobile manifests. One rule
governs all of it:

> **A link can ask the app to SHOW something. It can never ask the app to DO
> something.**

Any program on the machine can claim a custom scheme and any web page can
navigate to one without a click, so a link is an untrusted request from an
unknown party and the most it achieves is a screen with a button on it.

| Link | Does |
| --- | --- |
| `tmc://auth` | Polls the API now instead of waiting out the interval. Carries nothing |
| `tmc://install/<kind>/<id>` | Opens that item's page with the install controls ringed |
| `tmc://view/<kind>/<id>` | Opens that item's page |
| `tmc://sandbox/<id>` | Opens one sandbox |

`tmc://install/mod/1234` is the **guest** flow: a signed-in user gets a
subscription, which follows them to their other devices and keeps the mod
updated, but a guest has no account to hang one on.

Everything else parses to nothing, and an unrecognised link does nothing at all
— it does not fall through to the auth wake-up, because a default action is a
default action an attacker gets to trigger. `deeplink.rs`'s test names the
shapes that must never start working: `tmc://deploy/1`,
`tmc://settings/gameDir?path=/`, `tmc://rcon/1/exec?command=quit`.

## Settings: the two halves

| | App settings | Account settings |
| --- | --- | --- |
| Stored | `settings.json`, this machine | `UserSettings` on the website |
| Written by | `settings.rs` | `PATCH /api/app/v1/me` |
| Read by | `useSettings().app` | `useSettings().user` |
| Contains | Theme, scale, game directories, logging, latency, download limits, plugin prompts | Notifications, locale, timezone |
| Can fail | No | Yes — network, auth |

The test for which side something belongs on: **would this be wrong to apply on
a different machine?** A game directory would be. A notification preference
would not.

### Two of them are not settings at all

`gameDirs` and `downloadDir` anchor the plugin jail — `plugins::jail`
resolves every `PathRef` beneath them, so an installer holding
`{gameDir, "", write}` can write anywhere below. They are therefore the only
fields with their own commands:

- **`settings_patch` refuses them** (`JAIL_ROOT_FIELDS`), loudly rather than
  by dropping the key. The refusal is in `SettingsStore::patch`, so it holds for
  every caller and not just the one command.
- **`settings_set_game_dir` / `settings_set_download_dir`** run
  `anchor::validate_root`, which rejects a drive root, a system directory, and
  anything that *contains* the app's data, logs, cache, plugins or the user's
  home. A jail anchored above the app's own files would enclose `settings.json`
  and the plugin registry — a plugin that can rewrite the registry can grant
  itself permissions.
- The **canonical** path is what gets stored and what gets audited, so the value
  in `settings.json` is the one the jail will resolve to later.
- **A sandbox's own `gameDir` goes through the same validator**, on
  `sandbox_patch`. It gets no more trust for having arrived on a different
  command.
- Both are audited at **Security** level, so turning logging off cannot hide a
  change to where plugins may write.

What this does *not* establish is that a human chose the path. That came from
the OS dialog and is gone with it; the anchor's **identity** is what is checked
now, not its provenance. `anchor.rs`'s header says so plainly — don't let a
later comment upgrade the claim.

## Logging

`Settings → Logging` reads an append-only JSONL audit trail. It is a **record of
what the app did on the user's behalf**, not diagnostics — `tracing` handles
those and goes to stderr.

- `Security` level entries are written **even when the user turns logging off**.
  A switch that lets a plugin ask the user to stop watching is not a control.
- Clearing the log writes the "log cleared" entry **after** the truncation.
- Every plugin step, every download URL, every jail refusal, every permission
  grant lands here.

Write with the `audit!` macro. Never put a token or a credential in `data`.

## Cross-platform

One layout, keyed on the **window**, not the OS:

- **≥ 768px** — sidebar rail
- **< 768px** — bottom tab bar

A desktop window dragged narrow gets the phone layout, which is both correct and
the only way to test the mobile shell without a device.

- Safe-area insets come from `env(safe-area-inset-*)`, wired through
  `--safe-top` / `--safe-bottom` in `app.css`. Use them on anything fixed to an
  edge.
- **HashRouter**, because a Tauri build is a static bundle with no server to
  answer `/view/mod/12` on reload.
- The build target is `safari13` (`chrome105` on Windows) — the floor for the
  oldest supported macOS and Android webviews.
- `keyring` is desktop-only; `secure.rs` falls back to the app's private
  container on mobile, which the OS isolates and encrypts.
- `tauri-plugin-deep-link` is registered on desktop only. Mobile declares
  `tmc://` in the platform manifests generated by `tauri android init` /
  `tauri ios init`.

### The window frame is the app's, not the OS's

`decorations` is **off** in `tauri.conf.json` and `components/titlebar.tsx`
draws the frame. The reason is Linux: Tauri's backend there is wry → WebKitGTK,
so a decorated window gets a GTK titlebar themed by whatever desktop the user
runs — and there is **no Qt backend** to switch to. The only way a Tauri app
stops looking like a GTK app is to stop letting GTK draw any of it.

That decision cascades, and each piece is load-bearing:

- **Dragging is `data-tauri-drag-region`, never `-webkit-app-region: drag`.**
  The CSS property is a Chromium extension: it works in WebView2 on Windows and
  does nothing at all in WebKitGTK or WKWebView.
- **Resizing is eight fixed strips** calling `startResizeDragging`. With
  decorations off the window manager no longer offers a grab border, so without
  them the window cannot be resized at all.
- **Six `core:window:*` permissions** are in `capabilities/default.json` for
  exactly this. They act only on the main window and carry no data.
- **`tauri-plugin-dialog` is deliberately not registered.** Its folder picker is
  the GTK file chooser on Linux; `components/folder-picker.tsx` replaces it.
- **Native form controls are reset in `app.css`.** A GTK combo box, an Aqua
  select and a Fluent one are three shapes for one screen — the same mismatch
  the frame removes, arriving through a different door.

## Commands

```bash
npm install                # frontend deps (needs GitHub Packages auth for @modcommunity)
npm run dev                # vite only, no Rust — fastest loop for UI work
npm run desktop            # tauri dev
npm run desktop:build      # tauri build

npm run android:init       # once, generates gen/android
npm run android            # tauri android dev
npm run ios:init           # once, macOS only
npm run ios                # tauri ios dev

npm run check              # everything. Run before declaring done
npm run check:web          # tsc + eslint
npm run check:rust         # clippy -D warnings, whole workspace
npm run test:rust          # cargo test, whole workspace
npm run test:core          # tmc-core only — no GTK stack needed, runs anywhere
npm run fmt                # prettier + cargo fmt

npm run contract:sync      # re-copy the API contract from website-city
npm run sysroot            # build the local GTK sysroot (see below)
npm run shared:local       # install @modcommunity/shared from ../tmc-global
npm run shared:build       # rebuild it in place
```

### Pointing a build at the dev site

```bash
TMC_API_BASE=https://tmcdev.net:3002 npm run desktop   # debug: read at RUN time
TMC_API_BASE=https://tmcdev.net:3002 npm run android   # baked in at BUILD time
```

`tmc_core::api::api_base()` resolves the base once per process, from
`TMC_API_BASE` in the environment (**debug builds only**) and otherwise from
`TMC_API_BASE` at compile time, falling back to production. `core/build.rs`
carries the `rerun-if-env-changed` that makes the compile-time half honest —
without it a rebuild keeps the base the binary was FIRST built with.

Four properties, none of them incidental:

- **It is never a setting and never an argument from the webview.** There is no
  `api_set_base`; `api_env` is read-only. A "which server?" field is a phishing
  primitive — point the app at a look-alike and it sends that host a bearer
  token — and the same is true of a field a script in a mod description can
  reach.
- **A release build has no runtime path at all.** The environment of the
  process that launched the app is not a trust boundary: a `.desktop` file, a
  shortcut's "Start in", an installer's launch step all set one.
- **The value is validated, not trusted.** Scheme and host only — no path, no
  query, no credentials — reduced to an origin by `Url::origin`, and `http` is
  refused for anything but a loopback or LAN host. A refused override logs and
  falls back to the built-in base, because "my dev server saw no traffic" is a
  better failure than "my token went to a host I fat-fingered".
- **Each base gets its own stored session.** `secure.rs` keys the refresh token
  on `api_base_scope()`, so a dev run neither reads nor overwrites the
  production one. Sharing the entry meant the real refresh token was sent to the
  dev server on its first refresh — and a rejected refresh is terminal, so it
  also signed the developer out of the live site.

A non-production base is **shown** (title bar badge, Settings → App →
Development, and the sign-in copy names the real host) and **audited** at
Security level on launch. Nothing else on screen distinguishes staging from
production, which is how a screenshot of dev data becomes a bug report about
live data.

The OS hand-off (`tauri-plugin-opener`) stays scoped to `https://*`, so with a
plain-`http` local base the app can browse and query but cannot open the login
page or an article in the browser. The device screen prints the URL for exactly
that case.

### Building on Linux without root

The Tauri target needs `webkit2gtk-4.1`, `libsoup-3.0`, `libgtk-3` and
`libdbus-1` headers. Where you have root:

```bash
sudo apt install pkg-config build-essential \
  libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev libgtk-3-dev libdbus-1-dev
```

Where you do not — a shared dev box, a CI image, a container — `npm run sysroot`
builds a private one instead. `apt-get download` needs no privileges and
`dpkg -x` unpacks anywhere:

```bash
npm run sysroot                  # ~35 MB into ~/.local/tmc-sysroot
source ./scripts/linux-env.sh    # point pkg-config and the linker at it
npm run desktop
```

`scripts/cargo.sh` — which `check:rust` and `test:rust` go through — sources
that automatically when the system headers are absent, so the same npm script
works in both situations. The sysroot supplies headers and link stubs only;
runtime libraries still come from the system where it has them.

## Working here

### Type safety

- **`any` is banned** and enforced by eslint. `unknown` plus a parse is always
  available.
- **Every IPC call names a schema.** `invoke` returns whatever the caller claims;
  `src/lib/ipc/call()` makes that claim checked. Importing `invoke` anywhere
  else is an eslint error.
- **Every API response is parsed.** Same reason.
- `noUncheckedIndexedAccess` is on. A lot of this code indexes into
  plugin-supplied data.
- Floating promises are an error — a swallowed IPC rejection is an install that
  silently reported nothing.

### Rust

- `AppError` is the only error type crossing IPC. It serialises as
  `{ code, message }`; `code` is stable, `message` is for humans.
- `AppError::Internal`'s `Display` is a fixed sentence. Detail goes to
  `detail()` and the log, never to the webview.
- Prefer returning an `AppApiResult`-style error over panicking. A panic in a
  command is a `Result<_, String>` the frontend cannot classify.
- New privileged capability → new `#[tauri::command]` in `commands.rs`, with the
  policy check *in* the command, not in the caller.

### Adding a content kind

1. Add it to `ContentKindVals` in website-city's contract; `npm run contract:sync`.
2. Add a `KindSpec` entry in `website-city/src/lib/app-api/content.ts`.
3. Add labels to `KIND_LABELS` / `KIND_GROUPS` in `src/routes/browse.tsx`.

Nothing else — the card and the view page are kind-agnostic by construction.

### Adding a query protocol

1. A module under `core/src/net/query/`, exposing
   `async fn query(addr, timeout, …) -> AppResult<ServerQueryResult>`. Take a
   `SocketAddr` — never a hostname.
2. A variant on `QueryProtocol` matching Prisma's `SpyQueryProtocols` name, and
   an arm in `query()`'s match plus `is_native()`.
3. Parse with `net::reader::Reader`. No indexing, no slicing.
4. Tests: a golden reply, the malformed cases the protocol invites, and the
   truncated-prefix loop. **A parser with no truncation test is not finished.**

Nothing above the protocol module changes — the card, the panel and the graph
all read `ServerQueryResult`.

### Adding a game

1. `plugins/app/<slug>/manage_mod.json` — where its mods go. The slug is the
   game's URL segment on the website, lower-cased.
2. `plugins/app/<slug>/launch.json` — how to start it, if it can be started.
3. `plugins/app/<slug>/sandbox.json` — which deployment strategies suit it, its
   presets, its options, and how to FIND it (`detect`).
4. Nothing in Rust. If something needs adding in Rust, the file format is
   missing a field rather than the game being special.

The shipped examples under `examples/plugins/app/` are validated by a test that
builds each rule's real jail and resolves every step path through it — and, for
`sandbox.json`, checks that no preset names a strategy the game excludes and
that every option a preset sets survives its own schema.

### Adding a deployment strategy

1. A variant on `deploy::Strategy`, its `as_str`/`parse` arms, and its
   `link_kind`.
2. If it places files, a `LinkKind` and an arm in `link::place`. If it does not
   — a second virtualising strategy — it needs its own path in
   `engine::deploy`, and the ledger needs to describe what it did.
3. An arm in `available_strategies`, so the picker can say why it is
   unavailable rather than greying out a row with no explanation.
4. `DeployStrategyVals` in website-city's contract, and the app's mirror.
5. Tests: place, purge, and the "still ours" check that stops a purge deleting
   a file the user edited.

### Adding a step type

1. A variant on `Step` in `plugins/manifest.rs`.
2. An arm in `Executor::run_step`, resolving every path through
   `jail.resolve` and auditing the result.
3. An arm in `step_label` and in `required_roots`.
4. A test. **If the step can write, test that it cannot write outside the jail.**

## Artwork

Every list that names a game shows the game, and every list that names an item
shows the item. That is not decoration on this app in the way it would be on the
website: the site's chrome is built around one chosen game, so a mod card there
is unambiguous without a picture. Here the browse grid mixes Minecraft mods with
Rust servers, the sandbox list holds six games, and the download queue is forty
rows of similar text.

Two components, and the split between them is where the picture comes from:

  * **`GameIcon`** — a game's artwork. `ContentSummary.app.icon`, `facets.apps[]
    .icon` and `Install.app.icon` all carry it; a screen whose data is LOCAL
    (sandboxes, the library, anything out of SQLite) gets it from
    `useAppIcons()`, which shares a React Query key with the browse filters so
    it usually costs no request at all.
  * **`ItemThumb`** — an item's own cover, from `LibraryRow.image`. Never from a
    request: every row in a sandbox's mod list or the download queue is
    something the user subscribed to, so the picture is already on this device,
    and a lookup is a map hit rather than forty fetches in a list that redraws
    every second. `SandboxMod.modKey`, `Download.meta.item` and `LibraryRow.id`
    are all `kind:itemId`, which is what makes the lookup a one-liner.

**Both fall back to something rather than to nothing.** `GameIcon` draws the
game's initials on a tint hashed from its name — the same game is the same
colour in the browser, the sandbox list and the filter picker, because the hash
does not depend on the list's order or contents. `ItemThumb` draws the kind's
glyph. A blank space is worse than no picture: a list where some rows are
indented by an image and some are not reads as broken, not as sparse.

**Neither is ever the only thing carrying the name.** Every caller puts the
label beside it, and both render `aria-hidden` with `alt=""` — the name is
already in the row as text, so announcing it again makes a screen reader read
every row twice. An icon on its own is a guess, and a wrong guess about which
game a mod is for is how somebody installs it into the wrong folder.

**The local database does not cache artwork URLs**, deliberately. It mirrors
what the user owns; a CDN link cached in it would outlive every sync that
changed it, and the lookup costs nothing.

## Gotchas

- **A native `<select>`'s popup is drawn by the OS and cannot be styled.** The
  closed control takes CSS; the open list takes none of it, so every settings
  pane in a dark theme had one white rectangle in it. `components/select.tsx`
  is a real listbox and every `<select>` in the app is gone. Adding one back
  brings the white rectangle with it.
- **`plugins::jail` is the path jail; a `sandbox` is a user's mod profile.**
  They were both called `Sandbox` once. If a new type wants either name, it
  wants the other one.
- **A sandbox is an `AppInstall` on the wire.** The cloud model predates the
  word and `/api/app/v1/installs` is shipped; renaming the endpoint would 400
  every request from every installed copy of the app.
- **Epic's `bIsIncompleteInstall` is the one field in its manifest that is not
  PascalCase.** `rename_all = "PascalCase"` derives `BIsIncompleteInstall`,
  which never matches, which reads as "no game is ever incomplete" and offers a
  folder that is still being written into. It carries an explicit `rename`.
- **Steam's `installdir` is not the display name.** `Grand Theft Auto V` lives
  in `steamapps/common/GTAV`. Using the name is the single most common way to
  look in the wrong place.
- **A manifest whose folder is gone is not an installed game.** Steam leaves
  them behind after a failed uninstall and after moving a game between
  libraries; offering one produces a game directory that fails every check the
  moment somebody accepts it.
- **GOG Galaxy's database may be open.** It is read with `mode=ro&immutable=1`;
  read-only alone still takes a shared lock and still reads the WAL, so a
  Galaxy mid-write either blocks the read or hands back a torn one.
- **A `200` in answer to a `Range` request means the server ignored it.**
  Appending that to a `.tmcpart` produces a file that is too long, passes every
  length check, and fails its checksum with an error nobody can explain. The
  file restarts instead.
- **A download's checksum is computed from the FILE, not incrementally.** An
  incremental hash cannot survive a resume — the bytes from the first attempt
  never pass through this process — and a checksum that silently stops being
  checked on resumed downloads is worse than none.
- **Frostbite's packet `size` counts the two header fields it is part of.**
  Omitting them leaves four bytes of every packet in the buffer, which presents
  as "the second command always fails".
- **A merge tree built from an unsorted list is wrong in a way that looks
  right.** `merge::build` sorts internally; the last mod added wins instead of
  the highest priority, which nobody notices until they reorder their list and
  nothing changes.
- **`pluginData` is scoped per sandbox and per mod.** Two sandboxes staging the
  same mod run identical steps with an identical `{fileName}`. With one shared
  scratch directory the second run's download lands on the first's.
- **`tokio::spawn` inside a function that the spawned task calls back into is
  an infinitely large future type.** The download queue's `pump` → `run_one` →
  `pump` cycle is broken with one `Box<dyn Future>`; without it the compiler
  reports "cannot satisfy `impl Future: Send`", which does not obviously mean
  "you wrote a recursive future".

- **A query string carries no types, so `parseQuery` does not guess.** Values
  arrive as strings and the contract coerces per field (`z.coerce.number()`,
  `QueryBool`). Guessing from shape is the obvious approach and it silently
  broke `?search=2024`, `?search=true` and — because every cursor is a numeric
  id — the second page of every listing.
- **Version comparison is numeric per component, and refuses what it cannot
  order.** `1.10.0` sorts BEFORE `1.9.0` as a string, so an update banner built
  on a string compare either never appears or never goes away. `is_newer` in
  `commands/api.rs` parses dotted components and returns false for anything
  non-numeric — staying quiet beats nagging somebody toward a version they
  already have.
- **`z.coerce.boolean()` is `Boolean(value)`**, so the string `"false"` is
  `true`. Use `QueryBool` from the contract.
- **`serde(rename_all = "SCREAMING_SNAKE_CASE")` turns `A2S` into `A2_S`.** It
  splits between a digit and the letter after it, so the one protocol whose name
  contains a digit mid-word gets a wire name nothing uses. Every Source server
  then failed to deserialise — and because `query_servers` takes a
  `Vec<QueryRequest>`, one such row rejected the WHOLE batch, so a screenful of
  servers showed `TO` because one of them ran CS2. `QueryProtocol::A2S` carries
  an explicit `#[serde(rename = "A2S")]`, and
  `every_protocol_round_trips_under_its_wire_name` checks all eighteen against a
  hardcoded list. Reading the enum will not catch this: the variant names look
  identical to the wire names.
- **Tailwind 4 generates a utility only for a token that EXISTS.** `bg-surface-2`
  was written in seven places and produced no background at all, because the
  shared theme calls it `--surface-secondary` — a bug that looks exactly like a
  design choice, since the panel simply sits flat against its parent.
  `app.css`'s `@theme inline` block aliases `--color-surface-2` to it. Before
  inventing a colour name, check it against `@modcommunity/shared`'s
  `src/styles/theme.css`.
- **Unlayered CSS in `app.css` beats Tailwind's `@layer utilities`,** whatever
  the specificity — that is the cascade's layer rule, not a specificity contest.
  A `button { text-transform: inherit }` added to fix one header silently
  overrode `uppercase` on every button in the app. Resets in that file must be
  properties no utility sets.
- **`-webkit-app-region: drag` is a no-op in WebKitGTK and WKWebView.** It is a
  Chromium extension, so it appears to work on Windows and silently does nothing
  on the two platforms the custom titlebar most needs. Use
  `data-tauri-drag-region`.
- **The offline cache is an ALLOW-list, and the timestamps it restores are the
  original ones.** `lib/api/offline-cache.ts` stores `browse`, `content`,
  `facets` and `reviews` — public catalogue data — into `localStorage`, and puts
  them back at boot with the `dataUpdatedAt` they had. Restoring them as fresh
  would suppress the refetch and pin the app to whatever it last saw. It is an
  allow-list because the failure modes are not symmetrical: a new query holding
  account data that nobody remembered to exclude gets written to disk in
  cleartext, while a new public one that nobody remembered to include costs a
  spinner. `me`, `log` and `plugins` are all excluded by not being on it.
- **Every browse filter lives in the URL, never in component state.** A
  filtered browse has to survive a reload, a deep link and the back button, and
  a HashRouter over a static bundle has nowhere else durable to put it.
  `browse.tsx`'s `param`/`flag`/`num`/`idList` are the only place that encoding
  is decoded; multi-selects are comma lists.
- **`sort=players` is a deprecated alias of `curUsers`.** The app invented the
  name; the website has always called it `curUsers`. It stays in the contract's
  enum because an installed build is not redeployed with the server and would
  otherwise 400 on every server browse — but it is never offered in the sort
  dropdown.
- **The server browser defaults to the table, every other kind to the grid.**
  `?view=grid` / `?view=table` overrides it, and `view` is only honoured for
  `kind === 'server'` — nothing else has a table to switch to.
- **The server browser's defaults are the website's**, from
  `lib/user/settings/default.ts`: sort by player count descending, online only,
  table view. Sorting servers by "newest" instead means 2.6 million rows, nearly
  all of them freshly imported and never once seen online — the browser looked
  broken because every row read `TO`, and it was right to.
- **An app ref carries the game's FULL name, never `App.nameShort`.** The site
  abbreviates because its chrome is built around one chosen game; the app has no
  such chrome and shows a flat list of every game in the catalogue, where a
  `nameShort` that is unset — or set to `''`, which `??` happily returns — is a
  blank, selectable row in the filter's game picker. `mapApp` in website-city's
  `lib/app-api/content.ts` and the `/facets` handler both send `App.name`, and
  `lib/api/labels.ts`'s `appLabel` is the display-side guard for an older server
  or a genuinely empty name. Use it wherever a game is named on screen.
- **Ping is the FIRST column in the table**, as it is on the website. The table
  is wider than its pane and scrolls horizontally, so any column on the right can
  be off-screen — and the one the app exists to provide must never be the one
  that disappears.
- **A live `maxPlayers` is only believed when `>=` the live player count.**
  Several games report A2S `max_players` as something other than capacity — Rust
  answers ignoring the queue, which rendered as `144/51`. The measured PLAYER
  count always wins; only the slot count falls back to the API's.
- **Articles are opened in the system browser, not rendered.** `lib/external.ts`
  holds the list. Their bodies are laid out for the website's content column and
  the app's markdown subset strips exactly the parts carrying that layout.
- **The live-query context's accessors are stable; `version` is the render
  signal.** Anything that puts a context accessor in an effect dependency list
  re-runs that effect on its own result — which, for the server panel, was an
  unbounded query loop against a real game server.
- **Brand marks come from `react-icons/fa6`, never lucide.** lucide dropped its
  brand glyphs; `Github` / `Twitter` / `Facebook` survived only as deprecated
  aliases and are gone from ~0.475. `@modcommunity/shared` imported all three,
  which is why this package pinned `lucide-react` to `0.468.0` to build at all.
  Fixed in `shared@4.2.3`, so the pin is gone and lucide tracks `^0.542.0` with
  the other repos. lucide stays the house icon set for everything else.
- **Vite 8 minifies with oxc.** Setting `minify: 'esbuild'` demands esbuild as a
  separate install.
- **`@source` in `app.css` is required.** Tailwind ignores `node_modules`, so
  without it the shared components' classes are never generated.
- **`Path::join` with an absolute argument discards the base.** That single
  behaviour is why `join_relative` exists and why nothing in the plugin path
  should ever call `join` on untrusted input.
- **A tRPC mutation cannot set a cookie on a streamed response.** Relevant when
  touching the website's device-approval router — see website-city's
  `src/trpc/react.tsx`.
- **`generate_handler!` needs full module paths.** `commands::auth::auth_begin`,
  not a `pub use` of it — the macro also needs the hidden `__cmd__*` items
  `#[tauri::command]` generates, and those do not travel through a re-export.
- **`tauri.conf.json`'s feature allowlist and `Cargo.toml`'s `tauri` features
  must agree**, or the build script refuses with a message about
  `protocol-asset`. The asset protocol is deliberately off in both.
- **`keyring` pulls `libdbus-sys` on Linux**, which needs a dev package the
  security tests have no business requiring. It is behind `tmc-core`'s
  `os-keyring` feature, off by default and enabled by the app crate — which is
  why `npm run test:core` works on a bare machine.
- **A `///` block followed by a blank line documents the NEXT item.** Module
  headers in this crate are `//!` at the top of the file; clippy's
  `empty_line_after_doc_comments` catches the mistake and `check:rust` runs with
  `-D warnings`.
- **`serde(rename_all)` does not rename variant fields.** Enum variants in the
  plugin manifest need `rename_all_fields = "camelCase"` too, or a manifest
  would have to write `max_bytes`.

## Not built yet

Honest list, so nothing here reads as finished when it is not:

- **Self-updating.** The app CHECKS — `/api/app/v1/version` against two
  `SiteSetting` rows, once per launch when `autoUpdateCheck` is on, with a
  banner offering the download page in the user's browser. It does not install
  anything. A real updater needs `tauri-plugin-updater`, a signing key held by
  whoever cuts releases, and a manifest endpoint; shipping the client half
  against none of those would be a feature naming a capability it does not have,
  with a silently-installed binary as the consequence.
- **Writes.** The app is read-only against the API for publishing — no
  commenting or uploading. Reviews, review votes, reports, subscriptions and
  sandboxes DO write.
- **USVFS injection, proven.** The strategy is implemented end to end and gated
  behind `usvfs-hooks`, off by default. The tree and the blob are tested on
  every platform; the two hundred lines that patch an import table in somebody
  else's game process are type-checked against the Windows target and have never
  been run against a game. Turning the feature on is a decision to find out.
- **Plugin distribution.** Plugins install from a local folder and there is no
  registry to fetch them from. Signature *checking* is implemented and
  `requireSignedPlugins` enforces it, but nothing is published to be checked
  yet. Signing is `examples/plugin-sign`; what is missing is somewhere to
  publish the result and a key distribution story better than "paste this hex
  string".
- **Proof that a human chose a jail anchor.** `anchor::validate_root` decides
  whether a *directory* is an acceptable jail anchor, which is the enforceable
  half. The other half — that the path came from a real click rather than from
  script — died with the native dialog and cannot be recovered while the picker
  is drawn by the app. Restoring it needs an OS-level confirmation the webview
  cannot forge.
- **Protocols left on `TCP_ONLY`.** `DISCORD`, `GTA_NETWORK`, `GTA_RAGE`,
  `SCUM` — and unlike the rest of this list, they are not waiting to be
  written. Each is one the scanner speaks by asking a third party rather than
  the server, which the section above explains cannot be done from a user's
  device. `commands/servers.rs`'s `UNIMPLEMENTED` test constant names one of
  them, so a native implementation fails two tests on purpose.
- **No test runner on the frontend side.** `npm run check` is `tsc` plus
  `eslint`, and there are no unit tests in `src/`. The Rust side carries the
  logic that would most repay them — parsers, the jail, the deploy engine — but
  `lib/api/offline-cache.ts`'s allow-list is security-relevant and is currently
  held up by a code comment rather than by a test.
- **No end-to-end test against a real game server.** The protocol parsers are
  covered by golden-reply and fuzz-shaped unit tests; the socket paths above
  them have been exercised only against the bounds checks, not a live box. The
  same is true of RCON: the codecs are tested, the sockets are not.
- **No end-to-end test of a real deploy against a real game.** The deployment
  engine is tested against temporary directories — including the hard-link and
  symlink paths where the platform supports them — but nothing has been linked
  into an actual Steam folder and launched.
- **The download queue's own network path IS tested**, against a loopback HTTP
  server: resume, the ignored-range trap, checksum rejection, a dropped
  connection retrying, cancel and pause. That is the exception, not the rule.
