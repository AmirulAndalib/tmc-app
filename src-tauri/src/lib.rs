mod commands;
mod paths;
mod state;

use tauri::Manager;

use tmc_core::audit;

use crate::state::AppState;

/// Cache is scratch space for in-flight downloads. Clearing it on launch means
/// a crash mid-install cannot leave a half-written archive that a later run
/// picks up as if it were complete.
fn clear_cache(paths: &paths::AppPaths) {
    let downloads = paths.cache.join("downloads");

    if downloads.exists() {
        let _ = std::fs::remove_dir_all(&downloads);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tmc_app_lib=info,tmc_core=info,warn".into()),
        )
        .init();

    /*
     * `tauri-plugin-dialog` is deliberately NOT registered.
     *
     * Its folder picker is the platform's, which on Linux is a GTK file chooser
     * — themed by whatever desktop the user is running and matching neither the
     * app nor the other four targets. Folder choosing is `components/
     * folder-picker` over `commands::fs`, which looks the same everywhere; see
     * that module's header for what listing directories from the webview costs.
     */
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init());

    /*
     * Deep links accelerate the login: the browser bounces to `tmc://auth`
     * after approval so the app polls immediately instead of waiting out its
     * interval. Registration is desktop-only here — on Android and iOS the
     * scheme is declared in the platform manifests that `tauri android init` /
     * `tauri ios init` generate, and calling the desktop registrar there is
     * both unnecessary and unsupported.
     *
     * The link is a WAKE-UP, never a credential. Nothing is read out of the
     * URL; the app polls the API exactly as it would have on its timer. That is
     * deliberate — a custom scheme can be claimed by any other app on the
     * machine, so anything carried in one is public.
     *
     * Shadowed rather than reassigned, so `builder` needs no `mut` on the
     * mobile targets where this is compiled out.
     */
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_deep_link::init());

    builder
        .setup(|app| {
            let handle = app.handle().clone();
            let version = app.package_info().version.to_string();

            let state = AppState::build(&handle, version)
                .map_err(|e| format!("could not start: {}", e.detail()))?;

            clear_cache(&state.paths);

            audit!(
                state.audit,
                Info,
                App,
                "app.start",
                format!("TMC {} started", state.version)
            );

            /*
             * A build aimed at anything but the real site says so in the audit
             * trail, at Security level so turning logging off cannot hide it.
             * Where an account's credentials are being sent is exactly the kind
             * of fact the log exists to hold, and "which site was this?" is the
             * first question about any screenshot from a dev build.
             */
            if !tmc_core::api::api_base_is_prod() {
                audit!(
                    state.audit,
                    Security,
                    App,
                    "app.api_base",
                    format!("Talking to {} — NOT production", tmc_core::api::api_base())
                );
            }

            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;

                // Best effort: a failed registration means the accelerator does
                // not work, not that the app cannot sign in.
                let _ = app.deep_link().register("tmc");

                let emitter = app.handle().clone();

                app.deep_link().on_open_url(move |event| {
                    use tauri::Emitter;
                    use tmc_core::deeplink::{parse, DeepLink};

                    for url in event.urls() {
                        /*
                         * A link the app does not understand does NOTHING. It
                         * does not fall through to the auth wake-up and it does
                         * not reach the webview — anything on the machine can
                         * claim a custom scheme, and a default action is a
                         * default action an attacker gets to trigger.
                         */
                        let Some(link) = parse(url.as_str()) else {
                            tracing::debug!("ignoring unrecognised deep link");

                            continue;
                        };

                        match link {
                            // Still a wake-up carrying nothing: it makes the
                            // app poll now instead of waiting out its interval.
                            DeepLink::Auth => {
                                let _ = emitter.emit("tmc://auth-return", ());
                            }
                            /*
                             * Everything else asks the app to SHOW a screen.
                             * Never to act — see `tmc_core::deeplink`. The
                             * webview navigates and the user presses the button
                             * themselves, or does not.
                             */
                            other => {
                                let _ = emitter.emit("tmc://open", &other);
                            }
                        }
                    }
                });
            }

            app.manage(state);

            /*
             * The download queue's bridge to the webview, and the queue itself
             * restored from the last session. Both after `manage`, because both
             * reach the state through the handle.
             */
            commands::downloads::spawn_bridge(app.handle().clone());

            let restore_handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                use tauri::Manager;

                commands::downloads::restore(&restore_handle.state::<AppState>()).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::session_state,
            commands::auth::auth_begin,
            commands::auth::auth_poll,
            commands::auth::auth_cancel,
            commands::auth::auth_sign_out,
            commands::api::api_get,
            commands::api::api_send,
            commands::api::api_env,
            commands::settings::settings_get,
            commands::settings::settings_patch,
            commands::settings::settings_set_game_dir,
            commands::settings::settings_set_download_dir,
            commands::settings::settings_reset,
            commands::fs::fs_roots,
            commands::fs::fs_list_dirs,
            commands::logs::log_read,
            commands::logs::log_clear,
            commands::logs::log_path,
            commands::servers::ping_server,
            commands::servers::query_server,
            commands::servers::query_servers,
            commands::servers::latency_series,
            commands::servers::latency_clear,
            commands::plugins::plugin_list,
            commands::plugins::plugin_inspect,
            commands::plugins::plugin_approve,
            commands::plugins::plugin_set_enabled,
            commands::plugins::plugin_remove,
            commands::plugins::plugin_theme,
            commands::plugins::plugin_run,
            commands::plugins::plugin_query_server,
            commands::library::library_list,
            commands::library::library_sync,
            commands::library::library_install,
            commands::library::library_uninstall,
            commands::library::library_installs,
            commands::library::library_install_dirs,
            commands::library::library_set_install_dir,
            commands::library::library_supported_apps,
            commands::library::library_plugin_errors,
            commands::library::library_reload_plugins,
            commands::library::library_rules_for,
            commands::library::launch_preview,
            commands::library::launch_install,
            commands::library::launch_available,
            commands::sandbox::sandbox_list,
            commands::sandbox::sandbox_get,
            commands::sandbox::sandbox_create,
            commands::sandbox::sandbox_patch,
            commands::sandbox::sandbox_delete,
            commands::sandbox::sandbox_set_default,
            commands::sandbox::sandbox_add_mod,
            commands::sandbox::sandbox_remove_mod,
            commands::sandbox::sandbox_set_mod_enabled,
            commands::sandbox::sandbox_reorder,
            commands::sandbox::sandbox_stage,
            commands::sandbox::sandbox_deploy,
            commands::sandbox::sandbox_purge,
            commands::sandbox::sandbox_verify,
            commands::sandbox::sandbox_strategies,
            commands::sandbox::sandbox_spec,
            commands::sandbox::sandbox_check,
            commands::sandbox::sandbox_refresh_dependencies,
            commands::sandbox::sandbox_add_missing,
            commands::downloads::download_list,
            commands::downloads::download_pause,
            commands::downloads::download_resume,
            commands::downloads::download_cancel,
            commands::downloads::download_set_priority,
            commands::downloads::download_set_limit,
            commands::downloads::download_set_global_limit,
            commands::downloads::download_set_concurrency,
            commands::downloads::download_clear_finished,
            commands::detect::detect_games,
            commands::detect::detect_apply,
            commands::rcon::rcon_list,
            commands::rcon::rcon_create,
            commands::rcon::rcon_update,
            commands::rcon::rcon_set_password,
            commands::rcon::rcon_delete,
            commands::rcon::rcon_connect,
            commands::rcon::rcon_disconnect,
            commands::rcon::rcon_is_connected,
            commands::rcon::rcon_exec,
            commands::rcon::rcon_history,
            commands::rcon::rcon_clear_history,
            commands::rcon::rcon_suggest_protocol,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
