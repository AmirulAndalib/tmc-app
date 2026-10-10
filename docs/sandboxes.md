# The library, sandboxes, and how mods reach the game

## The library

The device's answer to the account's subscriptions: which of them this machine holds, which are on disk, and how they got there.

> The Library SCREEN has three views and they are genuinely different nouns: **Games** is what somebody else's launcher installed and this app mods, **TMC Games** is what this app installed from a build TMC published (see [playing.md](playing.md#games-tmc-publishes)), and **Subscribed** is what the account subscribed to. One list holding all three would have to explain, per row, which kind of thing it was.

**SQLite, not a JSON file** (`library/db.rs`). Everything else the app persists — settings, the plugin registry — is a small file rewritten whole, and that is right for those: read at launch, written on a click, never contended. The library is not like that. The sync loop writes it on a timer while the UI reads it on every render and the installer writes single rows mid-install, and a whole-file rewrite under that pattern loses one writer to another exactly when somebody is watching a progress bar.

Schema changes are **stepwise** — each migration takes the database from `n-1` to `n` and runs only if it has not. Re-running one batch happens to be safe today because every statement is `IF NOT EXISTS`, and stops being safe the first time a step needs an `ALTER TABLE`.

**It polls** (`library/sync.rs`), because a subscription can be created in a BROWSER and there is no push channel to an installed desktop app that is reliable across three desktop platforms, two mobile ones, corporate firewalls and sleeping laptops. A watermark makes that cheap: each response carries a `revision`, the next request sends it back, and a device open for an hour has made sixty requests and transferred one row.

**A full sync still happens** every `FULL_SYNC_EVERY` passes and always on launch, because a delta cannot express a DELETION — there is nothing left to poll. Anything the device holds that the server did not send has been unsubscribed elsewhere.

**An upsert from the server never touches the device-local columns.** That is what stops a resync forgetting what is installed, which is the single easiest way to turn a working library into an endless reinstall loop.

**The plan is executed in Rust, not returned to the webview.** A frontend that decided what to install is one an injected script can talk into installing something.

## Sandboxes, and how mods reach the game

A **sandbox** is what other mod managers call a profile or an instance: a named set of mods with its own load order, its own deployment method and its own launch settings.

> The word is why `plugins::jail` is called a jail. It used to be `plugins::sandbox::Sandbox` — the path jail — and two `Sandbox` types in one crate, one of them a security boundary, is a mistake waiting for somebody to reach for the wrong one.

**On the wire a sandbox is an `AppInstall`.** The cloud model predates the word and `/api/app/v1/installs` is shipped, so renaming the endpoint would 400 every request from every installed copy of the app. The app says "sandbox"; the API says "install"; they are the same thing.

### What lives where

By the same test the settings use — *would this be wrong to apply on a different machine?*

| Fact | Where | Because |
| --- | --- | --- |
| name, mods, order, options, launch flags | the account (`AppInstall`) | signing in on a second machine should reproduce it |
| which folder it deploys into | the device | that path exists on one machine |
| which release of each mod is staged | the device | so does that |
| the deployment ledger | the device | it describes files on one disk |

The cloud half is **optional per sandbox**. `cloudSync` off means it is never sent anywhere and a reconcile cannot delete it — which is the whole answer to "I do not want my mod list on your server", and it has to be per-sandbox rather than global because the useful case is "sync my modpack profiles, not the one I use for testing".

### Two steps, not one

```text
  stage   — run the game's install rule with the sandbox's own staging folder
            standing in for the game directory
  deploy  — mirror every staged file into the real game folder, by whichever
            mechanism the sandbox is set to
```

Splitting them is what makes the whole feature work:

- **Every install rule written before sandboxes existed still works.** A rule that copies to `mods/{fileName}` writes to `<staging>/<sandbox>/<mod>/mods/foo.jar`, and deployment puts `mods/foo.jar` in the game folder. The rule never learns which strategy is in use, and it should not — "where does this game's mod go" and "how do files reach the game folder" are different questions.
- **Switching sandboxes is a link operation, not a download.** Staging survives an undeploy.
- **Nothing half-downloaded is ever in the game folder.**

