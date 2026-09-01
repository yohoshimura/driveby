use crate::backup::{self, BackupState, Settings, Task};
use crate::fsutil::long_path;
use crate::persist;
use chrono::{DateTime, Datelike, Local, Months, NaiveTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tauri::async_runtime;
use tauri::{AppHandle, Emitter, Manager};
use tokio::time;
use tracing::{info, warn};

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

/// How a task decides it is time to run.
///
/// The two kinds answer different questions. An interval answers "how long
/// since the last one", which is what `hourly`/`daily`/`weekly`/`monthly`
/// have always meant here — a daily task backed up at 03:00 stays a 03:00
/// task only until one run is late. A calendar schedule answers "when",
/// which is the only way to say "in the evening, when I am not working".
#[derive(Clone, Debug, PartialEq, Eq)]
enum Schedule {
    Interval(Interval),
    Calendar { days: Vec<Weekday>, time: NaiveTime },
}

/// 0 = Sunday, the numbering `Date#getDay` uses — the task form writes
/// these, so its convention is the one on disk.
fn weekday_of(index: u8) -> Option<Weekday> {
    Some(match index {
        0 => Weekday::Sun,
        1 => Weekday::Mon,
        2 => Weekday::Tue,
        3 => Weekday::Wed,
        4 => Weekday::Thu,
        5 => Weekday::Fri,
        6 => Weekday::Sat,
        _ => return None,
    })
}

/// None means "this task never fires on its own" — `manual`, an unknown
/// keyword, or a custom schedule with no days or an unreadable time. A
/// half-configured custom schedule has to be inert rather than default to
/// something: firing at midnight because the time did not parse would be a
/// backup the user never asked for, at an hour they never chose.
fn schedule_for(task: &Task) -> Option<Schedule> {
    if task.schedule.as_deref() != Some("custom") {
        return interval_for(task.schedule.as_deref()).map(Schedule::Interval);
    }
    let days: Vec<Weekday> = task
        .schedule_days
        .as_ref()?
        .iter()
        .filter_map(|d| weekday_of(*d))
        .collect();
    if days.is_empty() {
        return None;
    }
    let time = NaiveTime::parse_from_str(task.schedule_time.as_deref()?, "%H:%M").ok()?;
    Some(Schedule::Calendar { days, time })
}

/// The most recent moment a calendar schedule came round, at or before
/// `now`, or None if it has not come round in the last week (which for a
/// weekly cycle means it never has).
///
/// Generic over the timezone so the tests can pin one; production passes
/// `Local`, because "22:00" means 22:00 on the clock on the wall, and a
/// backup scheduled for the evening must not drift into the afternoon in
/// summer.
fn last_occurrence<Tz: TimeZone>(
    now: DateTime<Tz>,
    days: &[Weekday],
    time: NaiveTime,
) -> Option<DateTime<Utc>> {
    let zone = now.timezone();
    for back in 0..=7 {
        let date = now.date_naive() - chrono::Duration::days(back);
        if !days.contains(&date.weekday()) {
            continue;
        }
        // `earliest()` decides both awkward cases the way we want. When the
        // clocks go back, 02:30 happens twice and the earlier one is the
        // one that has certainly already passed. When they go forward, the
        // time may not exist at all that day — no occurrence, so the day
        // contributes nothing rather than silently sliding an hour.
        let Some(local) = zone.from_local_datetime(&date.and_time(time)).earliest() else {
            continue;
        };
        if local <= now {
            return Some(local.with_timezone(&Utc));
        }
    }
    None
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

/// Is `task` due, given when it last ran and when we first saw it?
///
/// A task that has never run has no `last`, so its schedule clock starts at
/// `first_seen` — the moment this process first observed it. That timestamp
/// has to be *remembered*: the pre-1.5 scheduler kept only the set of task
/// ids it had seen and fell back to `last.unwrap_or(now)`, which reset the
/// reference point to "now" on every single tick. `now - now` is never >=
/// interval, so a scheduled task that had never been backed up by hand
/// never fired on its own — ever.
///
/// Takes `now` in whatever zone the caller cares about — production passes
/// the local one, because a calendar schedule is written in wall-clock time
/// and a test that depended on the machine's timezone would be a test that
/// passes here and fails on the runner.
fn is_due<Tz: TimeZone>(
    now: DateTime<Tz>,
    last: Option<DateTime<Utc>>,
    clock: TaskClock,
    schedule: &Schedule,
) -> bool {
    match schedule {
        Schedule::Interval(interval) => {
            let now = now.with_timezone(&Utc);
            let on_schedule = now >= next_due(last.unwrap_or(clock.first_seen), *interval);
            // A run that fails never advances `last` — `update_last_backup`
            // only writes on success, so that partial failures don't reset
            // the schedule clock. Without a second anchor the schedule alone
            // therefore held the task due on every 60-second tick once it had
            // crossed its interval, so a task whose drive was unplugged
            // retried once a minute forever and filled the history with one
            // failure row per minute. Attempts are spaced by the same
            // interval as successful runs.
            let retry_ready = clock
                .last_attempt
                .is_none_or(|a| now >= next_due(a, *interval));
            on_schedule && retry_ready
        }
        // A calendar schedule fires once per occurrence: due when the last
        // occurrence is newer than both the last successful run and the last
        // attempt. That "newer than the last run" is also the catch-up rule
        // — the app closed at 22:00 and opened at 23:00 still runs, because
        // 22:00 is an occurrence nothing has answered yet.
        Schedule::Calendar { days, time } => {
            let Some(occurrence) = last_occurrence(now, days, *time) else {
                return false;
            };
            let anchor = last.unwrap_or(clock.first_seen);
            occurrence > anchor && clock.last_attempt.is_none_or(|a| occurrence > a)
        }
    }
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
    /// Whether the user has already been told, for the absence going on
    /// right now, that this task's destinations are not connected. Reset
    /// when they come back, so the next absence is announced again — and
    /// only once, rather than at every occurrence of a daily schedule.
    #[serde(default)]
    missing_notified: bool,
}

/// The destinations of one task that are not there right now.
///
/// Emitted as a set — every task with something missing — so the UI can
/// replace its whole map on each event: a task dropping out of the list is
/// how it learns the drive came back.
#[derive(Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct MissingDestinations {
    task_id: String,
    task_name: String,
    missing: Vec<String>,
}

/// Stat every distinct destination once, all at the same time, with a
/// deadline on each.
///
/// A dead network share can leave `metadata` blocking for the whole SMB
/// timeout — tens of seconds — so each probe is capped at two seconds:
/// generous for a local volume, short enough to matter.
///
/// The probes used to run one after another, per task, which made the cap
/// misleading. Fifteen tasks with two unreachable destinations each cost
/// sixty seconds of pure waiting, and the tick then slept another sixty on
/// top — the scheduler spent half its life blocked and every interval
/// schedule drifted later. Run together, the whole sweep costs one timeout
/// rather than one per destination, and a destination two tasks share is
/// stat'd once.
async fn probe_all(destinations: &HashSet<String>) -> HashMap<String, bool> {
    let checks = destinations.iter().map(|destination| async move {
        let probe = time::timeout(
            Duration::from_secs(2),
            tokio::fs::metadata(long_path(Path::new(destination))),
        )
        .await;
        (
            destination.clone(),
            matches!(probe, Ok(Ok(meta)) if meta.is_dir()),
        )
    });
    futures_util::future::join_all(checks).await.into_iter().collect()
}

/// The destinations of `task` that `present` says are not there.
fn missing_from(task: &Task, present: &HashMap<String, bool>) -> Vec<String> {
    task.destinations()
        .into_iter()
        .filter(|d| !present.get(d).copied().unwrap_or(false))
        .collect()
}

/// Has this task anywhere left to write?
///
/// Both ends of `tick` ask this — one to re-arm the unplugged-drive reminder,
/// the other to decide whether to speak — so it is written once. Asking it two
/// different ways is what made the reminder go quiet: the flag was cleared on
/// *nothing* missing while the reminder fired on *everything* missing, and a
/// task whose drives went, half-returned, and went again fell in the gap.
///
/// A task with no destinations at all trivially has nowhere missing, and never
/// reaches the reminder anyway; it counts as writable so the flag cannot stick.
fn has_somewhere_to_write(destinations: &[String], missing: &[String]) -> bool {
    destinations.is_empty() || missing.len() < destinations.len()
}

pub fn spawn(app: AppHandle) {
    async_runtime::spawn(async move {
        time::sleep(Duration::from_secs(10)).await;
        // Tasks observed for the first time start their schedule clock from
        // that observation instead of 1970, so a fresh install with five
        // daily tasks doesn't fire all five 10s after launch (#13). The
        // clocks are reloaded from scheduler.json so "first observation"
        // means first ever, not first since the last app restart.
        let initial: HashMap<String, TaskClock> = match persist::data_path(&app, "scheduler.json") {
            Ok(p) => {
                let v = persist::read_json_or(&p, serde_json::json!({})).await;
                serde_json::from_value(v).unwrap_or_default()
            }
            Err(_) => HashMap::new(),
        };
        let seen: Mutex<HashMap<String, TaskClock>> = Mutex::new(initial);
        // What the UI was last told about absent destinations. Held here
        // rather than re-derived, so the event only fires when the answer
        // actually changes instead of once a minute for ever.
        let mut reported: Vec<MissingDestinations> = Vec::new();
        // Survives the tick, so a failed scheduler.json write is retried on the
        // next one. Held here rather than inside tick for exactly that reason:
        // as a local it was reset to false after a failure, and the clocks were
        // then only re-persisted if some later tick happened to change
        // something else.
        let mut dirty = false;
        loop {
            if let Err(e) = tick(&app, &seen, &mut reported, &mut dirty).await {
                tracing::warn!("scheduler tick failed: {}", e);
            }
            time::sleep(Duration::from_secs(60)).await;
        }
    });
}

async fn tick(
    app: &AppHandle,
    seen: &Mutex<HashMap<String, TaskClock>>,
    reported: &mut Vec<MissingDestinations>,
    dirty: &mut bool,
) -> anyhow::Result<()> {
    let Ok(tasks_path) = persist::data_path(app, "tasks.json") else {
        return Ok(());
    };
    let Ok(settings_path) = persist::data_path(app, "settings.json") else {
        return Ok(());
    };

    // Whether the task list on disk can be believed. A tasks.json we could
    // not read yields no tasks, and "no tasks" must not be mistaken for "the
    // user deleted every task" — see the clock GC at the end of this tick.
    let loaded: persist::Loaded<serde_json::Value> = persist::load_json(&tasks_path).await;
    let list_is_trustworthy = !matches!(loaded, persist::Loaded::Damaged);
    let tasks_json = match loaded {
        persist::Loaded::Ok(value) => value,
        _ => serde_json::Value::Array(vec![]),
    };

    let settings_json: serde_json::Value =
        persist::read_json_or(&settings_path, serde_json::json!({})).await;
    let settings: Settings = match serde_json::from_value(settings_json) {
        Ok(s) => s,
        Err(e) => {
            // All-or-nothing, so one wrong-typed field costs the user their
            // exclude patterns on every scheduled run while manual runs — whose
            // settings come from the frontend — keep working. Say so rather
            // than diverge in silence.
            warn!("settings.json could not be read ({}); scheduled runs use defaults", e);
            Settings::default()
        }
    };

    // Deserialised per entry, not as a `Vec<Task>`: serde abandons the whole
    // array at the first bad element, so one hand-edited task would stop every
    // other task from being scheduled — silently.
    let entries: Vec<serde_json::Value> = tasks_json.as_array().cloned().unwrap_or_default();
    let mut tasks: Vec<Task> = Vec::with_capacity(entries.len());
    for entry in &entries {
        match serde_json::from_value::<Task>(entry.clone()) {
            Ok(task) => tasks.push(task),
            Err(e) => warn!(
                "a task in tasks.json could not be read ({}); it will not run on a schedule",
                e
            ),
        }
    }

    let state = match app.try_state::<BackupState>() {
        Some(s) => s,
        None => return Ok(()),
    };

    let now = Utc::now();
    // Taken from every entry carrying an id, including entries that failed to
    // deserialise above: the task is still on disk, so its clock has to survive
    // the GC even while it cannot be scheduled.
    let live_ids: HashSet<String> = entries
        .iter()
        .filter_map(|e| e.get("id").and_then(|i| i.as_str()).map(String::from))
        .collect();
    // Rebuilt from scratch each tick and compared with what the UI was last
    // told, so the event fires on change rather than once a minute.
    let mut absent: Vec<MissingDestinations> = Vec::new();

    // One sweep for the whole tick. Manual tasks are included deliberately:
    // their cards show an absent destination too, and that is what the
    // `destinations-status` event below carries.
    let present = probe_all(&tasks.iter().flat_map(|t| t.destinations()).collect()).await;

    for task in tasks {
        let destinations = task.destinations();
        let missing = missing_from(&task, &present);
        // "Nowhere left to write" — the state the reminder is about, and the
        // one its flag has to be cleared out of. Computed once because the two
        // used to be written separately and did not agree: the flag cleared on
        // *nothing* missing while the reminder fired on *everything* missing,
        // leaving a gap at "some missing". A task with two drives that lost
        // both, got one back, then lost both again fell into it and was never
        // mentioned a second time.
        let all_missing = !has_somewhere_to_write(&destinations, &missing);
        if !missing.is_empty() {
            absent.push(MissingDestinations {
                task_id: task.id.clone(),
                task_name: task.name.clone(),
                missing: missing.clone(),
            });
        }
        if !all_missing {
            // There is somewhere to write again: arm the reminder for the
            // next time there is not.
            let mut s = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(clock) = s.get_mut(&task.id) {
                if clock.missing_notified {
                    clock.missing_notified = false;
                    *dirty = true;
                }
            }
        }

        let Some(schedule) = schedule_for(&task) else {
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
                        missing_notified: false,
                    };
                    v.insert(fresh);
                    (fresh, true)
                }
            }
        };
        *dirty |= first_observation;
        if first_observation && last.is_none() {
            continue;
        }

        // In the local zone: "22:00" is 22:00 on the clock on the wall.
        if !is_due(now.with_timezone(&Local), last, clock, &schedule) {
            continue;
        }

        // Record the attempt before spawning. A run that fails leaves
        // `lastBackup` untouched, so this is the only thing standing between
        // a persistently failing task and a retry on every tick.
        {
            let mut s = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(c) = s.get_mut(&task.id) {
                c.last_attempt = Some(now);
                *dirty = true;
            }
        }

        // Nowhere to write: don't start a run that would walk the whole
        // source only to fail, and don't file a red history row for a drive
        // sitting in a drawer. Tell the user instead — once per absence,
        // not once per occurrence.
        //
        // The attempt above was recorded deliberately: it consumes this
        // occurrence, so plugging the drive back in at lunchtime does not
        // set a backup going while the user is still handling the disk. It
        // goes at the next scheduled time, which is what they asked for.
        if all_missing {
            let mut s = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let already_told = s.get(&task.id).is_some_and(|c| c.missing_notified);
            if !already_told {
                if let Some(clock) = s.get_mut(&task.id) {
                    clock.missing_notified = true;
                    *dirty = true;
                }
                drop(s);
                info!(task = %task.name, "destination not connected — backup skipped");
                let _ = app.emit(
                    "destination-missing",
                    MissingDestinations {
                        task_id: task.id.clone(),
                        task_name: task.name.clone(),
                        missing: missing.clone(),
                    },
                );
            }
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

    if absent != *reported {
        let _ = app.emit("destinations-status", &absent);
        *reported = absent;
    }

    // Forget tasks the user has deleted, so the map tracks the task list
    // rather than growing for the lifetime of the file.
    //
    // Only when the list can be believed. A tasks.json that could not be read
    // yields an empty `live_ids`, and this would then read it as "every task
    // was deleted" and drop every clock — re-anchoring `first_seen` for tasks
    // that have never run (delaying each a full interval) and forgetting
    // `last_attempt` for failing ones (allowing an immediate retry). Those are
    // exactly the two regressions the persisted clock exists to prevent, and a
    // transient lock on the file was enough to cause both.
    let snapshot = {
        let mut s = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if list_is_trustworthy {
            let before = s.len();
            s.retain(|id, _| live_ids.contains(id));
            *dirty |= s.len() != before;
        }
        if *dirty {
            Some(s.clone())
        } else {
            None
        }
    };
    if let (Some(snapshot), Ok(path)) = (snapshot, persist::data_path(app, "scheduler.json")) {
        persist::write_json_atomic(&path, &snapshot).await?;
        // Cleared only once the clocks are actually on disk. `?` above leaves
        // it set, so the next tick writes them again rather than waiting for
        // some unrelated change to mark the map dirty a second time.
        *dirty = false;
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
            missing_notified: false,
        }
    }

    fn dests(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// The sequence that used to go silent. The reminder's flag was cleared
    /// on *nothing* missing while the reminder fired on *everything* missing,
    /// so a task that lost both drives, got one back, then lost both again
    /// never spoke the second time — the flag was still set from the first.
    ///
    /// Step 2 is the one that matters: a task with one drive left can still
    /// run, so the reminder has to re-arm there, not only at full recovery.
    #[test]
    fn a_partial_recovery_re_arms_the_unplugged_reminder() {
        let all = dests(&["D1", "D2"]);

        assert!(
            !has_somewhere_to_write(&all, &all),
            "both drives gone: nothing to write, the reminder speaks"
        );
        assert!(
            has_somewhere_to_write(&all, &dests(&["D2"])),
            "one drive back: the task runs partially, so the reminder re-arms"
        );
        assert!(
            !has_somewhere_to_write(&all, &all),
            "both gone again: it must speak a second time"
        );
    }

    /// A single-destination task is the common case and must not regress:
    /// its only drive missing is the same thing as all of them missing.
    #[test]
    fn one_destination_missing_is_nowhere_to_write() {
        let one = dests(&["D1"]);
        assert!(!has_somewhere_to_write(&one, &one));
        assert!(has_somewhere_to_write(&one, &[]));
    }

    /// A task with no destinations never reaches the reminder at all, so it
    /// counts as writable — otherwise its flag could latch on and stay.
    #[test]
    fn a_task_with_no_destinations_never_latches_the_flag() {
        assert!(has_somewhere_to_write(&[], &[]));
    }

    fn day() -> Schedule {
        Schedule::Interval(Interval::Fixed(chrono::Duration::days(1)))
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
            missing_notified: false,
        };
        // The run at 24h failed, so `last` still says "never ran".
        assert!(!is_due(at(24 * HOUR + 60), None, clock, &day()));
        assert!(!is_due(at(47 * HOUR), None, clock, &day()));
        assert!(is_due(at(48 * HOUR), None, clock, &day()));
    }

    /// The retry gate must not swallow the catch-up run: a task last backed
    /// up days ago is due the moment we notice it, before any attempt.
    #[test]
    fn a_stale_task_is_due_immediately_on_first_sighting() {
        let clock = clock_seen_at(72 * HOUR);
        assert!(is_due(at(72 * HOUR), Some(at(0)), clock, &day()));
    }

    /// The regression: a scheduled task that has never been backed up by
    /// hand must still fire on its own, one interval after this process
    /// first noticed it. Before 1.5 the reference point was recomputed as
    /// "now" on every tick, so this case never became due.
    #[test]
    fn never_backed_up_task_becomes_due_one_interval_after_first_sighting() {
        assert!(!is_due(at(12 * HOUR), None, clock_seen_at(0), &day()));
        assert!(is_due(at(24 * HOUR), None, clock_seen_at(0), &day()));
        assert!(is_due(at(48 * HOUR), None, clock_seen_at(0), &day()));
    }

    #[test]
    fn last_backup_takes_precedence_over_first_sighting() {
        let last = Some(at(20 * HOUR));
        // 24h after first sighting but only 4h after the last run.
        assert!(!is_due(at(24 * HOUR), last, clock_seen_at(0), &day()));
        assert!(is_due(at(44 * HOUR), last, clock_seen_at(0), &day()));
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
        let hourly = Schedule::Interval(Interval::Fixed(chrono::Duration::hours(1)));
        assert!(!is_due(at(HOUR - 60), None, clock_seen_at(0), &hourly));
        assert!(is_due(at(HOUR), None, clock_seen_at(0), &hourly));
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

    // ── Calendar schedules ──────────────────────────────────────────
    //
    // Pinned to +02:00 rather than the machine's zone: `Local` on the CI
    // runner is UTC and here it is Paris, and a schedule that means "22:00
    // on the clock" is exactly the thing that would pass in one and fail in
    // the other.

    fn local(iso: &str) -> DateTime<chrono::FixedOffset> {
        DateTime::parse_from_rfc3339(iso).unwrap()
    }

    fn utc(iso: &str) -> DateTime<Utc> {
        local(iso).with_timezone(&Utc)
    }

    fn evening_on(days: &[Weekday]) -> Schedule {
        Schedule::Calendar {
            days: days.to_vec(),
            time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
        }
    }

    fn seen_at(iso: &str) -> TaskClock {
        TaskClock {
            first_seen: utc(iso),
            last_attempt: None,
            missing_notified: false,
        }
    }

    /// The dates these tests hang on, asserted rather than assumed.
    #[test]
    fn the_reference_dates_are_the_weekdays_the_tests_claim() {
        assert_eq!(local("2026-08-27T12:00:00+02:00").weekday(), Weekday::Thu);
        assert_eq!(local("2026-08-28T12:00:00+02:00").weekday(), Weekday::Fri);
        assert_eq!(local("2026-09-03T12:00:00+02:00").weekday(), Weekday::Thu);
    }

    #[test]
    fn a_calendar_schedule_fires_at_its_time_and_not_before() {
        let schedule = evening_on(&[Weekday::Mon, Weekday::Thu]);
        let clock = seen_at("2026-08-27T06:00:00+02:00");
        assert!(!is_due(
            local("2026-08-27T21:59:00+02:00"),
            None,
            clock,
            &schedule
        ));
        assert!(is_due(
            local("2026-08-27T22:00:00+02:00"),
            None,
            clock,
            &schedule
        ));
    }

    /// Once per occurrence, not once per tick: the scheduler wakes every
    /// 60 seconds, and "it is past 22:00" is true for the rest of the day.
    #[test]
    fn a_calendar_schedule_runs_once_per_occurrence() {
        let schedule = evening_on(&[Weekday::Thu]);
        let clock = seen_at("2026-08-27T06:00:00+02:00");
        let ran = Some(utc("2026-08-27T22:00:05+02:00"));
        assert!(!is_due(
            local("2026-08-27T23:30:00+02:00"),
            ran,
            clock,
            &schedule
        ));
        assert!(is_due(
            local("2026-09-03T22:00:00+02:00"),
            ran,
            clock,
            &schedule
        ));
    }

    /// The app was closed at 22:00 and opened at 23:10. The occurrence has
    /// not been answered, so it runs now — a backup tool that skipped the
    /// day because the machine was off at the exact minute would be missing
    /// the point.
    #[test]
    fn a_missed_occurrence_is_caught_up_on_the_same_day() {
        let schedule = evening_on(&[Weekday::Thu]);
        let clock = seen_at("2026-08-01T06:00:00+02:00");
        let ran = Some(utc("2026-08-20T22:00:00+02:00"));
        assert!(is_due(
            local("2026-08-27T23:10:00+02:00"),
            ran,
            clock,
            &schedule
        ));
    }

    /// A run that failed leaves `lastBackup` untouched, so the attempt is
    /// the only thing stopping a retry on the very next tick — the 1.5
    /// behaviour, carried over to calendar schedules.
    #[test]
    fn a_failed_calendar_run_waits_for_the_next_occurrence() {
        let schedule = evening_on(&[Weekday::Thu]);
        let clock = TaskClock {
            first_seen: utc("2026-08-01T00:00:00+02:00"),
            last_attempt: Some(utc("2026-08-27T22:00:05+02:00")),
            missing_notified: false,
        };
        assert!(!is_due(
            local("2026-08-27T22:01:00+02:00"),
            None,
            clock,
            &schedule
        ));
        assert!(!is_due(
            local("2026-08-28T09:00:00+02:00"),
            None,
            clock,
            &schedule
        ));
        assert!(is_due(
            local("2026-09-03T22:00:00+02:00"),
            None,
            clock,
            &schedule
        ));
    }

    /// A task created at 23:00 must not immediately fire for the 22:00 slot
    /// it was not around for.
    #[test]
    fn an_occurrence_from_before_the_task_existed_does_not_fire() {
        let schedule = evening_on(&[Weekday::Thu, Weekday::Fri]);
        let clock = seen_at("2026-08-27T23:00:00+02:00");
        assert!(!is_due(
            local("2026-08-27T23:05:00+02:00"),
            None,
            clock,
            &schedule
        ));
        assert!(is_due(
            local("2026-08-28T22:00:00+02:00"),
            None,
            clock,
            &schedule
        ));
    }

    #[test]
    fn last_occurrence_looks_back_a_week_at_most() {
        let ten_pm = NaiveTime::from_hms_opt(22, 0, 0).unwrap();
        // Thursday 21:00: today's slot has not arrived, so the answer is
        // last Thursday's.
        assert_eq!(
            last_occurrence(
                local("2026-08-27T21:00:00+02:00"),
                &[Weekday::Thu],
                ten_pm
            ),
            Some(utc("2026-08-20T22:00:00+02:00"))
        );
        assert_eq!(
            last_occurrence(local("2026-08-27T21:00:00+02:00"), &[], ten_pm),
            None
        );
    }

    fn custom_task(days: Option<Vec<u8>>, time: Option<&str>) -> Task {
        Task {
            id: "t".into(),
            name: "t".into(),
            source: "C:/src".into(),
            destination: None,
            destinations: Some(vec!["D:/dst".into()]),
            schedule: Some("custom".into()),
            schedule_days: days,
            schedule_time: time.map(|t| t.into()),
            last_backup: None,
        }
    }

    /// A half-configured custom schedule is inert. Defaulting the time to
    /// midnight would be a backup at an hour nobody chose; defaulting the
    /// days to all of them would be one every day.
    #[test]
    fn a_custom_schedule_needs_both_days_and_a_readable_time() {
        assert_eq!(
            schedule_for(&custom_task(Some(vec![1, 4]), Some("22:00"))),
            Some(evening_on(&[Weekday::Mon, Weekday::Thu]))
        );
        assert_eq!(schedule_for(&custom_task(Some(vec![]), Some("22:00"))), None);
        assert_eq!(schedule_for(&custom_task(None, Some("22:00"))), None);
        assert_eq!(schedule_for(&custom_task(Some(vec![1]), None)), None);
        assert_eq!(schedule_for(&custom_task(Some(vec![1]), Some("late"))), None);
        // Out-of-range day numbers are dropped, not rounded into a weekday.
        assert_eq!(schedule_for(&custom_task(Some(vec![9]), Some("22:00"))), None);
    }

    /// The four keyword schedules keep their meaning, and everything else
    /// still means "manual".
    #[test]
    fn keyword_schedules_are_unchanged() {
        let mut task = custom_task(None, None);
        task.schedule = Some("daily".into());
        assert_eq!(schedule_for(&task), Some(day()));
        task.schedule = Some("manual".into());
        assert_eq!(schedule_for(&task), None);
        task.schedule = None;
        assert_eq!(schedule_for(&task), None);
    }

    /// The clock is persisted to scheduler.json between app runs, so the
    /// JSON shape is a contract: camelCase keys, RFC3339 timestamps.
    #[test]
    fn task_clock_round_trips_through_json() {
        let clock = TaskClock {
            first_seen: at(1_000_000),
            last_attempt: Some(at(2_000_000)),
            missing_notified: true,
        };
        let json = serde_json::to_string(&clock).unwrap();
        assert!(json.contains("firstSeen"), "camelCase contract: {json}");
        assert!(json.contains("lastAttempt"), "camelCase contract: {json}");
        let back: TaskClock = serde_json::from_str(&json).unwrap();
        assert_eq!(back.first_seen, clock.first_seen);
        assert_eq!(back.last_attempt, clock.last_attempt);
        assert!(back.missing_notified);
    }

    /// A scheduler.json written before 1.7.2 has no `missingNotified`. It
    /// must still load — losing the clocks would re-anchor every never-run
    /// task and forget every pending retry.
    #[test]
    fn a_pre_1_7_2_clock_still_loads() {
        let json = r#"{"firstSeen":"2026-08-01T10:00:00Z","lastAttempt":null}"#;
        let clock: TaskClock = serde_json::from_str(json).unwrap();
        assert!(!clock.missing_notified);
    }
}
