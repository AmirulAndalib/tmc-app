# Downloads

Every file the app fetches goes through `download::DownloadManager` — a mod's release archive, a sandbox's staging, whatever a plugin's `download` step names. It is a subsystem rather than a function because a modpack is not one download, it is four hundred, over a home connection, on a laptop that will be closed halfway through.

```text
  enqueue ──▶ queued ──▶ running ──┬──▶ done
                 ▲         │       ├──▶ failed ──▶ (retry) ──▶ queued
                 └─────────┴───────┴──▶ paused ──▶ (resume) ──▶ queued
```

Three guarantees worth naming:

- **The destination filename only ever appears after the last byte and the checksum.** Progress lives in a `.tmcpart` file beside the target, so nothing downstream can pick up a partial file and treat it as complete.
- **A `200` in answer to a range request means the server ignored it**, so the file restarts rather than being appended to. Appending produces a file that is too long, passes every length check, and fails its checksum with an error nobody can explain.
- **A resumed file that fails its checksum starts again from zero, once**: a prefix left by an older version of the file passes every length check. A `.tmcpart` that is already the whole file (the app died before the rename) answers `416` and is finished by its checksum rather than discarded.
- **The checksum is computed from the FILE, not incrementally.** An incremental hash cannot survive a resume — the bytes from the first attempt never pass through this process — and a checksum that silently stops being checked on resumed downloads is worse than none.

**Downloading one named release goes through the queue too.** An item's page lists its version history, and both its buttons — the header's "Download 1.4" and each row's — enqueue rather than handing the URL to the system browser: somebody fetching an older release because the newest one broke their save is exactly the person who wants it pausable. `download_release` takes a kind and two ids, **never a URL and never a path**: Rust re-fetches the detail, picks the file, and puts it in the user's download folder. So the rule at the top of `commands/downloads.rs` still holds — there is no `download_start(url, path)`, and nothing an injected script could point at a file of its choosing.

**A plugin's `download` step goes through the queue** when the executor has a handle to one, with a deterministic id derived from the plugin and the destination — so re-running a failed install is the same row rather than a second writer for one file. The host allow-list, the size cap and the checksum are enforced on both paths; the queue moves where the bytes are read, not which checks run.

**The queue gets its own HTTP client, not the API's.** The API client attaches a bearer to everything it sends, and a mod archive comes from a CDN that has no business seeing one — a redirect to a third-party mirror would hand out an access token.

Bandwidth is a token bucket (`download::rate`), composed as global × per download, both live-adjustable. A request larger than the bucket can hold is served by taking the balance negative rather than refused, because a downloader whose chunk size it does not control would otherwise hang.

The device reports a SNAPSHOT of its queue to `/api/app/v1/downloads` so the website can show it. That row never drives anything — there is deliberately no endpoint that tells a device to pause or cancel — and it carries no URLs and no filesystem paths.
