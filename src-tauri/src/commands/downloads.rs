//! The download queue's IPC surface.
//!
//! **No command here takes a destination.** A download's target comes from the
//! thing that queued it — a sandbox's staging folder, a plugin step's jailed
//! path — and everything the webview can do is act on a queue row it can
//! already see: pause it, resume it, cancel it, reprioritise it, throttle it.
//!
//! That is the whole reason there is no `download_start(url, path)`. Such a
//! command would be an arbitrary-write primitive with a progress bar attached,
//! reachable from a rendered mod description.
//!
//! Progress reaches the UI as an EVENT rather than a poll. A queue of four
//! hundred files ticking twice a second is eight hundred renders a second if
//! the frontend asks; as an event stream it is one message per changed row, and
//! the webview coalesces them itself.

use serde::Serialize;
use tauri::{Emitter, Manager, State};

use tmc_core::download::{DownloadEvent, DownloadState, Status};
use tmc_core::error::AppResult;

use crate::state::AppState;

/// The event name the frontend listens on.
pub const EVENT: &str = "tmc://download";

/// The queue, plus the two totals every screen showing it wants.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub downloads: Vec<DownloadState>,
    pub active: usize,
    /// Combined bytes per second across everything running.
    pub speed_bps: u64,
    /// The global ceiling, or `None` for unlimited.
    pub limit_bps: Option<u64>,
}

#[tauri::command]
pub async fn download_list(state: State<'_, AppState>) -> AppResult<QueueSnapshot> {
    let downloads = state.downloads.list().await;

    Ok(QueueSnapshot {
        active: downloads.iter().filter(|d| d.status.is_active()).count(),
        speed_bps: downloads
            .iter()
            .filter(|d| d.status == Status::Running)
            .map(|d| d.speed_bps)
            .sum(),
        limit_bps: state.downloads.global_limit().await,
        downloads,
    })
}

#[tauri::command]
pub async fn download_pause(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.downloads.pause(&id).await
}

#[tauri::command]
pub async fn download_resume(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.downloads.resume(&id).await
}

#[tauri::command]
pub async fn download_cancel(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.downloads.cancel(&id).await?;

    // The persisted row goes too — a cancelled download is not something to
    // restore on the next launch.
    let _ = state.library.download_delete(&id);

    Ok(())
}

/// Move one up or down the queue. Higher runs first.
#[tauri::command]
pub async fn download_set_priority(
    state: State<'_, AppState>,
    id: String,
    priority: i32,
) -> AppResult<()> {
    state
        .downloads
        .set_priority(&id, priority.clamp(-100, 100))
        .await
}

/// This download's own ceiling, in bytes per second. `None` or `0` removes it.
#[tauri::command]
pub async fn download_set_limit(
    state: State<'_, AppState>,
    id: String,
    bps: Option<u64>,
) -> AppResult<()> {
    state.downloads.set_limit(&id, bps).await
}

/// The ceiling across everything, in bytes per second.
///
/// Persisted in app settings, so it survives a restart — a bandwidth limit
/// somebody set because their connection is shared is not a per-session
/// preference.
#[tauri::command]
pub async fn download_set_global_limit(
    state: State<'_, AppState>,
    bps: Option<u64>,
) -> AppResult<()> {
    state.downloads.set_global_limit(bps).await;

    state.settings.patch(serde_json::json!({
        "downloadLimitBps": bps.unwrap_or(0),
    }))?;

    Ok(())
}

/// How many transfers may run at once.
#[tauri::command]
pub async fn download_set_concurrency(state: State<'_, AppState>, n: usize) -> AppResult<()> {
    state.downloads.set_concurrency(n).await;

    state.settings.patch(serde_json::json!({
        "downloadConcurrency": n.clamp(1, tmc_core::download::MAX_CONCURRENT),
    }))?;

    Ok(())
}

#[tauri::command]
pub async fn download_clear_finished(state: State<'_, AppState>) -> AppResult<usize> {
    let cleared = state.downloads.clear_finished().await;

    let _ = state.library.download_clear_finished();

    Ok(cleared)
}

/// Forward queue events to the webview, and persist every change.
///
/// Spawned once at startup. Two jobs in one loop because both are driven by the
/// same event and doing them separately would mean two subscribers, two
/// wakeups, and a window where the UI shows a state the database does not have.
pub fn spawn_bridge(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let (mut events, library) = {
            let state = app.state::<AppState>();

            (
                state.downloads.subscribe(),
                std::sync::Arc::clone(&state.library),
            )
        };

        loop {
            match events.recv().await {
                Ok(DownloadEvent::Progress { download }) => {
                    /*
                     * Persisted only on a state CHANGE, not on every progress
                     * tick. A tick is twice a second per download; writing all
                     * of them would be hundreds of transactions a second for a
                     * number that is reconstructible from the `.part` file.
                     */
                    if !matches!(download.status, Status::Running) {
                        let _ = library.download_save(&download);
                    }

                    let _ = app.emit(EVENT, &*download);
                }
                Ok(DownloadEvent::Idle) => {
                    let _ = app.emit(EVENT, serde_json::json!({ "type": "idle" }));
                }
                // Only when the manager is gone, which is process shutdown.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                // A burst outran this subscriber. The next event carries the
                // current state, so there is nothing to recover.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });
}

/// Restore the queue from the last session and apply the saved limits.
///
/// Called once at startup. Paused rows come back paused; everything else that
/// was unfinished goes back in the queue and starts on its own.
pub async fn restore(state: &AppState) {
    let settings = state.settings.get();

    state
        .downloads
        .set_global_limit(Some(settings.download_limit_bps).filter(|b| *b > 0))
        .await;

    state
        .downloads
        .set_concurrency(settings.download_concurrency as usize)
        .await;

    let Ok(stored) = state.library.download_list() else {
        return;
    };

    for row in stored {
        if row.status.is_finished() {
            continue;
        }

        let paused = row.status == Status::Paused;

        if state.downloads.enqueue(row.request.clone()).await.is_err() {
            continue;
        }

        if paused {
            let _ = state.downloads.pause(&row.request.id).await;
        }
    }
}
