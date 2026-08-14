use crate::backup::{self, BackupState, Settings, Task};
use crate::persist;
use chrono::{DateTime, Utc};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::async_runtime;
use tauri::{AppHandle, Manager};
use tokio::time;
use tracing::info;

fn interval_for(schedule: Option<&str>) -> Option<chrono::Duration> {
    match schedule {
        Some("daily") => Some(chrono::Duration::days(1)),
        Some("weekly") => Some(chrono::Duration::weeks(1)),
        Some("monthly") => Some(chrono::Duration::days(30)),
        _ => None,
    }
}

fn data_path(app: &AppHandle, name: &str) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(name))
}

/// Is `task` due, given when it last ran and when we first saw it?
///
/// A task that has never run has no `last`, so its schedule clock starts at
/// `first_seen` — the moment this process first observed it. That timestamp
/// has to be *remembered*: the pre-1.5 scheduler kept only the set of task
/// ids it had seen and fell back to `last.unwrap_or(now)`, which reset the
/// reference point to "now" on every single tick. `now - now` is never >=
/// interval, so a scheduled task that had never been backed up by hand
/// never fired on its own — ever.
fn is_due(
    now: DateTime<Utc>,
    last: Option<DateTime<Utc>>,
    first_seen: DateTime<Utc>,
    interval: chrono::Duration,
) -> bool {
    now - last.unwrap_or(first_seen) >= interval
}

pub fn spawn(app: AppHandle) {
    async_runtime::spawn(async move {
        time::sleep(Duration::from_secs(10)).await;
        // Tasks observed for the first time on this app run start their
        // schedule clock from that observation instead of 1970, so a fresh
        // install with five daily tasks doesn't fire all five 10s after
        // launch (#13).
        let seen: Mutex<HashMap<String, DateTime<Utc>>> = Mutex::new(HashMap::new());
        loop {
            if let Err(e) = tick(&app, &seen).await {
                tracing::warn!("scheduler tick failed: {}", e);
            }
            time::sleep(Duration::from_secs(60)).await;
        }
    });
}

async fn tick(app: &AppHandle, seen: &Mutex<HashMap<String, DateTime<Utc>>>) -> anyhow::Result<()> {
    let Some(tasks_path) = data_path(app, "tasks.json") else {
        return Ok(());
    };
    let Some(settings_path) = data_path(app, "settings.json") else {
        return Ok(());
    };

    let tasks_json: serde_json::Value =
        persist::read_json_or(&tasks_path, serde_json::Value::Array(vec![])).await;
    let settings_json: serde_json::Value =
        persist::read_json_or(&settings_path, serde_json::json!({})).await;
    let settings: Settings = serde_json::from_value(settings_json).unwrap_or_default();
    let tasks: Vec<Task> = serde_json::from_value(tasks_json).unwrap_or_default();

    let state = match app.try_state::<BackupState>() {
        Some(s) => s,
        None => return Ok(()),
    };

    let now = Utc::now();
    let live_ids: HashSet<String> = tasks.iter().map(|t| t.id.clone()).collect();
    for task in tasks {
        let Some(interval) = interval_for(task.schedule.as_deref()) else {
            continue;
        };
        if state.is_active(&task.id) {
            continue;
        }

        let last = task
            .last_backup
            .as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc));

        // First time we observe this scheduled task in this process: record
        // *when* we saw it and don't fire. The user's launch shouldn't double
        // as a backup trigger; the next interval boundary is when it should
        // start running on its own.
        //
        // Recovering from poisoning rather than `.unwrap()`-panicking keeps
        // the scheduler alive across an earlier-tick panic; the worst case is
        // one stale "first observation" record, which is harmless.
        let (first_seen, first_observation) = {
            let mut s = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match s.entry(task.id.clone()) {
                Entry::Occupied(e) => (*e.get(), false),
                Entry::Vacant(v) => {
                    v.insert(now);
                    (now, true)
                }
            }
        };
        if first_observation && last.is_none() {
            continue;
        }

        if !is_due(now, last, first_seen, interval) {
            continue;
        }

        info!("scheduler: triggering backup for {}", task.name);
        let app_cloned = app.clone();
        let settings_cloned = settings.clone();
        async_runtime::spawn(async move {
            let state = match app_cloned.try_state::<BackupState>() {
                Some(s) => s,
                None => return,
            };
            // run_backup now owns lastBackup persistence and emits task-updated.
            let _ = backup::run_backup(&app_cloned, &state, task, settings_cloned).await;
        });
    }

    // Forget tasks the user has deleted, so the map tracks the task list
    // rather than growing for the lifetime of the process.
    {
        let mut s = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        s.retain(|id, _| live_ids.contains(id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    const HOUR: i64 = 3600;

    /// The regression: a scheduled task that has never been backed up by
    /// hand must still fire on its own, one interval after this process
    /// first noticed it. Before 1.5 the reference point was recomputed as
    /// "now" on every tick, so this case never became due.
    #[test]
    fn never_backed_up_task_becomes_due_one_interval_after_first_sighting() {
        let first_seen = at(0);
        let day = chrono::Duration::days(1);
        assert!(!is_due(at(12 * HOUR), None, first_seen, day));
        assert!(is_due(at(24 * HOUR), None, first_seen, day));
        assert!(is_due(at(48 * HOUR), None, first_seen, day));
    }

    #[test]
    fn last_backup_takes_precedence_over_first_sighting() {
        let first_seen = at(0);
        let last = Some(at(20 * HOUR));
        let day = chrono::Duration::days(1);
        // 24h after first sighting but only 4h after the last run.
        assert!(!is_due(at(24 * HOUR), last, first_seen, day));
        assert!(is_due(at(44 * HOUR), last, first_seen, day));
    }

    #[test]
    fn interval_for_recognises_supported_schedules() {
        assert_eq!(interval_for(Some("daily")), Some(chrono::Duration::days(1)));
        assert_eq!(
            interval_for(Some("weekly")),
            Some(chrono::Duration::weeks(1))
        );
        assert_eq!(
            interval_for(Some("monthly")),
            Some(chrono::Duration::days(30))
        );
        assert_eq!(interval_for(Some("hourly")), None);
        assert_eq!(interval_for(None), None);
    }
}
