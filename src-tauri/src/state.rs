use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use tauri::AppHandle;

use tmc_core::api::ApiClient;
use tmc_core::auth::AuthState;
use tmc_core::crypto::LocalCipher;
use tmc_core::download::DownloadManager;
use tmc_core::error::{AppError, AppResult};
use tmc_core::launch::{plan as build_launch_plan, LaunchContext, LaunchOptions, LaunchPlan};
use tmc_core::library::LibraryDb;
use tmc_core::logging::Audit;
use tmc_core::net::LatencyStore;
use tmc_core::plugins::apps::AppPlugins;
use tmc_core::plugins::registry::Registry;
use tmc_core::plugins::{jail_for, JailRoots};
use tmc_core::rcon::RconPool;
use tmc_core::secure::SecureStore;
use tmc_core::settings::{AppSettings, SettingsStore};

use crate::paths::AppPaths;

/// Everything the commands need, assembled once at startup.
///
/// Held behind `tauri::State`, so every command gets the same instances — which
/// matters for the ones holding locks (the refresh mutex, the audit writer) or
/// caches (the plugin registry, the latency history). Constructing per-call
/// would quietly break the single-refresh guarantee in [`ApiClient`] and reset
/// every latency graph on each render.
pub struct AppState {
    pub paths: AppPaths,
    pub settings: SettingsStore,
    pub audit: Arc<Audit>,
    pub secure: Arc<SecureStore>,
    pub auth: Arc<AuthState>,
    pub api: ApiClient,
    pub plugins: Registry,
    pub latency: LatencyStore,
    pub version: String,

    /// The device's library: which subscriptions are here, what is on disk,
    /// every sandbox, the deployment ledger and the download queue.
    pub library: Arc<LibraryDb>,

    /// The download queue. One per process — a second would mean two schedulers
    /// racing for the same concurrency budget and two writers per `.part` file.
    pub downloads: DownloadManager,

    /// Open RCON sessions, pooled so a console is a conversation rather than a
    /// reconnect per line.
    pub rcon: RconPool,

    /// The key that encrypts RCON passwords.
    ///
    /// Created LAZILY, on first use. A user who never adds a server never has a
    /// key — so there is nothing to steal, nothing to back up and nothing to
    /// migrate — and the credential-store round trip does not happen on a
    /// launch that will not need it.
    cipher: std::sync::OnceLock<LocalCipher>,

    /// App-scoped install and launch rules (`plugins/app/<slug>/…`).
    ///
    /// An `Arc` behind an `RwLock` so a reload swaps the whole set in one write
    /// while every reader keeps the snapshot it started with. Handing out a
    /// GUARD instead would mean an install holding a read lock for the length
    /// of a download, and a reload blocking behind it.
    app_plugins: RwLock<Arc<AppPlugins>>,
}

impl AppState {
    pub fn build(app: &AppHandle, version: String) -> AppResult<Self> {
        let paths = AppPaths::resolve(app)?;

        let settings = SettingsStore::load(paths.settings_file());
        let current = settings.get();

        let audit = Arc::new(Audit::new(paths.audit_file()));
        audit.set_verbose(current.verbose_logging);

        let secure = Arc::new(SecureStore::new(&paths.data));

        let auth = Arc::new(AuthState::new());
        auth.restore(&secure);

        let api = ApiClient::new(Arc::clone(&auth), Arc::clone(&secure), &version)?;

        /*
         * The download manager gets its OWN http client rather than the API's.
         * The API client attaches a bearer to everything it sends, and a mod
         * file comes from a CDN that has no business seeing one — a redirect to
         * a third-party mirror would hand somebody's access token to a host we
         * do not control.
         */
        let file_http = tmc_core::download::default_client(&version)?;

        let plugins = Registry::load(paths.registry_file(), paths.plugins.clone());

        let library = Arc::new(LibraryDb::open(paths.library_file())?);

        /*
         * Anything left mid-flight by a crash is reset here. A row stuck at
         * `installing` is not a state anything can act on — the process that
         * owned it is gone — and leaving it means the item never installs again
         * while the UI shows a spinner forever.
         */
        match library.reset_transient_states() {
            Ok(0) => {}
            Ok(n) => tracing::warn!("reset {n} interrupted library operation(s)"),
            Err(e) => tracing::warn!("could not reset library states: {}", e.detail()),
        }

        /*
         * A download left `running` by a crash comes back `queued`. Same
         * reasoning as the library's transient states: the process that owned
         * it is gone, so nothing will ever move it and the UI would show a
         * stalled bar forever.
         */
        match library.download_reset_running() {
            Ok(0) => {}
            Ok(n) => tracing::warn!("requeued {n} interrupted download(s)"),
            Err(e) => tracing::warn!("could not requeue downloads: {}", e.detail()),
        }

        let app_plugins = AppPlugins::load(&paths.plugins);

        for (source, error) in app_plugins.errors() {
            tracing::warn!("app plugin {source} did not load: {error}");
        }

        let downloads = DownloadManager::new(file_http, Arc::clone(&audit));
        let rcon = RconPool::new(Arc::clone(&audit));

        Ok(Self {
            paths,
            settings,
            audit,
            secure,
            auth,
            api,
            plugins,
            latency: LatencyStore::new(),
            version,
            downloads,
            rcon,
            cipher: std::sync::OnceLock::new(),
            library,
            app_plugins: RwLock::new(Arc::new(app_plugins)),
        })
    }

