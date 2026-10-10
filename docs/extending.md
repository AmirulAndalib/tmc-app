# Adding things

## Adding a content kind

1. Add it to `ContentKindVals` in website-city's contract; `npm run contract:sync`.
2. Add a `KindSpec` entry in `website-city/src/lib/app-api/content.ts`.
3. Add labels to `KIND_LABELS` /`KIND_GROUPS` in `src/routes/browse.tsx`.

Nothing else — the card and the view page are kind-agnostic by construction.

## Adding a query protocol

1. A module under `core/src/net/query/`, exposing `async fn query(addr, timeout, …) -> AppResult<ServerQueryResult>`. Take a `SocketAddr` — never a hostname.
2. A variant on `QueryProtocol` matching Prisma's `SpyQueryProtocols` name, and an arm in `query()`'s match plus `is_native()`.
3. Parse with `net::reader::Reader`. No indexing, no slicing.
4. Tests: a golden reply, the malformed cases the protocol invites, and the truncated-prefix loop. **A parser with no truncation test is not finished.**

Nothing above the protocol module changes — the card, the panel and the graph all read `ServerQueryResult`.

## Adding a game

1. `plugins/app/<slug>/manage_mod.json` — where its mods go. The slug is the game's URL segment on the website, lower-cased.
2. `plugins/app/<slug>/launch.json` — how to start it, if it can be started.
3. `plugins/app/<slug>/sandbox.json` — which deployment strategies suit it, its presets, its options, and how to FIND it (`detect`).
4. `plugins/app/<slug>/config.json` — where its settings files live, if it has any worth editing. Optional; a game without one has no config editor, which is the honest answer rather than a guess.
5. Nothing in Rust. If something needs adding in Rust, the file format is missing a field rather than the game being special.

`sandbox.json`'s `modTargets` earns its keep twice over now: it is what an import falls back to when it cannot tell where a payload belongs, and it is the complete list of folders the adopt scan looks in. A game whose targets are wrong imports mods to the wrong place and finds nothing to adopt.

The rules under `plugins/app/` are validated by a test that builds each rule's real jail and resolves every step path through it — and, for `sandbox.json`, checks that no preset names a strategy the game excludes and that every option a preset sets survives its own schema. It runs against `AppPlugins::shipped()`, the compiled-in copy, because that is the one a user actually gets.

**They are compiled into the binary**, so a new game needs a rebuild to appear — `cargo` reruns the build script when the directory changes. A rule dropped into `<app data>/plugins/app/` needs no rebuild and overrides a shipped game outright.

## Adding a mod manager

1. `plugins/manager/<id>.json` — its roots per platform, its `modsPath`, its game-directory-to-slug table, and its per-mod metadata file if it has one.
2. Nothing else. It is compiled in by `core/build.rs` on the next build, and `every_shipped_manager_descriptor_parses` fails the run if it does not load.

A descriptor that finds nothing fails closed and says so, which is the right failure: the folder names these managers use are their own, and being wrong about one costs a scan rather than a game folder.

## Adding a deployment strategy

1. A variant on `deploy::Strategy`, its `as_str`/`parse` arms, and its `link_kind`.
2. If it places files, a `LinkKind` and an arm in `link::place`. If it does not — a second virtualising strategy — it needs its own path in `engine::deploy`, and the ledger needs to describe what it did.
3. An arm in `available_strategies`, so the picker can say why it is unavailable rather than greying out a row with no explanation.
4. `DeployStrategyVals` in website-city's contract, and the app's mirror.
5. Tests: place, purge, and the "still ours" check that stops a purge deleting a file the user edited.

## Adding a step type

1. A variant on `Step` in `plugins/manifest.rs`.
2. An arm in `Executor::run_step`, resolving every path through `jail.resolve` and auditing the result.
3. An arm in `step_label` and in `required_roots`.
4. A test. **If the step can write, test that it cannot write outside the jail.**