`pluginData` is scoped per sandbox and per mod (`jail_for_scoped`). Two sandboxes staging the same mod run identical steps with an identical `{fileName}`; a shared scratch directory meant the second run's download landed on the first's, and a sandbox pinned to an older release quietly got the newer file.

### The four strategies

| Strategy | Game folder | Cost | Fails when |
| --- | --- | --- | --- |
| `direct` | modified | a full copy | never; it is the fallback |
| `hardlink` | link pointers | nothing | staging is on another drive |
| `symlink` | link pointers | nothing | Windows without Developer Mode |
| `usvfs` | untouched | nothing | not Windows, or not built with `usvfs-hooks` |

#### USVFS: the one that deploys nothing

The user-space virtual filesystem approach, in `src-tauri/usvfs/` (`tmc-usvfs`): the mods stay in staging and the game is *told* they are there, by a DLL injected before its entry point runs that patches `CreateFileW` and `GetFileAttributesW` in its import table and answers them from a merged in-memory tree.

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

**Deploying is not a file operation**, and everything downstream follows from that:

- the merged tree is serialised to a blob inside the sandbox's own staging folder, and **that publish IS the deploy**;
- **the ledger comes back empty**, which is the correct record rather than a gap — it names files in the game folder and there are none, so `purge` correctly does nothing and `verify` correctly reports a healthy folder;
- **a sandbox switching TO it still purges what the last strategy left**, which is the one write to the game folder a virtual deploy performs. Without it the user gets both: every file twice, with the virtual copy winning only where the tree happens to cover it;
- **there is no fallback in either direction.** Whether the game folder is modified at all is the reason somebody picks this, so silently linking instead would dirty a folder chosen to stay clean, and silently going virtual would leave a game that was never told about the mods.

**The revision is a content hash, not a counter or a clock.** The injected DLL compares it to notice that a blob it already read has changed; a counter would also tick for a redeploy producing an identical tree.

**It is gated twice, at deploy and at launch** (`deploy::usvfs_unavailable`, `spawn::launch_with_vfs`), on Windows AND on `tmc-core`'s `usvfs-hooks` feature, which is **off**. The two are halves of one decision: a build with only one on could publish a tree no launch would carry, or inject with nothing to inject. The refusals are separate strings because they are separate facts — "this is Windows-only" can never change for a Mac user, and "this build has it off" can.

Why the gate exists at all is a confidence split, stated in `usvfs/src/lib.rs`:

| Half | State |
| --- | --- |
| `tree` — the merged view, resolution, listings, case rules | **Tested**, on every platform |
| `shm` — publishing and reading the blob atomically | **Tested**, including every malformed input |
| `hooks` — IAT patching, the redirecting `CreateFileW` | **Type-checked against `x86_64-pc-windows-gnu`. Never run against a game.** |
| `inject` — suspended launch, remote `LoadLibrary` | **Type-checked.** Argument quoting and environment building are tested; the launch is not |

`cargo check -p tmc-usvfs --target x86_64-pc-windows-gnu` is how that type-checking happens and it is worth keeping working — it has found real bugs reading missed, such as `IMAGE_NT_HEADERS64` living in `Diagnostics::Debug` while being gated behind the `Win32_System_SystemInformation` feature. `tmc-core` cannot be cross-checked that cheaply (bundled SQLite needs a C cross-compiler, which only the `cargo-xwin` toolchain in [BUILDING.md](BUILDING.md) supplies), which is exactly why the injector is its own dependency-light crate.

**What it cannot do, even when it works:** calls through `GetProcAddress` are not redirected (the pointer never came from an import table), direct `ntdll` syscalls are not redirected, writes are not redirected, and every kernel-level anti-cheat treats injection as an attack — correctly. A game declaring `antiCheat: "kernel"` in its `sandbox.json` is refused this strategy before it reaches the engine.

**A URI launch rule and a virtual deploy are refused together.** A game started through `steam://` is started by Steam, and there is no process of ours to inject into; the alternative starts the game with none of the sandbox's mods and reports success, which is the hardest kind of bug to diagnose because the folder is stock and there is nothing to find.

