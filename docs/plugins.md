# The plugin system

**A plugin is data, never code.** No script engine, no WASM, no `exec` step. An installer describes file operations; a query plugin describes a datagram and how to read the reply; a theme describes colour tokens. Everything a plugin can express is something this crate implements and audits.

That is a real limitation and it is the point: the alternative is third-party code running with full filesystem access next to the user's saves and credentials, and nothing bolted on afterwards recovers from that.

## Four types

| Type | Declares | Runtime |
| --- | --- | --- |
| **Installer** | Ordered `install` /`uninstall` steps | `plugins/steps.rs` |
| **Server Live Query** | A hex request + a field list | `plugins/query.rs` |
| **Theme** | A map of CSS custom properties | `plugins/theme.rs` |
| **Manager** | Where another mod manager keeps its mods | `plugins/managers.rs` |

## The step vocabulary

`download`, `extract`, `copy`, `move`, `mkdir`, `remove`, `writeText`, `patchJson`. **Adding a step that runs a program, resolves an absolute path, or reads an environment variable breaks the model** — do not.

Every path is a `PathRef { root, path }` where `root` is one of `gameDir`, `pluginData`, `downloads`. The type cannot express an absolute path.

**`path` is relative to the ROOT, not to any grant.** A plugin granted `{gameDir, "mods", write}` writes to `{ root: gameDir, path: "mods/foo.jar" }`. Grants are *filters* over the resolved path, so several narrow grants under one root compose as a union and stay narrow — the alternative, anchoring each grant at its own subdirectory, cannot say which grant a `PathRef` meant and used to merge them into the widest one.

## The defences, and what each one catches

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

## Approval

`plugin_inspect` returns a fingerprint (SHA-256 of the canonical manifest), the full permission list and the signature state; `plugin_approve` takes the fingerprint back. That closes the window between the user reading the permissions and clicking approve.

`Registry::rescan` runs every launch and re-hashes each installed manifest. A drifted plugin is **disabled** and flagged `needsReapproval`; the toggle refuses to re-enable it, because the toggle is not where permissions are shown.

## Signatures

The fingerprint answers *"is this the same plugin the user approved?"*. It cannot answer *"did anybody the user trusts write it?"* — a hash of whatever is in the folder is perfectly good for a bundle that anything at all dropped there. `plugins/signature.rs` is the second question: Ed25519 over the manifest's **canonical bytes**, the same serialisation the fingerprint hashes, so a bundle reformatted in transit keeps its signature and a bundle whose declared permissions changed loses it. The detached signature is `plugin.sig`, hex.

**Three states, not two.** `Unsigned`, `Trusted(keyId)` and `Untrusted` — the last being a signature that matches nothing the user trusts, which is either a publisher whose key has not been added yet or a tampered bundle. Folding it into "unsigned" is how the interesting case disappears, so the UI and the refusal messages keep them apart.

**The trust store is the user's, and no key is compiled in.** Keys live in `plugins/trusted-keys.json`, and TMC's own publishing key will be a row in it like anybody else's — the honest shape while there is no registry to distribute plugins through, and still correct once there is. Bundling a key nobody signs with would make `requireSignedPlugins` a switch that refuses everything.

**`requireSignedPlugins` must stay enforced.** A stored boolean with nothing behind it is worse than not having it — somebody turns it on, believes they are protected, and installs accordingly. With it on, `active()` refuses an unsigned or untrusted plugin before the executor can be reached.

Two properties of the gate that are easy to lose:

- **It re-verifies rather than reading `record.signature`.** The stored verdict is a claim about a check that ran at some earlier time under a trust store that may since have changed, and this is the last gate before something writes to a game folder.
- **Trusting a key rescans immediately.** "Add the publisher's key" is advice that has to work when followed, not after a restart — and removing a key has to stop those plugins at once, which is the entire reason to remove one.

**Producing one** is `cargo run -p tmc-core --example plugin-sign` — `keygen`, `sign <dir> <secret>`, `verify <dir> <public>`. It exists so the feature is usable without reimplementing it from a doc comment, which would mean guessing at the one detail that matters. Its two properties are worth checking by hand after any change to `Manifest`: a manifest minified and key-sorted still verifies, and one extra `gameDir` write grant does not.

What a signature buys is bounded and stated in the module header: it says a holder of that key produced *these* permissions and *these* steps. It says nothing about whether they are safe. A signed plugin is confined by exactly the same jail as an unsigned one.

Examples live in `examples/plugins/` and are **validated by two Rust tests**: one parses every manifest, the other builds each installer's real jail and resolves every step path through it. The second is the one with teeth — parsing only proves the JSON is well-formed, while a manifest whose grants and step paths disagree is the mistake an author copying the example would inherit.

## App plugins: the rules for one game

Distinct from the bundles above, and deliberately:

| | Registry plugin | App plugin |
| --- | --- | --- |
| Where | `plugins/<id>/plugin.json` | `plugins/app/<slug>/*.json\|yaml` |
| Identified by | a reverse-DNS id the author picks | the GAME it handles |
| Answers | "what can this plugin do?" | "where do this game's mods go, and how do its sandboxes work?" |

A bundle is something a user installed; an app plugin is a *rule for a game*, the app ships one per supported game, and there are dozens of tiny ones. The directory name is the game's **URL slug**, lower-cased. `disabled/` is skipped at any depth, which is the whole mechanism for turning a rule off without deleting it.

