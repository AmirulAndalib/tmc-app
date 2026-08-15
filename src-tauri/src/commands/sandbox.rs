//! The sandbox surface: create a profile, put mods in it, put it in front of
//! the game.
//!
//! The rules from `commands/mod.rs` hold, and one more that is specific to this
//! file and load-bearing: **the frontend never names a directory to deploy
//! into.** It names a sandbox by id; Rust looks up that sandbox's folder, or
//! the app's configured folder for the game, and everything below resolves
//! through the jail from there. A command that took a target path would make
//! every check in `anchor` and `deploy` advisory — an injected script in a mod
//! description could point a deploy at anything.
//!
//! The same applies to staging: a staging folder is derived from the sandbox
//! id, never supplied.

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use tmc_core::deploy::{PurgeReport, StrategyReport, VerifyReport};
use tmc_core::error::{AppError, AppResult};
use tmc_core::library::dependency::{DependencyReport, Edge, Relation};
use tmc_core::library::deploy::{
    deploy_sandbox, purge_sandbox, stage_mod, strategies_for, verify_sandbox, StageOutcome,
};
use tmc_core::library::sandbox::{NewSandbox, Sandbox, SandboxPatch};
use tmc_core::plugins::apps::SandboxSpec;

use crate::state::AppState;

/// A sandbox plus the answers every screen showing one wants.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxRow {
    #[serde(flatten)]
    pub sandbox: Sandbox,
    /// Has anything changed since the last deploy?
    pub needs_deploy: bool,
    /// Files this sandbox currently has in the game folder.
    pub deployed_files: usize,
    /// The folder it deploys into, or `None` when none is configured yet — the
    /// UI's cue to ask for one before anything else.
    pub target_dir: Option<String>,
}

fn row(sandbox: Sandbox, state: &AppState) -> AppResult<SandboxRow> {
    let ledger = state.library.sandbox_ledger(sandbox.id)?;
    let settings = state.settings.get();

    Ok(SandboxRow {
        needs_deploy: sandbox.needs_deploy(&ledger),
        deployed_files: ledger.len(),
        target_dir: tmc_core::library::deploy::target_dir(&sandbox, &settings)
            .ok()
            .map(|p| p.to_string_lossy().into_owned()),
        sandbox,
    })
}

/// Every sandbox, or one game's.
#[tauri::command]
pub fn sandbox_list(state: State<'_, AppState>, app_id: Option<i64>) -> AppResult<Vec<SandboxRow>> {
    state
        .library
        .sandbox_list(app_id)?
        .into_iter()
        .map(|s| row(s, &state))
        .collect()
}

#[tauri::command]
pub fn sandbox_get(state: State<'_, AppState>, id: i64) -> AppResult<Option<SandboxRow>> {
    match state.library.sandbox_get(id)? {
        Some(sandbox) => Ok(Some(row(sandbox, &state)?)),
        None => Ok(None),
    }
}

/// Create one, optionally from one of the game's presets.
///
/// The preset is applied HERE rather than in the webview: a preset carries a
/// deployment strategy and a set of option values, and letting the frontend
/// assemble those would mean a sandbox whose settings never went through the
/// game's own schema.
#[tauri::command]
pub fn sandbox_create(
    state: State<'_, AppState>,
    mut new: NewSandbox,
    preset: Option<String>,
) -> AppResult<SandboxRow> {
    let plugins = state.app_plugins();

    let spec = new
        .app_slug
        .as_deref()
        .and_then(|slug| plugins.sandbox_spec(slug));

    if let (Some(spec), Some(preset_id)) = (spec, preset.as_deref()) {
        let chosen = spec
            .preset(preset_id)
            .ok_or_else(|| AppError::invalid("That preset does not exist for this game."))?;

        new.preset = Some(chosen.id.clone());

        if let Some(env) = chosen
            .environment
            .as_deref()
            .and_then(tmc_core::library::sandbox::Environment::parse)
        {
            new.environment = env;
        }

        if let Some(strategy) = chosen
            .strategy
            .as_deref()
            .and_then(tmc_core::deploy::Strategy::parse)
        {
            new.strategy = strategy;
        }

        if chosen.game_version.is_some() {
            new.game_version = chosen.game_version.clone();
        }

        if chosen.loader.is_some() {
            new.loader = chosen.loader.clone();
        }

        // The schema's defaults first, then the preset's — so a preset that
        // sets one option does not silently clear the rest.
        let mut options = spec.default_options();

        options.extend(chosen.options.clone());

        new.options = options;
    }

    // A strategy the game says it does not support is refused at creation
    // rather than at deploy, when forty mods have already been assigned.
    if let Some(spec) = spec {
        if !spec.deploy.allows(new.strategy.as_str()) {
            return Err(AppError::invalid(format!(
                "This game does not support {} deployment.",
                new.strategy.as_str()
            )));
        }

        new.options = spec.clamp_options(&new.options);
    }

    let id = state.library.sandbox_create(&new)?;

    tmc_core::audit!(
        state.audit,
        Info,
        App,
        "sandbox.create",
        format!(
            "{} ({}, {})",
            new.name,
            new.environment.as_str(),
            new.strategy.as_str()
        )
    );

    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::internal("the sandbox vanished after being created"))?;

    row(sandbox, &state)
}

