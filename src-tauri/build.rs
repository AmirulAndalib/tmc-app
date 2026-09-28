fn main() {
    /*
     * The updater's public key and endpoint are compiled in, so a rebuild has
     * to notice when either changes.
     *
     * Without this, a build that was first made with no key keeps having no key
     * forever — cargo does not know an `option_env!` was read — and the symptom
     * is an app that silently has no updater after somebody has configured one.
     * The same reasoning `core/build.rs` carries for `TMC_API_BASE`.
     */
    println!("cargo:rerun-if-env-changed=TMC_UPDATER_PUBKEY");
    println!("cargo:rerun-if-env-changed=TMC_UPDATER_ENDPOINT");
    println!("cargo:rerun-if-changed=updater.pub");

    /*
     * THE KEY IS IN THE REPOSITORY, SO EVERY BUILD HAS ONE.
     *
     * `updater.pub` is the public half of the release signing key — public by
     * definition, and the private half lives only in the release workflow's
     * secrets. Reading it here means a developer's `npm run desktop:build`
     * produces an app that verifies updates exactly as a release does, instead
     * of one that silently has no updater. `TMC_UPDATER_PUBKEY` in the
     * environment still wins, so a fork signing its own releases sets that and
     * never edits this file.
     *
     * An empty or missing file is no key, not a placeholder: the app then has
     * no updater until one is configured in Settings.
     */
    let from_env = std::env::var("TMC_UPDATER_PUBKEY")
        .ok()
        .filter(|k| !k.trim().is_empty());

    if from_env.is_none() {
        if let Ok(key) = std::fs::read_to_string("updater.pub") {
            let key = key.trim();

            if !key.is_empty() {
                println!("cargo:rustc-env=TMC_UPDATER_PUBKEY={key}");
            }
        }
    }

    tauri_build::build()
}
