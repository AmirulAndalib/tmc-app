# Prior art

This app is not the first mod manager, and the comments throughout it name the others on purpose: Mod Organizer's virtual filesystem, Vortex's hard-link deployment, r2modman's and CurseForge's per-profile instances, Gale's config form. Where one of them settled a question well, the reasoning is credited to them at the point it applies, so the next person to read that code learns why the shape is what it is rather than assuming it was arbitrary.

**Credited, not copied.** Every line in this repository is written for it. What is taken from other projects is the same thing any engineer takes from published work — the knowledge that a problem exists and roughly how it tends to be solved — and that is exactly what the comments record. Three specific categories are worth separating, because they are treated differently:

- **Protocol and file-format behaviour** is fact about somebody else's wire format or on-disk layout, and the comments name the authority that settled it — `spy`'s scanner for query ports, `go-a2s` for split-reply framing, each game's own modding documentation for where its mods go. A format is not authorship, and getting it wrong is a bug in this app.
- **Folder layouts in `plugins/manager/`** are the documented defaults each manager publishes for its own users. They are data in this repository because they belong to those projects and change on their schedule — a corrected descriptor is a JSON edit, not a patch to Rust.
- **Nothing is vendored.** No third-party source is copied into this tree, and nothing here is a derivative work of another manager. Dependencies are declared in `package.json` and the `Cargo.toml`s, under their own licences.

If a comparison in a comment ever reads as a claim on somebody else's work rather than a note about why this code looks the way it does, rewrite it. The projects named here are the reason a lot of this was tractable, and the attribution is meant to say so.