#[tauri::command]
pub fn sandbox_patch(
    state: State<'_, AppState>,
    id: i64,
    mut patch: SandboxPatch,
) -> AppResult<SandboxRow> {
    let existing = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();

    let spec = existing
        .app_slug
        .as_deref()
        .and_then(|slug| plugins.sandbox_spec(slug));

    if let Some(spec) = spec {
        if let Some(strategy) = patch.strategy {
            if !spec.deploy.allows(strategy.as_str()) {
                return Err(AppError::invalid(format!(
                    "This game does not support {} deployment.",
                    strategy.as_str()
                )));
            }
        }

        // Clamped against the game's own schema, so a value the webview sent
        // cannot reach a command line unbounded.
        if let Some(options) = &patch.options {
            patch.options = Some(spec.clamp_options(options));
        }
    }

    /*
     * A game folder set here goes through the anchor validator, exactly as a
     * global game directory does. It is the anchor of the jail for every
     * install into this sandbox, so it gets no more trust for having arrived on
     * a different command.
     */
    if let Some(Some(dir)) = &patch.game_dir {
        let canonical = tmc_core::anchor::validate_root(dir, &protected(&state))?;

        patch.game_dir = Some(Some(canonical.to_string_lossy().into_owned()));

        tmc_core::audit!(
            state.audit,
            Security,
            App,
            "sandbox.game_dir",
            format!("{} → {}", existing.name, canonical.display())
        );
    }

    state.library.sandbox_patch(id, &patch)?;

    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::internal("the sandbox vanished after being patched"))?;

    row(sandbox, &state)
}

/// Delete a sandbox, taking its deployment out of the game folder first.
///
/// `keep_files` leaves the staging folder alone, which is what somebody
/// rebuilding a profile wants — the mods are already downloaded.
#[tauri::command]
pub fn sandbox_delete(
    state: State<'_, AppState>,
    id: i64,
    keep_files: Option<bool>,
) -> AppResult<PurgeReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let staging = state.paths.staging_dir();
    let backups = state.paths.backup_dir();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &staging, &backups);

    /*
     * Undeployed BEFORE the row is deleted. The ledger is the only record of
     * which files in the game folder are ours, and it goes with the row — so a
     * delete that skipped this would strand every file the sandbox placed, with
     * nothing left that knows how to remove them.
     */
    let report = purge_sandbox(&state.library, &sandbox, &ctx).unwrap_or_default();

    if !keep_files.unwrap_or(false) {
        let _ = std::fs::remove_dir_all(tmc_core::deploy::stage_root(&staging, id));
    }

    state.library.sandbox_delete(id)?;

    tmc_core::audit!(
        state.audit,
        Info,
        App,
        "sandbox.delete",
        format!("{} ({} files removed)", sandbox.name, report.removed)
    );

    Ok(report)
}

#[tauri::command]
pub fn sandbox_set_default(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    state.library.sandbox_set_default(id)
}

// ------------------------------------------------------------------ Contents

/// Add a subscribed item to a sandbox.
///
/// The name comes from the LIBRARY rather than from the caller: the webview
/// naming an item would let a rendered mod description put a row in somebody's
/// sandbox list under any label it liked.
#[tauri::command]
pub async fn sandbox_add_mod(
    state: State<'_, AppState>,
    id: i64,
    kind: String,
    item_id: i64,
) -> AppResult<SandboxRow> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let entry = state.library.find_item(&kind, item_id)?.ok_or_else(|| {
        AppError::invalid(
            "Subscribe to this item first — the app only installs things your account asked \
                 it to keep.",
        )
    })?;

    if entry.app_id.is_some_and(|app| app != sandbox.app_id) {
        return Err(AppError::invalid(
            "That item is for a different game than this sandbox.",
        ));
    }

    state
        .library
        .sandbox_add_mod(id, &kind, item_id, &entry.name)?;

    /*
     * The item's dependency edges are fetched HERE, once, and cached. The
     * alternative is asking the API per item at deploy time, which is forty
     * requests on a screen somebody is waiting on for data that changes about
     * as often as a mod is re-released.
     *
     * A failure is not fatal: the item is already in the sandbox, and the
     * report names anything it could not check rather than pretending it found
     * nothing.
     */
    if let Err(err) = cache_dependencies(&state, &kind, item_id).await {
        let detail = err.detail();

        tracing::warn!("could not fetch dependencies for {kind}:{item_id}: {detail}");
    }

    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::internal("the sandbox vanished"))?;

    row(sandbox, &state)
}

