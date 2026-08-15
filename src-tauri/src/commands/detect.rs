//! Finding installed games, and offering what was found.
//!
//! **Detection suggests; the user applies.** `detect_games` is read-only and
//! returns candidates. `detect_apply` is what actually points a game directory
//! somewhere, and it runs the same [`tmc_core::anchor::validate_root`] a
//! hand-typed path does — a folder is not more trustworthy for having been
//! found automatically, and a game directory is a jail anchor.
//!
//! The two are separate commands rather than one for exactly that reason: a
//! scan that configured as it went would be a scan that could not be shown to
//! anybody first.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, State};

use tmc_core::detect::{DetectReport, DetectedGame};
use tmc_core::error::{AppError, AppResult};
use tmc_core::settings::AppSettings;

use crate::state::AppState;

/// A candidate, plus what applying it would mean.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    #[serde(flatten)]
    pub game: DetectedGame,
    /// Is this folder already configured for its game?
    pub already_set: bool,
    /// Would applying it replace a different folder?
    pub replaces: Option<String>,
}

/// Everything this machine has installed that the app can recognise.
///
/// The scan touches the filesystem, so it runs off the UI thread. It is not
/// cached: somebody opens this screen because they just installed something.
#[tauri::command]
pub async fn detect_games(app: AppHandle, state: State<'_, AppState>) -> AppResult<Vec<Candidate>> {
    let roots = state.detect_roots(&app);
    let plugins = state.app_plugins();
    let settings = state.settings.get();

    let report: DetectReport =
        tauri::async_runtime::spawn_blocking(move || tmc_core::detect::scan(&roots, &plugins))
            .await
            .map_err(|e| AppError::internal(format!("detect: {e}")))?;

    Ok(report
        .games
        .into_iter()
        .map(|game| annotate(game, &settings, &state))
        .collect())
}

fn annotate(game: DetectedGame, settings: &AppSettings, state: &AppState) -> Candidate {
    let configured = game
        .slug
        .as_deref()
        .and_then(|slug| app_id_for(slug, state))
        .and_then(|id| settings.game_dirs.get(&id.to_string()).cloned());

    let already_set = configured
        .as_deref()
        .is_some_and(|current| same_folder(current, &game.path));

    Candidate {
        already_set,
        replaces: configured.filter(|_| !already_set),
        game,
    }
}

/// Point a game's directory at a detected folder.
///
/// Takes the SLUG and the path, and resolves the app id itself — the frontend
/// naming an app id and a path together is how a mismatched pair ends up
/// pointing one game's installer at another game's folder.
#[tauri::command]
pub fn detect_apply(state: State<'_, AppState>, slug: String, path: String) -> AppResult<()> {
    let app_id = app_id_for(&slug, &state).ok_or_else(|| {
        AppError::invalid("The app does not know that game — nothing would use this folder.")
    })?;

    let (_, stored) =
        state
            .settings
            .set_game_dir(&app_id.to_string(), Some(&path), &protected(&state))?;

    tmc_core::audit!(
        state.audit,
        Security,
        App,
        "detect.apply",
        format!("{slug} → {}", stored.clone().unwrap_or_default())
    );

    Ok(())
}

/// Which TMC app id a slug belongs to on this device.
///
/// From the sandboxes and the library, which are the only places the app learns
/// the mapping — a slug is the plugin folder's name and an id is the API's.
fn app_id_for(slug: &str, state: &AppState) -> Option<i64> {
    if let Ok(sandboxes) = state.library.sandbox_list(None) {
        if let Some(found) = sandboxes
            .iter()
            .find(|s| s.app_slug.as_deref() == Some(slug))
        {
            return Some(found.app_id);
        }
    }

    state
        .library
        .list()
        .ok()?
        .into_iter()
        .find(|e| e.app_slug.as_deref() == Some(slug))
        .and_then(|e| e.app_id)
}

/// Are these the same directory, allowing for a symlink or a trailing slash?
fn same_folder(a: &str, b: &str) -> bool {
    let canon = |p: &str| {
        std::path::Path::new(p)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(p))
    };

    canon(a) == canon(b)
}

fn protected(state: &AppState) -> Vec<PathBuf> {
    vec![
        state.paths.data.clone(),
        state.paths.logs.clone(),
        state.paths.cache.clone(),
        state.paths.plugins.clone(),
        state.paths.staging_dir(),
        state.paths.backup_dir(),
    ]
}