**Capability is probed, not assumed.** `deploy::link::probe` creates one file and links it, twice, and reports what worked. Every rules table for this is wrong somewhere — a Linux box with staging on an exFAT USB drive, a macOS volume with links disabled, a Windows machine with Developer Mode on.

**Falling back between the two LINK strategies is automatic and reported. Falling back to copying is not.** Copying is not a worse link, it is a different decision: it writes gigabytes, it modifies the game's own files, and its failure mode is a game folder that needs restoring rather than unlinking. A sandbox that asked for links and can have neither gets an error naming both fixes.

### Three properties that hold across every strategy

- **Nothing is overwritten.** A file already in the game folder that the app did not put there is MOVED to the backup store before its place is taken, and a purge puts it back.
- **Nothing is removed unless it is still ours.** Every removal re-checks the file against the ledger row that claims it — a symbolic link must still point at its staging file, a hard link must still share its identity, a copy must still have its recorded size and mtime. Anything else is left alone and reported, so a config the user edited after deploying survives.
- **Staging is never modified.** Deployment only ever reads from it, which is what lets two sandboxes share one downloaded copy of a mod.

### The ledger

`deploy::ledger` is the only record of which files in somebody's game folder belong to us, and undeploying by rescanning cannot replace it: a rescan can see that `Data/textures/sky.dds` exists, and cannot see whether the app put it there or whether it shipped with the game. Guessing wrong either strands a modded file forever or deletes a base-game asset.

It is written even when a deploy reported errors — a partial deploy put files on disk, and losing the record of them creates exactly the orphans the ledger exists to prevent.

### Conflicts

The merge tree decides one winner per path before anything touches the disk, by priority, with ties broken on the mod key so the answer is stable across runs. Losers are **reported**, never merged: nothing here understands any game's file formats, and a manager that silently produces a file neither author wrote is how "it works for me" bug reports are made.

Sorting happens inside `merge::build`, not in the caller. A tree built from an unsorted list is wrong in a way that looks right until somebody reorders their list and nothing changes.

### `sandbox.json`

The strategy matrix per game, plus presets, options and detection hints:

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
    "detect": { "steamAppIds": ["271590"], "markers": ["Game.exe"] }
  }
}
```

- **`options`** is a *form description* and nothing more. It cannot express a condition, a computation or a dependency between fields — a settings schema that can do those is a program, and the plugin model rests on plugins not being programs. A game needing one needs a second preset instead.
- **Values are clamped against the schema on every write**, and a key the schema does not declare is dropped. The options reach the cloud and come back, so "the server said 900 GB of heap" is answered here rather than by a game that will not start.
- **`antiCheat: "kernel"`** is load-bearing. Kernel-level anti-cheat systems watch for a game folder whose files are not where they should be, so a game declaring it gets Direct as its default and a warning on every other option — the cost of guessing wrong is somebody's account, not a failed install.

Options reach a command line only through `optionArgs` in the game's own `launch.json`. `LaunchOptions` carries them in a flattened `extra` map; an object or an array contributes nothing, and there is still no shell.

## Sharing a sandbox

A code, not a file. `library::share` encodes a sandbox as `TMC1-<base64>`.

**Why a code.** A file export needs a path to write and a file import needs a path to read, and the webview names neither — the one command that takes a path returns directory names and reads nothing. Trading that guarantee for a save dialog is not a trade worth making, and a code is what people do with an exported profile anyway: put it in a message.

**Item ids, never files.** Shipping the files would make this a redistribution channel for other people's work, with no download counted, no licence respected and no update path.

**Nothing about this machine.** No game directory, no launch environment, no ledger, no staging path — the same test the settings split uses, and a code is by definition going somewhere else. A game folder in a shared code is also somebody's username in a chat message. `a_code_carries_nothing_about_this_machine` decodes an export and asserts none of it is there.

**Importing previews first**, because a code is a string from somebody else and pressing Import can make two hundred subscriptions on an account. It subscribes and adds; it does not stage — two hundred mods is two hundred downloads, and starting them inside a command the UI is awaiting is a frozen dialog with no queue to look at. `cloudSync` and `autoUpdate` come from the importer's own defaults: whether somebody's mod list leaves their machine is their decision, not the exporter's.