/// What this sandbox's items say about each other.
///
/// A local query — every edge was cached when its item was added — so a screen
/// can call it on every render without a request going anywhere.
#[tauri::command]
pub fn sandbox_check(state: State<'_, AppState>, id: i64) -> AppResult<DependencyReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    state.library.dependency_check(&sandbox)
}

/// Re-fetch the edges for everything in a sandbox.
///
/// What the "check again" button calls. Bounded by the sandbox's own size, and
/// one failure does not stop the rest — an item whose page 404s should not cost
/// the other thirty their answer.
#[tauri::command]
pub async fn sandbox_refresh_dependencies(
    state: State<'_, AppState>,
    id: i64,
) -> AppResult<DependencyReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    for member in &sandbox.mods {
        if let Err(err) = cache_dependencies(&state, &member.kind, member.item_id).await {
            tracing::warn!(
                "could not refresh dependencies for {}: {}",
                member.mod_key,
                err.detail()
            );
        }
    }

    state.library.dependency_check(&sandbox)
}

/// Add everything a sandbox is missing, where the account is subscribed to it.
///
/// **Only subscribed items.** A missing dependency the user has not subscribed
/// to cannot be materialised — the installer refuses an item the account did
/// not ask to keep — so adding the row would produce a sandbox entry that can
/// never download. The UI links those out to their pages instead.
#[tauri::command]
pub async fn sandbox_add_missing(state: State<'_, AppState>, id: i64) -> AppResult<Vec<String>> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let report = state.library.dependency_check(&sandbox)?;

    let mut added = Vec::new();

    for edge in report.missing {
        let Some(entry) = state.library.find_item(&edge.rel_kind, edge.rel_id)? else {
            continue;
        };

        if state
            .library
            .sandbox_add_mod(id, &edge.rel_kind, edge.rel_id, &entry.name)
            .is_ok()
        {
            let _ = cache_dependencies(&state, &edge.rel_kind, edge.rel_id).await;

            added.push(entry.name);
        }
    }

    Ok(added)
}

/// Fetch one item's dependency edges from the API and cache them.
///
/// The parsing is deliberately forgiving: a field the server adds later must
/// not make the whole item uncheckable, and an edge whose relation this build
/// does not recognise is dropped rather than guessed at — guessing would mean
/// inventing a requirement or a conflict out of a string.
async fn cache_dependencies(state: &AppState, kind: &str, item_id: i64) -> AppResult<()> {
    let path = format!("/content/{kind}/{item_id}");

    let value = state
        .api
        .request(tmc_core::api::Method::GET, &path, None, false)
        .await?;

    let edges: Vec<Edge> = value
        .get("dependencies")
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|raw| {
                    Some(Edge {
                        kind: kind.to_string(),
                        item_id,
                        rel_kind: raw.get("kind")?.as_str()?.to_string(),
                        rel_id: raw.get("id")?.as_i64()?,
                        relation: Relation::parse(raw.get("relation")?.as_str()?)?,
                        name: raw
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("Unknown")
                            .to_string(),
                        icon: raw
                            .get("icon")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        note: raw
                            .get("note")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    state.library.dependency_set(kind, item_id, &edges)
}

#[tauri::command]
pub fn sandbox_remove_mod(
    state: State<'_, AppState>,
    id: i64,
    mod_key: String,
) -> AppResult<SandboxRow> {
    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let staging = state.paths.staging_dir();
    let backups = state.paths.backup_dir();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &staging, &backups);

    tmc_core::library::deploy::unstage_mod(&state.library, id, &mod_key, &ctx);

    state.library.sandbox_remove_mod(id, &mod_key)?;

    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    row(sandbox, &state)
}

#[tauri::command]
pub fn sandbox_set_mod_enabled(
    state: State<'_, AppState>,
    id: i64,
    mod_key: String,
    enabled: bool,
) -> AppResult<()> {
    state.library.sandbox_set_mod_enabled(id, &mod_key, enabled)
}

/// Rewrite the load order, first to last.
#[tauri::command]
pub fn sandbox_reorder(
    state: State<'_, AppState>,
    id: i64,
    keys: Vec<String>,
) -> AppResult<Vec<tmc_core::library::sandbox::SandboxMod>> {
    if keys.len() > tmc_core::library::sandbox::MAX_MODS_PER_SANDBOX {
        return Err(AppError::invalid(
            "That is more items than a sandbox holds.",
        ));
    }

    state.library.sandbox_reorder(id, &keys)?;
    state.library.sandbox_mods(id)
}

// -------------------------------------------------------------------- Staging

/// Download and unpack everything in a sandbox that is not staged yet.
///
/// Returns one outcome per item rather than failing on the first: a mod whose
/// game has no rule must not stop the other forty from staging.
#[tauri::command]
pub async fn sandbox_stage(
    state: State<'_, AppState>,
    id: i64,
    force: Option<bool>,
) -> AppResult<Vec<StageOutcome>> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let staging = state.paths.staging_dir();
    let backups = state.paths.backup_dir();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &staging, &backups);

    let force = force.unwrap_or(false);

    let mut out = Vec::new();

    for member in &sandbox.mods {
        if !member.enabled {
            continue;
        }

        // Already staged at the release the server currently offers.
        let entry = state.library.find_item(&member.kind, member.item_id)?;

        let Some(entry) = entry else {
            continue;
        };

        let current = member.staged_at.is_some() && member.release_id == entry.latest_release_id;

        if current && !force {
            continue;
        }

        out.push(stage_mod(&state.library, &sandbox, member, &entry, &ctx).await);
    }

    Ok(out)
}

