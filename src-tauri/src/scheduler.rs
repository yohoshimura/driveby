use crate::backup::{self, BackupState, Settings, Task};
use crate::persist;
use chrono::{DateTime, Months, Utc};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::async_runtime;
use tauri::{AppHandle, Manager};
use tokio::time;
use tracing::info;

/// A schedule's spacing. Monthly is its own variant because a calendar
/// month is not a fixed number of days — "every 30 days" drifts through
/// the calendar a day or two per cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interval {
    Fixed(chrono::Duration),
    Monthly,
}

fn interval_for(schedule: Option<&str>) -> Option<Interval> {
    match schedule {
        Some("hourly") => Some(Interval::Fixed(chrono::Duration::hours(1))),
        Some("daily") => Some(Interval::Fixed(chrono::Duration::days(1))),
        Some("weekly") => Some(Interval::Fixed(chrono::Duration::weeks(1))),
        Some("monthly") => Some(Interval::Monthly),
        _ => None,
    }
}

/// When a run anchored at `anchor` next comes due. Monthly clamps to the
/// end of shorter months (Jan 31 → Feb 28/29) via `checked_add_months`.
fn next_due(anchor: DateTime<Utc>, interval: Interval) -> DateTime<Utc> {
    match interval {
        Interval::Fixed(d) => anchor + d,
        Interval::Monthly => anchor
            .checked_add_months(Months::new(1))
            // Unreachable this side of year 262143; fall back to 31 days.
            .unwrap_or(anchor + chrono::Duration::days(31)),
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
    clock: TaskClock,
    interval: Interval,
) -> bool {
    let on_schedule = now >= next_due(last.unwrap_or(clock.first_seen), interval);
    // A run that fails never advances `last` — `update_last_backup` only
    // writes on success, so that partial failures don't reset the schedule
    // clock. Without a second anchor the schedule alone therefore held the
    // task due on every 60-second tick once it had crossed its interval, so
    // a task whose drive was unplugged retried once a minute forever and
    // filled the history with one failure row per minute. Attempts are
    // spaced by the same interval as successful runs.
    let retry_ready = clock
        .last_attempt
        .is_none_or(|a| now >= next_due(a, interval));
    on_schedule && retry_ready
}

/// What the scheduler remembers about a task between ticks. `tasks.json`
/// only records *successful* runs, so neither field can be recovered from
/// it — the map is persisted to `scheduler.json` so an app restart neither
/// re-anchors a never-run task's clock nor forgets that a failing task
/// just attempted (which used to allow an immediate retry on relaunch).
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskClock {
    /// When the scheduler first observed the task.
    first_seen: DateTime<Utc>,
    /// When we last started a run for it, successful or not.
    last_attempt: Option<DateTime<Utc>>,
}

pub fn spawn(app: AppHandle) {
    async_runtime::spawn(async move {
        time::sleep(Duration::from_secs(10)).await;
        // Tasks observed for the first time start their schedule clock from
        // that observation instead of 1970, so a fresh install with five
        // daily tasks doesn't fire all five 10s after launch (#13). The
        // clocks are reloaded from scheduler.json so "first observation"
        // means first ever, not first since the last app restart.
        let initial: HashMap<String, TaskClock> = match data_path(&app, "scheduler.json") {
            Some(p) => {
                let v = persist::read_json_or(&p, serde_json::json!({})).await;
                serde_json::from_value(v).unwrap_or_default()
            }
            None => HashMap::new(),
        };
        let seen: Mutex<HashMap<String, TaskClock>> = Mutex::new(initial);
        loop {
            if let Err(e) = tick(&app, &seen).await {
                tracing::warn!("scheduler tick failed: {}", e);
            }
            time::sleep(Duration::from_secs(60)).await;
        }
    });
}

async fn tick(app: &AppHandle, seen: &Mutex<HashMap<String, TaskClock>>) -> anyhow::Result<()> {
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
    // Batch clock mutations into at most one scheduler.json write per tick.
    let mut dirty = false;
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

        // First time we observe this scheduled task: record *when* we saw
        // it and don't fire. The user's launch shouldn't double as a backup
        // trigger; the next interval boundary is when it should start
        // running on its own.
        //
        // Recovering from poisoning rather than `.unwrap()`-panicking keeps
        // the scheduler alive across an earlier-tick panic; the worst case is
        // one stale "first observation" record, which is harmless.
        let (clock, first_observation) = {
            let mut s = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match s.entry(task.id.clone()) {
                Entry::Occupied(e) => (*e.get(), false),
                Entry::Vacant(v) => {
                    let fresh = TaskClock {
                        first_seen: now,
                        last_attempt: None,
                    };
                    v.insert(fresh);
                    (fresh, true)
                }
            }
        };
        dirty |= first_observation;
        if first_observation && last.is_none() {
            continue;
        }

        if !is_due(now, last, clock, interval) {
            continue;
        }

        // Record the attempt before spawning. A run that fails leaves
        // `lastBackup` untouched, so this is the only thing standing between
        // a persistently failing task and a retry on every tick.
        {
            let mut s = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(c) = s.get_mut(&task.id) {
                c.last_attempt = Some(now);
                dirty = true;
            }
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
    // rather than growing for the lifetime of the file.
    let snapshot = {
        let mut s = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = s.len();
        s.retain(|id, _| live_ids.contains(id));
        dirty |= s.len() != before;
        if dirty {
            Some(s.clone())
        } else {
            None
        }
    };
    if let (Some(snapshot), Some(path)) = (snapshot, data_path(app, "scheduler.json")) {
        persist::write_json_atomic(&path, &snapshot).await?;
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

    fn clock_seen_at(secs: i64) -> TaskClock {
        TaskClock {
            first_seen: at(secs),
            last_attempt: None,
        }
    }

    fn day() -> Interval {
        Interval::Fixed(chrono::Duration::days(1))
    }

    /// A task whose backup keeps failing never updates `lastBackup` — that
    /// is only written on success — so the schedule clock was its only
    /// anchor and it came due again on the very next 60-second tick, and on
    /// every tick after that. Each one emits `backup-complete`, which the UI
    /// turns into another failure row in the history.
    #[test]
    fn a_failing_task_waits_a_full_interval_before_retrying() {
        let clock = TaskClock {
            first_seen: at(0),
            last_attempt: Some(at(24 * HOUR)),
        };
        // The run at 24h failed, so `last` still says "never ran".
        assert!(!is_due(at(24 * HOUR + 60), None, clock, day()));
        assert!(!is_due(at(47 * HOUR), None, clock, day()));
        assert!(is_due(at(48 * HOUR), None, clock, day()));
    }

    /// The retry gate must not swallow the catch-up run: a task last backed
    /// up days ago is due the moment we notice it, before any attempt.
    #[test]
    fn a_stale_task_is_due_immediately_on_first_sighting() {
        let clock = clock_seen_at(72 * HOUR);
        assert!(is_due(at(72 * HOUR), Some(at(0)), clock, day()));
    }

    /// The regression: a scheduled task that has never been backed up by
    /// hand must still fire on its own, one interval after this process
    /// first noticed it. Before 1.5 the reference point was recomputed as
    /// "now" on every tick, so this case never became due.
    #[test]
    fn never_backed_up_task_becomes_due_one_interval_after_first_sighting() {
        assert!(!is_due(at(12 * HOUR), None, clock_seen_at(0), day()));
        assert!(is_due(at(24 * HOUR), None, clock_seen_at(0), day()));
        assert!(is_due(at(48 * HOUR), None, clock_seen_at(0), day()));
    }

    #[test]
    fn last_backup_takes_precedence_over_first_sighting() {
        let last = Some(at(20 * HOUR));
        // 24h after first sighting but only 4h after the last run.
        assert!(!is_due(at(24 * HOUR), last, clock_seen_at(0), day()));
        assert!(is_due(at(44 * HOUR), last, clock_seen_at(0), day()));
    }

    #[test]
    fn interval_for_recognises_supported_schedules() {
        assert_eq!(
            interval_for(Some("hourly")),
            Some(Interval::Fixed(chrono::Duration::hours(1)))
        );
        assert_eq!(
            interval_for(Some("daily")),
            Some(Interval::Fixed(chrono::Duration::days(1)))
        );
        assert_eq!(
            interval_for(Some("weekly")),
            Some(Interval::Fixed(chrono::Duration::weeks(1)))
        );
        assert_eq!(interval_for(Some("monthly")), Some(Interval::Monthly));
        assert_eq!(interval_for(None), None);
        assert_eq!(interval_for(Some("fortnightly")), None);
    }

    #[test]
    fn hourly_task_becomes_due_after_an_hour() {
        let hourly = Interval::Fixed(chrono::Duration::hours(1));
        assert!(!is_due(at(HOUR - 60), None, clock_seen_at(0), hourly));
        assert!(is_due(at(HOUR), None, clock_seen_at(0), hourly));
    }

    /// "Monthly" means a calendar month, not 30 fixed days — anchored at
    /// Jan 31 the next due date clamps to the end of February instead of
    /// drifting into early March a few days at a time.
    #[test]
    fn monthly_means_calendar_month_with_end_clamping() {
        let jan31 = DateTime::parse_from_rfc3339("2026-01-31T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let feb28 = DateTime::parse_from_rfc3339("2026-02-28T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(next_due(jan31, Interval::Monthly), feb28);

        let mar15 = DateTime::parse_from_rfc3339("2026-03-15T08:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let apr15 = DateTime::parse_from_rfc3339("2026-04-15T08:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(next_due(mar15, Interval::Monthly), apr15);
    }

    /// The clock is persisted to scheduler.json between app runs, so the
    /// JSON shape is a contract: camelCase keys, RFC3339 timestamps.
    #[test]
    fn task_clock_round_trips_through_json() {
        let clock = TaskClock {
            first_seen: at(1_000_000),
            last_attempt: Some(at(2_000_000)),
        };
        let json = serde_json::to_string(&clock).unwrap();
        assert!(json.contains("firstSeen"), "camelCase contract: {json}");
        assert!(json.contains("lastAttempt"), "camelCase contract: {json}");
        let back: TaskClock = serde_json::from_str(&json).unwrap();
        assert_eq!(back.first_seen, clock.first_seen);
        assert_eq!(back.last_attempt, clock.last_attempt);
    }
}