| File | Declares |
| --- | --- |
| `manage_mod.json` /`manage_asset.yaml` | Install and uninstall steps for one content kind |
| `launch.json` | How to start the game |
| `sandbox.json` | Deployment strategies, presets, the game's own options, and detection hints |
| `config.json` | Where this game keeps the files a player edits by hand |

**The safety model is unchanged.** Each file compiles to a synthetic `Manifest` and runs through the same jail and the same executor, so it can express nothing a registry plugin cannot. What it adds is only the *selection* — which rule applies to which game, kind and file.

### Where they come from, and why they are compiled in

Two sources, and `AppPlugins::resolve` is the only thing that reads both:

| Source | Holds |
| --- | --- |
| Compiled into the binary from `<repo>/plugins/app/` | the games the app ships support for |
| `<app data>/plugins/app/` | whatever the user added |

**The shipped half is embedded by `core/build.rs`, not shipped beside the binary.** A Tauri resource is a separate file next to the exe — which the portable Windows build does not carry, and neither does an AppImage moved out of its directory. Without the rules the cost is not "fewer games": **detection finds nothing, no game can be launched, no sandbox knows its strategy and the config editor has nothing to list — silently**, because every one of those features reads an empty map perfectly happily. That is far too much to hang on a file being adjacent.

The rules stay files in the repository; the build script only reads them. `the_compiled_in_rules_match_the_repository` asserts the two agree, so a build script that quietly stopped embedding one fails a test rather than shipping.

**A slug the user has rules for REPLACES the shipped one** rather than merging with it. Two authors' rules for one game interleaved by specificity would answer `choose` with a rule neither of them expected to win.

**A rule never has to name the site it downloads from.** `download_hosts` adds `api_base()`'s host to every app rule's net allow-list, because `{fileUrl}` comes from the API and Rust picked the base — an author naming it restates a constant, and one who gets it wrong writes a rule that cannot download and finds out when somebody clicks install. (When the shipped rules named the production host, a build pointed at the dev site refused every download.) The user still sees the host — `permission_summary` reads the manifest this builds.

### The games that ship

**The canonical list is the README's table** (one directory per game under `plugins/app/`) — which game, where its mods land, and whether anybody has actually run it. Repeating it here would be a second list to keep in step, and the one in the README is the one a user reads.

What belongs here instead is the shape of those rules, because it is what the next one has to fit into:

| Family | Slugs | Mods land in | Default |
| --- | --- | --- | --- |
| Open-world RPGs + script extender | `skyrim`, `skyrim-se`, `skyrim-vr`, `fallout-3`, `fallout-4`, `fallout-new-vegas`, `oblivion`, `starfield`, `morrowind` | `Data/`, plus `Data/<XSE>/Plugins` for the extender's own DLLs | `hardlink` |
| BepInEx | `valheim`, `vrising`, `sotf` | `BepInEx/plugins`, `patchers`, `config` | `hardlink` |
| One engine's `custom`/`addons` folder | `tf2`, `gmod` | `tf/custom`, `garrysmod/addons` | `hardlink` |
| Plain `Mods/` folder | `rimworld`, `stardew-valley`, `mount-and-blade-2`, `witcher-3`, `7-days-to-die`, `palworld`, `kerbal-space-program`, `cyberpunk-2077` | one folder, named by the game | `hardlink` |
| Instance model | `minecraft` | `mods`, `resourcepacks`, `shaderpacks`, `datapacks`, `config` | `direct` |
| Documents, not the install | `sims-3`, `sims-4`, `baldurs-gate-3` | the user's documents folder | `direct` |
| Game root | `gtav`, `gta-3`, `gta-4`, `gta-san-andreas`, `gta-vice-city` | `modloader/`, `scripts/`, or the root itself | `direct` |
| Server only | `rust` | `oxide/plugins`, `oxide/config`, `oxide/data` | `direct` |

Four of those rows carry a decision rather than a path:

- **`antiCheat: "kernel"` is set on `gtav` and `rust`**, which is what forces Direct and warns on every other option. Guessing wrong there costs somebody their account, not an install.
- **`rust` is the dedicated SERVER.** Its anti-cheat is kernel-level, there is no client-side modding of it, and inventing one would be inventing a ban.
- **The three documents-folder games are why a sandbox's `gameDir` is not "where Steam put it".** The install folder holds no mods at all, so a sandbox pointed at it deploys perfectly and changes nothing.
- **`gmod`'s Workshop subscriptions are the game's own business**, not a sandbox's — the rule covers `addons/`, `lua/autorun` and the workshop cache, and does not try to own what Steam already manages.

**`minecraft` is the instance model, deliberately.** A sandbox given its own game folder gets its own `mods`, `config`, `saves` and `options.txt`, the way the instance-based launchers all do it — so `defaultStrategy` is `direct`. Copying is not laziness there: the launcher is pointed at the folder with `--workDir` and everything inside is expected to be a real file, loaders resolve paths, some mods rewrite their own jar on first run, and Windows refuses to make a symbolic link at all without Developer Mode. Hard links are offered for somebody running several packs that share mods off one drive.

Everything else defaults to `hardlink`, which is the app's own default and right when a game folder is shared rather than instanced.
