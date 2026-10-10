# History

Bugs this repository has had, kept because each could come back and the symptom is what you will see first. The rule each one left behind is in the linked file; this is how it looked.

## Server queries

- **Every A2S server showed `TO`.** `serde(rename_all = "SCREAMING_SNAKE_CASE")` derived the wire name `A2_S`, so every A2S row failed to deserialise — and because `query_servers` takes a `Vec<QueryRequest>`, one such row rejected the whole batch, so a screenful of unrelated servers timed out together. Fixed with an explicit `rename` and a round-trip test over `WIRE_NAMES`. Rule: [gotchas.md](gotchas.md#rust-and-tauri).
- **Servers looked dead because the app probed the wrong port.** `portOffset` and `swapGamePort` were read as port selection; the first is dead config the scanner never reads, the second is post-scan bookkeeping. Probing `game + offset` hit a port nobody listened on, which renders as a timeout. Rule: [live-queries.md](live-queries.md#which-port-gets-queried-and-why-not-the-other-two).
- **Every split A2S roster came out "incomplete".** Reading the older packed `total`/`number` byte while also consuming a size field parsed fragment 0 plausibly and decoded every later fragment as index 0, so they overwrote each other. Rule: [live-queries.md](live-queries.md#rules-for-anything-under-netquery).
- **A screenful of servers all read `TO` by default.** The browser once sorted servers by "newest", which is millions of freshly imported rows never seen online — the app was right and looked broken. The defaults now copy the website's. Rule: [gotchas.md](gotchas.md#contract-query-strings-and-the-browse-screen).
- **The server panel queried a real game server in a loop.** A live-query context accessor in an effect dependency list re-ran the effect on its own result. Rule: [gotchas.md](gotchas.md#contract-query-strings-and-the-browse-screen).

## The contract and the browse screen

- **`?search=2024` and every second page broke.** `parseQuery` guessed types from shape, and every cursor is a numeric id. Rule: the contract coerces per field ([gotchas.md](gotchas.md#contract-query-strings-and-the-browse-screen)).
- **The app listed penalised servers the site hid.** `/browse` did not pin `hasPenalties: false`, so the app was the one surface still showing `SPY_FLAG`'d redirect farms and fake-player-count boxes. Rule: [api.md](api.md#the-browse-filters-mirror-the-websites-deliberately).

## Launching and playing

- **"Join server" did nothing, every time.** The button handed `connectUrl` (always `steam://` or a sibling) to `openUrl`, whose capability is scoped to `https://*`; the refusal was silent. Rule: non-`https` goes through Rust ([gotchas.md](gotchas.md#typescript-css-and-the-build)).
- **Every launch reported `playedSeconds: 0`.** `spawn::run` spawned the game and dropped the `Child`, so the Library could not tell a game was open and a crash left nothing to read. `session::Sessions` keeps it now. Rule: [playing.md](playing.md#play-sessions).
- **A Godot client started at its menu and joined nothing.** Its arguments were passed without the bare `--` the engine needs before user arguments. Rule: [playing.md](playing.md#the-launch-arguments-are-a-vector-never-a-command-line).

## Plugins and rules

- **The first release supported no games at all, silently.** The app rules were a Tauri resource beside the binary, which the portable Windows exe and a relocated AppImage do not carry; detection, launch, sandboxes and the config editor all accept an empty map without complaint. They are compiled in now, and a test compares them to the repository. Rule: [plugins.md](plugins.md#where-they-come-from-and-why-they-are-compiled-in).
- **A dev build refused every download.** The shipped rules named the production host in their allow-list, and a dev build's files come from the dev host. `download_hosts` now adds `api_base()`'s host. Rule: [plugins.md](plugins.md#where-they-come-from-and-why-they-are-compiled-in).
- **`requireSignedPlugins` was a stored boolean with nothing behind it.** Rule: [plugins.md](plugins.md#signatures).
- **A sandbox pinned to an older release got the newer file.** Two sandboxes staging the same mod shared one `pluginData` scratch directory, and the second download landed on the first. Rule: [sandboxes.md](sandboxes.md#two-steps-not-one).

## Paths, auth and updates

- **Every Steam path carried `\\?\`.** `detect::steam::dedupe` returned the canonical form it only meant to use as a duplicate key, and every game path is joined onto a library path; `starts_with` checks then answered "outside" for everything. Rule: `canon::canonicalize` ([gotchas.md](gotchas.md#rust-and-tauri)).
- **A dev run signed the developer out of the live site.** Dev and production shared one stored refresh token, so the real token was sent to the dev server, refused, and a refused refresh is terminal. Sessions are keyed on `api_base_scope()` now. Rule: [development.md](development.md#pointing-a-build-at-the-dev-site).
- **A runtime updater key was impossible.** The updater plugin was registered only when a key was compiled in, and `updater_builder()` panics without its managed state. Rule: [updater.md](updater.md).

## Styling

- **Panels sat flat against their parent.** `bg-surface-2` was written in several places and generated nothing, because the shared theme calls it `--surface-secondary`; Tailwind 4 emits a utility only for a token that exists. Rule: [gotchas.md](gotchas.md#typescript-css-and-the-build).
- **Every button lost `uppercase`.** A `button { text-transform: inherit }` reset in unlayered `app.css` beat Tailwind's `@layer utilities`. Rule: [gotchas.md](gotchas.md#typescript-css-and-the-build).
- **`lucide-react` was pinned to an old version to build at all.** lucide dropped its brand glyphs and `@modcommunity/shared` imported three of them; fixed in `shared@4.2.3`, and the pin is gone. Rule: brand marks come from `react-icons/fa6`.
