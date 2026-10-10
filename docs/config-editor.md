# Editing a game's own settings

Every mod manager studied for this has a config editor, and every one exists for the same moment: a loader wrote `BepInEx/config/com.author.mod.cfg` the first time the game ran, one value in it is wrong, and fixing it meant leaving the manager.

## What the jail is for here, and what it is not

Worth stating exactly, because it is easy to overstate in both directions.

**The app process has the user's filesystem permissions and must.** `deploy::link` writes into game folders; the plugin executor unpacks archives into them. Nothing about a config editor changes that and nothing needed to.

What these commands add is the ability for the **webview** to name a file to be read or written — the rule at the top of `commands/mod.rs`, and the reason a mod description rendered next to `invoke` is a nuisance rather than a problem. So the surface is bounded the way every other privileged surface here is, and the cost is nothing anybody wanted: a config editor that could open `/etc/shadow` is not a better config editor.

## Two bounds, not one

| Bound | Catches |
| --- | --- |
| The declared locations compile to jail grants | A path outside the game's folders entirely |
| `config::locate` re-runs the spec's own rules on every read and write | A path inside a granted root that the game never offered |

The second is not redundant. A location naming the game's ROOT — which `{ path: "", files: ["options.txt"] }` legitimately does — grants the jail the whole game folder, so the jail alone would happily resolve `saves/world.dat`. The listing's own matching rules are therefore re-run on the requested path, which means the only files that can be opened are ones the list would have shown.

## `config.json`

```json
{
  "manifestVersion": 1,
  "config": {
    "locations": [
      { "label": "Mod settings", "root": "gameDir", "path": "config",
        "extensions": ["toml", "cfg"], "recursive": true },
      { "label": "Game options", "root": "gameDir", "path": "",
        "files": ["options.txt"] }
    ]
  }
}
```

- **A location is a FOLDER, never a single file.** Not a limitation — it is what stops the grant it compiles into destroying the thing it exposes. `Jail::build` pre-creates every writable grant's prefix, so a location naming `options.txt` would create a *directory* called `options.txt` where the game's settings file belongs. Name the folder; list the file in `files`.
- **`files` replaces the extension test rather than adding to it.** A location naming both would be asking two questions with one answer.
- **Grants are DERIVED from the locations**, not declared beside them. An author who adds a location and forgets a matching grant would otherwise get a folder that lists nothing, with no error anywhere.

## Why the editor is a plain text box

One manager has a schema-driven form with typed fields, and it is genuinely nicer where it works. It works by understanding BepInEx's own config dialect — and the moment a game uses TOML, an INI, a properties file or something bespoke, a form either cannot render it or renders it wrongly and writes back something the loader will not parse.

A text box round-trips every format exactly. The safety is elsewhere and is real: the previous contents are copied into the app's backup folder on every save, the write goes to a temporary file and is renamed (a half-written config is a game that will not start), and a file whose bytes are not UTF-8 is refused rather than mangled — reading one as lossy UTF-8 and writing it back replaces every invalid byte, which is an editor that destroys the file it was opened to fix.

The backup goes under the app's own directory rather than beside the file: a `.bak` next to a config is a file the game's own loader may try to parse, and one studied loader does exactly that.

## `allowConfigEditing`

ON by default — editing a loader's `.cfg` is ordinary work for a mod manager. It exists because this is the one feature that lets the webview name a file to be written, and **what it buys is bounded and is stated on the settings row itself**: it removes those three commands and nothing else. It does not sandbox the process, which writes to game folders whenever it deploys.