// ----------------------------------------------------------------- Deployment

/// Put the sandbox in front of the game.
#[tauri::command]
pub fn sandbox_deploy(
    state: State<'_, AppState>,
    id: i64,
    dry_run: Option<bool>,
) -> AppResult<tmc_core::deploy::DeployReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let staging = state.paths.staging_dir();
    let backups = state.paths.backup_dir();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &staging, &backups);

    let mut report = deploy_sandbox(&state.library, &sandbox, &ctx, dry_run.unwrap_or(false))?;

    /*
     * Dependency problems ride along as WARNINGS on the deploy report, and the
     * deploy still happens. The metadata is author-written and frequently wrong
     * — a required edge left in place after a mod absorbed its own dependency
     * is the normal state of every mod site — and a refusal with no escape
     * hatch is one people learn to ignore, which costs the accurate warnings
     * their credibility too. See `library::dependency`.
     */
    if let Ok(deps) = state.library.dependency_check(&sandbox) {
        for edge in &deps.missing {
            report.warnings.push(format!(
                "{} is required by something in this sandbox and is not in it.",
                edge.name
            ));
        }

        for clash in &deps.conflicts {
            report.warnings.push(format!(
                "{} and {} are marked as incompatible.",
                clash.a_name, clash.b_name
            ));
        }
    }

    Ok(report)
}

/// Take it back out.
#[tauri::command]
pub fn sandbox_purge(state: State<'_, AppState>, id: i64) -> AppResult<PurgeReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let staging = state.paths.staging_dir();
    let backups = state.paths.backup_dir();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &staging, &backups);

    purge_sandbox(&state.library, &sandbox, &ctx)
}

/// Check what the last deploy left behind against what is on disk.
#[tauri::command]
pub fn sandbox_verify(state: State<'_, AppState>, id: i64) -> AppResult<VerifyReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let staging = state.paths.staging_dir();
    let backups = state.paths.backup_dir();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &staging, &backups);

    verify_sandbox(&state.library, &sandbox, &ctx)
}

/// Which deployment strategies would actually work for this sandbox, here.
#[tauri::command]
pub fn sandbox_strategies(state: State<'_, AppState>, id: i64) -> AppResult<Vec<StrategyReport>> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let staging = state.paths.staging_dir();
    let backups = state.paths.backup_dir();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &staging, &backups);

    strategies_for(&sandbox, &ctx)
}

/// A game's presets, option schema and deployment rules.
///
/// `None` for a game that ships no `sandbox.json`, which is not an error — it
/// gets the app's defaults, and that is the right answer for the many games
/// where "put the file in `mods/` and link it" is the whole story.
#[tauri::command]
pub fn sandbox_spec(state: State<'_, AppState>, slug: String) -> Option<SandboxSpec> {
    state.app_plugins().sandbox_spec(&slug).cloned()
}

/// Directories a game folder must not be, or contain.
///
/// The app's own data, logs, cache, plugins, staging and backups. A jail
/// anchored above any of them would enclose the plugin registry — and a plugin
/// that can rewrite the registry can grant itself permissions.
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
