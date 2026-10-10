# Finding the games

`detect::scan` reads what the launchers already wrote down rather than asking somebody to type a path.

| Source | Reads |
| --- | --- |
| Steam | `libraryfolders.vdf`, then every `appmanifest_*.acf` |
| Epic | `Data/Manifests/*.item`, plus Heroic/Legendary on Linux |
| GOG | `galaxy-2.0.db`, and the plain `GOG Games` folder |
| Xbox, Ubisoft, EA, Battle.net | their fixed folder layouts, one level deep |
| a game's own hints | `sandbox.json`'s `detect.paths` |

**Detection suggests; it never configures.** Applying a result goes through `anchor::validate_root` exactly as a hand-typed path does — a folder is not more trustworthy for having been found automatically, and a game directory is a jail anchor.

**It reads no environment variable.** The platform directories arrive as `DetectRoots`, filled by the Tauri crate from its path resolver.

Matching a folder to a TMC game, in descending order of confidence: the launcher's own id (exact, language-independent, survives a rename), then a marker file, then the normalised display name. A name match must also find the marker; an id match is not second-guessed.

Traps that are commented at the site:

- **Steam's `installdir` is not the display name.** A game's folder under `steamapps/common/` is usually an abbreviation of its title. Using the name is the single most common way to look in the wrong place.
- **A Steam manifest whose folder is gone is not an installed game.** Steam leaves them behind after a failed uninstall and after moving a game between libraries; offering one produces a game directory that fails every check the moment somebody accepts it.
- **GOG Galaxy's database may be open.** It is read with `mode=ro&immutable=1`; read-only alone still takes a shared lock and still reads the WAL, so a Galaxy mid-write either blocks the read or hands back a torn one.
- **Epic's `bIsIncompleteInstall` is the one field in its manifest that is not PascalCase.** `rename_all = "PascalCase"` derives `BIsIncompleteInstall`, which never matches and reads as "no game is ever incomplete", offering a folder that is still being written into. It carries an explicit `rename`.

## Finding games the launchers do not know about

`detect::scan` reads Steam's manifests, Epic's `.item` files and Galaxy's database. That is the right first answer and covers most machines. It cannot cover a game copied from another PC, a dedicated server unpacked by hand, or a drive that was moved.

`detect::walk` walks folders **the user ticked**. `roots` is a required argument with no default, and that is the design rather than an inconvenience: a scan of "the filesystem" is a scan of somebody's documents, their photos and every network share the machine has mounted, and a feature that did that by default would be indistinguishable from one looking for something else.

Bounded on four axes, each a real machine rather than a hypothetical:

| Bound | The machine it exists for |
| --- | --- |
| Depth | A game's own asset tree, which is where the hundred thousand files are |
| Directory count | A `node_modules` tree, a `/nix/store` |
| Wall clock | A network share that has gone away and answers `readdir` in thirty seconds |
| Never follows a symlink | `~/.wine/dosdevices/z: -> /`, which is a layout people have |

Two properties worth keeping:

- **A matched folder is not descended into.** Everything below a game's root is the game's own data.
- **An unmatched folder is not a finding.** That is what separates this from a file browser: a list of every directory on a drive is not a scan result.

`WalkStop` distinguishes "walked everything" from "hit a limit", because showing them identically is how somebody concludes their game is undetectable when the walk simply never reached it.

**Applying is still separate.** `detect_apply_many` runs each candidate through `detect_apply`'s own validation — a batch where one folder is refused must still apply the other eleven and say which one was not.
