# Importing mods from outside the account

Everything above assumes an account subscribed to something. That is the right model for what the site holds and it is the whole model for nothing else — and "nothing else" is most of what is on a modder's disk: the jar from a Discord thread, the folder they built themselves, the two hundred mods already sitting in Vortex, the pack downloaded before they had an account.

A **local mod** is one directory of files plus editable text about it, and it is deployed by exactly the code that deploys a subscribed one.

`deploy::DeployMod` wants a key, a name, a priority and a folder laid out as it belongs under the game directory. A staged subscription is one of those; an import is another. So the merge tree, the conflict report, the ledger, the backup-before-overwrite rule and the "still ours" purge check all apply to an imported mod with **no new code and no second set of rules to keep in step** — `library::deploy::mod_root` is the single place the two sources meet, and `an_imported_mod_and_a_subscribed_one_deploy_through_one_ledger` is what stops that claim rotting.

| File | Owns |
| --- | --- |
| `local/metadata.rs` | `tmc.json` — what an archive says about where it came from |
| `local/store.rs` | The files: importing, laying out, moving, removing |
| `local/adopt.rs` | What is already in a game folder that nothing here claims |
| `local/vault.rs` | How the webview names a path it is never shown |
| `plugins/managers.rs` | The fourth plugin type: another mod manager, read |

## The webview still never names a path

`commands/import.rs` looks like an exception to `commands/mod.rs`'s second rule and is not one. Every path it acts on was produced by **this side** — the OS delivered a drag-and-drop event to the window, or one of the two scans listed a directory — and reaches the frontend as an opaque token.

What that buys is bounded and `local/vault.rs` says so: a script with execution in the webview can replay a token for a file the user just dropped, which is nothing it could not get by asking them to drop it again. It cannot name `~/.ssh/id_ed25519`, because **no token exists for one**. Tokens expire and the store is capped, because a capability with no lifetime is one an injected script can hoard.

`dragDropEnabled` is therefore ON in `tauri.conf.json`, which also means the webview receives no HTML5 drag events at all — the point, since an `ondrop` handler there would be the untrusted half deciding what was dropped, holding real paths to do it with. The hover events are emitted from Rust carrying a COUNT and nothing else.

## `tmc.json`

The website puts a small JSON document at a release archive's root; the importer reads it for the item's name, version and link. It is a **hint about identity**, not a permission and not a signature: it arrives inside a file a stranger supplied, so `installPath` goes through `join_relative` like any other untrusted path, and an item link is offered only when `source`'s ORIGIN matches this build's `api_base()` — a production archive must not hand a dev build an item id from a different database.

Not signed, deliberately. The app has the machinery, and it would be the wrong question: a release file is authored by the mod's author and served by the site unmodified, so a signing key covering every upload is a key on a web server. The thing worth verifying about a download is its **checksum**, which the API already carries and the queue already enforces.

## The one genuinely ambiguous decision

Whether to unpack. `.jar` is a zip and must not be unpacked; a resource pack is a `.zip` and must not be either; a mod shipped as a zip of loose DLLs must be. So the **game's own install rules decide**: `AppPlugins::mod_extensions` reads `match.extensions` off the `manage_*` rules, and a file whose extension a rule accepts is copied verbatim. Nothing invents a second list, and a game that grows a new packaging format gets it for free.

Where the payload lands is a guess and is treated as one — the sidecar's `installPath`, then "does its top level already hold a declared mod target?", then the game's first `modTargets` entry — and then the user can change it. `store::relayout` is what makes that cheap: a directory rename inside the store, not a re-import.

## Adopting what is already there

`local::adopt` lists the immediate children of the folders a game's `sandbox.json` declares as mod targets, and subtracts everything the app can account for: the deployment ledger, a subscription's installed files, and previously adopted rows. Getting that subtraction wrong is worse than not having the feature — adopt something the app already deploys and the user has one mod listed twice, with the merge tree correctly reporting every file as conflicting with itself.

It is **not a filesystem scan**. A game with no `sandbox.json` offers nothing, which is the honest answer rather than a guess about where its mods might be. And it **copies**: the original stays exactly where it was, so backing out is deleting a row and the game keeps working either way.

## Reading another mod manager

The fourth plugin type. `plugins/manager/*.json` is compiled into the binary by `core/build.rs` alongside the app rules and for the same reason, with `<app data>/plugins/manager/` as a per-id overlay; a registry bundle carrying a `manager` block is the third source and goes through the approval and signature gate on its way out.

A descriptor names directories and says how they nest — a base the APP resolves (`home`, `appData`, `localData`, `programData`, `documents`), a relative path, a `modsPath` with `{game}` and at most **one** `*` segment whose name becomes the profile/instance label, a game-id-to-slug table, and optionally a per-mod JSON file with pointers into it. It cannot run a program, read an environment variable, name an absolute path or reach the network, because nothing in the format expresses any of those. One wildcard, not any number: two would make a scan the product of two listings over somebody's whole `AppData`.

**Every shipped descriptor reads local storage, and that is where the answer is** — not a limitation of the plugin model:

- **CurseForge's** API needs a key issued per application and does not answer "what has this user installed"; the instance folders do.
- **Nexus's** API is per-user and **Vortex** has no remote API at all; its staging folders are the record.
- **Thunderstore's** API is open but serves the *registry*, not what a machine installed; r2modman's profiles are on disk.

Ships: r2modman, Thunderstore Mod Manager, Vortex, CurseForge, Prism, MultiMC. The game folder names those managers use are their own and are the one thing likely to be wrong — a corrected descriptor dropped in `plugins/manager/` is the whole fix, which is exactly why this is data rather than a `vortex.rs`.

MO2 is **not** shipped, and the reason is worth keeping: its instances are named freely by the user and the game is recorded inside `ModOrganizer.ini`, so no declarative name match is reliable. A descriptor that guessed would find the wrong game's mods, which is worse than finding none.

## What a local mod deliberately has not got

**Updates.** There is no release history, no checksum from anybody and no URL to re-fetch — that is what makes it local. `LocalMod` has no `latest_version`, the auto-updater never sees one (it looks items up with `find_item`, which returns nothing for `local:`), and the honest way to update an import is to import the new file. When a sidecar named an item on this site the row keeps the reference so the UI can offer "subscribe to this instead", which is what turns a snapshot into something the app can keep current.

**A second identity space.** The key is `local:<id>`, in the same `sandbox_mod` table and the same ledger as `mod:1234`. One list, one load order, one deploy.

**A place in `subscription`.** `local_mod` is its own table because `subscription` is a mirror the full sync rewrites — anything the server did not send is deleted, and a dropped jar surviving its first sync is the one behaviour an import must have.

## Two layout traps

- **An archive that already holds `mods/` is a slice of the game folder.** Nesting it under the game's mod target again produces `mods/mods/foo.jar`, which deploys, reports no error and does nothing. `store::suggest_rel_path` tests the payload's top level against the game's declared `modTargets` before it nests anything.
- **`store::relayout` cannot rename in place.** Moving a payload from the game's root into `mods` is a rename of a directory into itself, which every platform refuses — and root-to-subdirectory is the FIRST case a user hits, because it is what an unrecognised archive gets. It goes via a sibling holding directory, and puts the files back if the second rename fails.