    /// The key that encrypts RCON passwords, created on first use.
    ///
    /// Every failure is the same one — the credential store is unreachable, or
    /// the stored key is corrupt — and it is worth surfacing rather than
    /// silently minting a replacement, which would turn "the entry is broken"
    /// into "every saved password is wrong".
    pub fn cipher(&self) -> AppResult<&LocalCipher> {
        if let Some(cipher) = self.cipher.get() {
            return Ok(cipher);
        }

        let created = LocalCipher::load_or_create(&self.secure)?;

        // A race here means two keys were generated and one is discarded — but
        // `load_or_create` writes before returning, so both loaded the SAME key
        // and the loser is identical to the winner.
        let _ = self.cipher.set(created);

        self.cipher
            .get()
            .ok_or_else(|| AppError::internal("cipher was not initialised"))
    }

    /// Everything a sandbox operation needs, assembled from this state.
    pub fn sandbox_ctx<'a>(
        &'a self,
        plugins: &'a tmc_core::plugins::apps::AppPlugins,
        settings: &'a AppSettings,
        roots: &'a tmc_core::plugins::JailRoots,
        staging: &'a std::path::Path,
        backups: &'a std::path::Path,
    ) -> tmc_core::library::deploy::SandboxCtx<'a> {
        tmc_core::library::deploy::SandboxCtx {
            plugins,
            roots,
            settings,
            http: self.api.raw(),
            audit: &self.audit,
            downloads: Some(&self.downloads),
            staging_root: staging,
            backup_root: backups,
        }
    }

    /// The platform directories game detection reads.
    ///
    /// From Tauri's resolver, never from an environment variable — the same
    /// rule every other path in this app follows, and the reason `tmc_core::
    /// detect` takes them as an argument instead of finding them itself.
    pub fn detect_roots(&self, app: &AppHandle) -> tmc_core::detect::DetectRoots {
        use tauri::Manager;

        let resolver = app.path();

        #[allow(unused_mut)]
        let mut program_files: Vec<PathBuf> = Vec::new();
        #[allow(unused_mut)]
        let mut drives: Vec<PathBuf> = Vec::new();

        #[cfg(windows)]
        {
            for letter in b'A'..=b'Z' {
                let root = PathBuf::from(format!("{}:\\", letter as char));

                if !root.is_dir() {
                    continue;
                }

                for rel in ["Program Files", "Program Files (x86)"] {
                    let candidate = root.join(rel);

                    if candidate.is_dir() {
                        program_files.push(candidate);
                    }
                }

                drives.push(root);
            }
        }

        tmc_core::detect::DetectRoots {
            home: resolver.home_dir().ok(),
            config: resolver.config_dir().ok(),
            local_data: resolver.local_data_dir().ok(),
            program_data: program_data_dir(),
            program_files,
            drives,
        }
    }

    /// The directories a plugin jail may be anchored to.
    ///
    /// Resolved here rather than in `tmc-core` because the answer is
    /// platform-specific and Tauri's resolver is the only thing that knows it.
    /// The core decides what a plugin may *do* with them.
    pub fn jail_roots(&self) -> JailRoots {
        JailRoots {
            data: self.paths.data.clone(),
            cache: self.paths.cache.clone(),
        }
    }

    /// A snapshot of the app-scoped rules.
    ///
    /// Cloning the `Arc`, not the rules. A caller keeps the set it started with
    /// for as long as it holds the handle, which is exactly what an install
    /// that takes two minutes wants — a rule reload mid-download must not
    /// change what is being installed.
    pub fn app_plugins(&self) -> Arc<AppPlugins> {
        self.app_plugins
            .read()
            .map(|g| Arc::clone(&g))
            .unwrap_or_else(|e| Arc::clone(&e.into_inner()))
    }

    /// Re-read `plugins/app/`. Cheap, and the only way to pick up a rule the
    /// user just dropped in without restarting.
    pub fn reload_app_plugins(&self) {
        let fresh = Arc::new(AppPlugins::load(&self.paths.plugins));

        if let Ok(mut slot) = self.app_plugins.write() {
            *slot = fresh;
        }
    }

    pub fn settings_snapshot(&self) -> AppSettings {
        self.settings.get()
    }

    // ------------------------------------------------------------- Installs

    /// One mirrored install's payload, parsed.
    fn install_payload(&self, id: i64) -> Option<serde_json::Value> {
        self.library
            .install_payloads()
            .ok()?
            .into_iter()
            .filter_map(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .find(|v| v.get("id").and_then(serde_json::Value::as_i64) == Some(id))
    }

    /// The default install for an app, if one has been mirrored.
    pub fn default_install_for(&self, app_id: i64) -> Option<i64> {
        self.library
            .install_payloads()
            .ok()?
            .into_iter()
            .filter_map(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .find(|v| {
                v.get("appId").and_then(serde_json::Value::as_i64) == Some(app_id)
                    && v.get("isDefault")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
            })
            .and_then(|v| v.get("id").and_then(serde_json::Value::as_i64))
    }

    /// The four facts an install contributes to an install run: its directory
    /// on THIS machine, its loader, its game version and its name.
    pub fn install_facts(
        &self,
        id: i64,
    ) -> (
        Option<PathBuf>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        let dir = self
            .library
            .install_local_dir(id)
            .ok()
            .flatten()
            .map(PathBuf::from);

        let Some(payload) = self.install_payload(id) else {
            return (dir, None, None, None);
        };

        let string = |key: &str| {
            payload
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };

        (dir, string("loader"), string("gameVersion"), string("name"))
    }

    // --------------------------------------------------------------- Launch

    /// Resolve what launching an install would run.
    ///
    /// Everything is looked up here rather than accepted from the caller: the
    /// rule, the game directory, the options. A frontend that could supply any
    /// of them would make every check in `tmc_core::launch` advisory.
    pub fn launch_plan(&self, install_id: i64) -> AppResult<LaunchPlan> {
        let payload = self
            .install_payload(install_id)
            .ok_or_else(|| AppError::invalid("That install is not on this device yet."))?;

        let app_id = payload
            .get("appId")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| AppError::invalid("That install names no app."))?;

        let slug = payload
            .get("app")
            .and_then(|a| a.get("slug"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| AppError::invalid("That install's app has no slug."))?;

        let (dir, loader, game_version, name) = self.install_facts(install_id);

        let plugins = self.app_plugins();

        let rule = plugins
            .launch_for(&slug, loader.as_deref())
            .ok_or_else(|| AppError::invalid("This app has no launch rule."))?;

        /*
         * The install's own directory becomes the `gameDir` root, exactly as it
         * does for an install run — the same mechanism rather than a second
         * one, so a profile can never end up with laxer path checks than the
         * main install.
         */
        let mut settings = self.settings.get();

        if let Some(dir) = &dir {
            settings
                .game_dirs
                .insert(app_id.to_string(), dir.display().to_string());
        }

        let manifest = rule.as_manifest();
        let roots = self.jail_roots();

        let jail = jail_for(&manifest, &roots, &settings, Some(app_id))?;

        let options: LaunchOptions = payload
            .get("options")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let ctx = LaunchContext {
            install_name: name,
            game_version,
            loader,
            install_dir: dir,
            game_dir: jail
                .root_path(tmc_core::plugins::manifest::FsRoot::GameDir)
                .map(std::path::Path::to_path_buf),
        };

        let mut plan = build_launch_plan(rule, &jail, &options, &ctx)?;

        /*
         * The install's own `launchArgs` and `launchEnv` are appended AFTER the
         * rule's. The rule knows how to start the game; the user's extra flags
         * are theirs, and a game's argument parser takes the last occurrence of
         * a repeated flag — so appending is what makes an override actually
         * override.
         */
        if let Some(extra) = payload
            .get("launchArgs")
            .and_then(serde_json::Value::as_array)
        {
            for value in extra {
                let Some(arg) = value.as_str() else { continue };

                // Same refusal the rule's own arguments get. A user-typed
                // argument is not more trusted than a plugin-supplied one.
                if arg.contains('\0') || arg.contains('\n') || arg.contains('\r') {
                    return Err(AppError::jail(
                        "A launch argument contains a control character.",
                    ));
                }

                plan.args.push(arg.to_string());
            }
        }

        if let Some(env) = payload
            .get("launchEnv")
            .and_then(serde_json::Value::as_object)
        {
            for (key, value) in env {
                let Some(value) = value.as_str() else {
                    continue;
                };

                if key.contains('\0') || value.contains('\0') {
                    return Err(AppError::jail(
                        "A launch environment value contains a NUL byte.",
                    ));
                }

                plan.env.insert(key.clone(), value.to_string());
            }
        }

        if plan.args.len() > 128 {
            return Err(AppError::invalid(
                "That launch produces too many arguments.",
            ));
        }

        Ok(plan)
    }
}

/// `%ProgramData%`, which Tauri's resolver does not expose.
///
/// Derived from the system drive rather than read from the environment, for the
/// same reason `api_base` is not read from one in a release build: the
/// environment of the process that launched the app is not a trust boundary,
/// and a shortcut's "Start in" can set anything. It is only ever used to LOOK
/// for a launcher's manifests, never to write.
fn program_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        for letter in b'C'..=b'Z' {
            let candidate = PathBuf::from(format!("{}:\\ProgramData", letter as char));

            if candidate.is_dir() {
                return Some(candidate);
            }
        }

        None
    }

    #[cfg(not(windows))]
    {
        None
    }
}
