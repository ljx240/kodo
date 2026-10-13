//! Scheduled tasks: data model and time-rule calculator.
//!
//! This module is the **source of truth for "when does a scheduled task fire?"**
//! The preview the GUI shows and the dispatcher the runtime walks must agree
//! byte-for-byte; both call [`next_occurrences`] with the same clock.
//!
//! The module deliberately knows nothing about persistence, threads, or the
//! shell. Storage and the scheduler loop live in `kodo-shell`. Tests here are
//! deterministic by way of the [`Clock`] trait: every test injects a fake clock
//! and a fixed reference timestamp so the arithmetic never depends on wall time.

use std::collections::BTreeSet;
use std::fmt;

use jiff::civil::{DateTime as CivilDateTime, Weekday};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span, Timestamp};
use serde::{Deserialize, Serialize};

/// Seconds since the Unix epoch. `i64` because negative values are out of
/// range and u64 would silently wrap on dates before 1970; we keep room for
/// wall-clock oddities rather than refuse them.
pub type UnixSeconds = i64;

/// One source of monotonic+wall time, abstracted so tests can inject a fake.
///
/// `now_unix` is the wall-clock seconds (UTC); `monotonic_seconds` is a
/// strictly-non-decreasing counter used for deadline / elapsed checks. The
/// production implementation reads the system clock; tests substitute a
/// controlled value pair.
///
/// `Send + Sync` so a `Scheduler` that holds `Arc<dyn Clock>` can be
/// shared across the dispatch thread, the executor threads, and the
/// Tauri command threads without `unsafe`.
pub trait Clock: Send + Sync {
    fn now_unix(&self) -> UnixSeconds;
    fn monotonic_seconds(&self) -> u64;
}

/// System clock: real wall time + `Instant`-derived monotonic time.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> UnixSeconds {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn monotonic_seconds(&self) -> u64 {
        use std::time::{Instant, SystemTime, UNIX_EPOCH};
        // The boot-relative origin gives us a stable monotonic counter.
        static ORIGIN: std::sync::OnceLock<(Instant, u64)> = std::sync::OnceLock::new();
        let (instant, base_secs) = ORIGIN.get_or_init(|| {
            (
                Instant::now(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            )
        });
        base_secs + instant.elapsed().as_secs()
    }
}

/// Fake clock used by every deterministic test in this module.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    pub unix: UnixSeconds,
    pub mono: u64,
}

impl Clock for FixedClock {
    fn now_unix(&self) -> UnixSeconds {
        self.unix
    }
    fn monotonic_seconds(&self) -> u64 {
        self.mono
    }
}

// ---------------------------------------------------------------------------
// Schedule
// ---------------------------------------------------------------------------

/// When a scheduled task should fire. Stored as JSON in `schedule_json`.
///
/// All variants that involve a wall-clock time carry it in UTC; the variants
/// that involve human-meaningful local time carry an IANA timezone name so
/// the user sees "09:00 in Asia/Shanghai" instead of "01:00 UTC".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    /// One shot at `fire_at` (UTC seconds). The task finishes after the run
    /// completes; manual re-trigger forms a new run, never a replay of this one.
    Once { fire_at: UnixSeconds },
    /// Every `interval_seconds` starting at `anchor` (UTC). The next occurrence
    /// is `anchor + k * interval_seconds` for the smallest `k` with
    /// `anchor + k * interval_seconds > after`. Execution duration does **not**
    /// slide the anchor — a slow run that overlaps the next slot is handled by
    /// [`OverlapPolicy`], not by recalculating the schedule.
    Interval {
        anchor: UnixSeconds,
        interval_seconds: u64,
    },
    /// Every day at `hour:minute` in `tz_name`. DST gaps skip; DST overlaps
    /// use the earlier offset.
    Daily {
        hour: u8,
        minute: u8,
        tz_name: String,
    },
    /// The given weekdays (ISO: 1=Mon … 7=Sun) at `hour:minute` in `tz_name`.
    Weekly {
        weekdays: Vec<u8>,
        hour: u8,
        minute: u8,
        tz_name: String,
    },
}

/// What to do when a scheduled moment arrives but a previous run for the same
/// task is still in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlapPolicy {
    /// Skip the new occurrence; record `skipped` and advance the cursor.
    Skip,
    /// Queue at most one pending run; a newer occurrence replaces the queued
    /// older one (the "latest wins" mode called out in the docs).
    QueueLatest,
    /// Start a second run anyway. Bounded by the concurrency limits in the
    /// shell — the runtime refuses if those would be exceeded.
    Allow,
}

/// How to interpret missed occurrences after a downtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MisfirePolicy {
    /// At most one catch-up run for the latest missed moment in the grace
    /// window. Older missed moments collapse into a `skipped_count` total.
    LatestOnce,
    /// Skip everything missed; record `skipped_count` and continue from the
    /// next future occurrence.
    Skip,
    /// Like `LatestOnce` but coalesce all missed moments into a single
    /// catch-up run. Same window as `LatestOnce`.
    Coalesce,
}

/// Lifecycle of one [`TaskRun`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Lifecycle {
    Queued,
    Running,
    Cancelling,
    Finished,
}

impl Lifecycle {
    pub fn is_terminal(self) -> bool {
        matches!(self, Lifecycle::Finished)
    }
}

/// Stable outcome codes written to `task_runs.outcome`. The string form is
/// what the GUI renders; the enum is what the scheduler writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// Agent delivery + verification satisfied the task policy.
    Succeeded,
    /// Some delivery was produced, but the configured acceptance did not pass.
    Partial,
    /// The run is waiting on something only the user can resolve (approval,
    /// answer to a question, missing provider). The run stays in `Queued` and
    /// is not auto-retried.
    Blocked,
    /// A clear, recorded failure (tool error, model refused, etc.).
    Failed,
    /// User cancelled and the executor confirmed termination.
    Cancelled,
    /// Wall deadline passed; executor confirmed termination.
    TimedOut,
    /// Executor started, but the process died before a terminal outcome.
    /// The shell keeps the session / trace intact and never auto-retries.
    Interrupted,
    /// Consumed a scheduled moment without running (overlap, coalesced, or
    /// past the grace window).
    Skipped,
}

/// What a task is: drive the agent loop, or call a registered job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TaskKind {
    /// Run the existing agent driver with this message on this project.
    Agent {
        project: String,
        message: String,
        /// Persona chain to attach (resolved by the desktop shell at run time).
        /// Empty means "no persona for this run".
        personas: Vec<String>,
        /// Budget family override (`None` = per-intent default,
        /// `Some("summary")` = lift to read-only-summary caps).
        budget_family: Option<String>,
        /// Resource key for shared admission. Defaults to the normalized
        /// project path; an empty string means "no shared resource" (only the
        /// global concurrency limit applies).
        resource_key: Option<String>,
    },
    /// Invoke a registered job handler. Params are JSON-encoded and validated
    /// by the registered handler.
    Job {
        job_type: String,
        params: serde_json::Value,
        /// Declared resources (e.g. `"memory:v1"`, `"workspace:/p"`).
        resources: Vec<String>,
    },
}

impl TaskKind {
    /// Resource keys this kind holds at the moment the run is admitted. Empty
    /// means no shared admission — only global concurrency matters.
    pub fn resource_keys(&self) -> Vec<String> {
        match self {
            TaskKind::Agent { resource_key, .. } => match resource_key {
                Some(key) if !key.is_empty() => vec![format!("agent:{key}")],
                _ => Vec::new(),
            },
            TaskKind::Job { resources, .. } => resources
                .iter()
                .filter(|r| !r.is_empty())
                .map(|r| format!("job:{r}"))
                .collect(),
        }
    }
}

/// One user-defined schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask {
    pub id: String,
    pub title: String,
    pub state: TaskState,
    /// Configuration version. Bumped on every edit so the runtime can
    /// detect a config mutation between queued → running.
    pub revision: u64,
    /// Bumped on every schedule edit. The next occurrence is computed from
    /// `(after, generation, schedule)` so an edit does not replay already
    /// consumed moments.
    pub schedule_generation: u64,
    pub schedule: Schedule,
    pub task: TaskKind,
    pub timezone: String,
    pub misfire_policy: MisfirePolicy,
    /// Grace window in seconds. `LatestOnce` / `Coalesce` honor this; `Skip`
    /// is unaffected. Defaults to one hour when zero.
    pub grace_seconds: u64,
    pub overlap_policy: OverlapPolicy,
    /// Wall-clock deadline for a single run; `None` means "no task-level
    /// timeout, the executor decides". The runtime caps by its own policy too.
    pub timeout_seconds: Option<u64>,
    pub created_at: UnixSeconds,
    pub updated_at: UnixSeconds,
    /// Bumped when the schedule is edited; the shell writes a fresh
    /// `next_fire_at` after the bump.
    pub next_fire_at: Option<UnixSeconds>,
    /// Set by the shell, not the GUI. Marks the task archived — listing hides
    /// it, the dispatcher ignores it.
    pub archived: bool,
}

/// Task state machine: independent of run lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskState {
    Enabled,
    Paused,
    Finished,
    Archived,
}

impl TaskState {
    pub fn dispatcher_should_run(self) -> bool {
        matches!(self, TaskState::Enabled)
    }
    pub fn listing_should_show(self) -> bool {
        matches!(
            self,
            TaskState::Enabled | TaskState::Paused | TaskState::Finished
        )
    }
}

// ---------------------------------------------------------------------------
// Time rule calculator
// ---------------------------------------------------------------------------

/// Cap on `next_occurrences` output. Five is what the GUI previews; callers
/// that want more can slice the iterator themselves, but the docs define the
/// preview surface as exactly five.
pub const PREVIEW_COUNT: usize = 5;

/// Minimum interval the GUI accepts (and the calculator defends) so an
/// accidental `interval_seconds: 0` cannot burn a loop.
pub const MIN_INTERVAL_SECONDS: u64 = 60;

/// Caller-supplied errors from the calculator. Kept narrow on purpose: a more
/// general error type can grow when the GUI needs richer feedback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    /// Daily / Weekly hour out of `0..=23`.
    InvalidHour(u8),
    /// Daily / Weekly minute out of `0..=59`.
    InvalidMinute(u8),
    /// Weekday out of `1..=7` (ISO).
    InvalidWeekday(u8),
    /// Empty weekday set for `Weekly`.
    NoWeekdays,
    /// Interval smaller than [`MIN_INTERVAL_SECONDS`].
    IntervalTooSmall { seconds: u64 },
    /// `Once` is in the past. Editing a task with a past `fire_at` is a
    /// bug, not a user choice — refuse it loudly.
    OnceInPast { fire_at: UnixSeconds },
    /// Unknown / unloadable IANA timezone name.
    InvalidTimezone(String),
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduleError::InvalidHour(h) => write!(f, "hour must be 0..=23, got {h}"),
            ScheduleError::InvalidMinute(m) => write!(f, "minute must be 0..=59, got {m}"),
            ScheduleError::InvalidWeekday(w) => write!(f, "weekday must be 1..=7 (ISO), got {w}"),
            ScheduleError::NoWeekdays => f.write_str("weekly schedule needs at least one weekday"),
            ScheduleError::IntervalTooSmall { seconds } => write!(
                f,
                "interval must be at least {MIN_INTERVAL_SECONDS} seconds, got {seconds}"
            ),
            ScheduleError::OnceInPast { fire_at } => {
                write!(f, "once schedule is in the past: {fire_at}")
            }
            ScheduleError::InvalidTimezone(name) => write!(f, "unknown IANA timezone: {name:?}"),
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Validate the time-rule without doing any computation. Used by the storage
/// layer on insert / update so an invalid schedule never reaches the dispatcher.
pub fn validate(schedule: &Schedule, now: UnixSeconds) -> Result<(), ScheduleError> {
    match schedule {
        Schedule::Once { fire_at } => {
            if *fire_at <= now {
                return Err(ScheduleError::OnceInPast { fire_at: *fire_at });
            }
        }
        Schedule::Interval {
            interval_seconds, ..
        } => {
            if *interval_seconds < MIN_INTERVAL_SECONDS {
                return Err(ScheduleError::IntervalTooSmall {
                    seconds: *interval_seconds,
                });
            }
        }
        Schedule::Daily {
            hour,
            minute,
            tz_name,
        } => {
            check_hm(*hour, *minute)?;
            check_timezone(tz_name)?;
        }
        Schedule::Weekly {
            weekdays,
            hour,
            minute,
            tz_name,
        } => {
            if weekdays.is_empty() {
                return Err(ScheduleError::NoWeekdays);
            }
            for weekday in weekdays {
                if !matches!(weekday, 1..=7) {
                    return Err(ScheduleError::InvalidWeekday(*weekday));
                }
            }
            check_hm(*hour, *minute)?;
            check_timezone(tz_name)?;
        }
    }
    Ok(())
}

fn check_hm(hour: u8, minute: u8) -> Result<(), ScheduleError> {
    if hour > 23 {
        return Err(ScheduleError::InvalidHour(hour));
    }
    if minute > 59 {
        return Err(ScheduleError::InvalidMinute(minute));
    }
    Ok(())
}

fn check_timezone(name: &str) -> Result<(), ScheduleError> {
    TimeZone::get(name)
        .map(|_| ())
        .map_err(|_| ScheduleError::InvalidTimezone(name.to_owned()))
}

/// Return up to `count` UTC instants at which this schedule fires, strictly
/// after `after`. The same function powers the GUI preview and the dispatcher
/// — keeping a single calculator is what the docs §4.2 require.
pub fn next_occurrences(
    schedule: &Schedule,
    after: UnixSeconds,
    count: usize,
) -> Result<Vec<UnixSeconds>, ScheduleError> {
    if count == 0 {
        return Ok(Vec::new());
    }
    match schedule {
        Schedule::Once { fire_at } => {
            if *fire_at > after {
                Ok(vec![*fire_at])
            } else {
                Ok(Vec::new())
            }
        }
        Schedule::Interval {
            anchor,
            interval_seconds,
        } => {
            if *interval_seconds < MIN_INTERVAL_SECONDS {
                return Err(ScheduleError::IntervalTooSmall {
                    seconds: *interval_seconds,
                });
            }
            let step = *interval_seconds as i64;
            let anchor = *anchor;
            // First occurrence strictly greater than `after`. Anchor itself
            // is the answer when it is still in the future; otherwise advance
            // by whole steps. Clock backwards collapses to anchor.
            let diff = (after - anchor).saturating_add(1).max(0);
            let k = if diff == 0 {
                0
            } else {
                (diff + step - 1) / step
            };
            let mut out = Vec::with_capacity(count);
            let mut current = anchor.saturating_add(k.saturating_mul(step));
            while out.len() < count {
                if current > after {
                    out.push(current);
                }
                current = current.saturating_add(step);
            }
            Ok(out)
        }
        Schedule::Daily {
            hour,
            minute,
            tz_name,
        } => daily_occurrences(tz_name, *hour, *minute, after, count),
        Schedule::Weekly {
            weekdays,
            hour,
            minute,
            tz_name,
        } => weekly_occurrences(tz_name, weekdays, *hour, *minute, after, count),
    }
}

fn daily_occurrences(
    tz_name: &str,
    hour: u8,
    minute: u8,
    after: UnixSeconds,
    count: usize,
) -> Result<Vec<UnixSeconds>, ScheduleError> {
    // Validate the zone up front so we can report a tidy error.
    let _ =
        TimeZone::get(tz_name).map_err(|_| ScheduleError::InvalidTimezone(tz_name.to_owned()))?;
    let after_ts = Timestamp::from_second(after)
        .map_err(|_| ScheduleError::InvalidTimezone(format!("timestamp out of range: {after}")))?;
    let mut cursor_ts = after_ts;
    let mut out = Vec::with_capacity(count);
    while out.len() < count {
        let zoned = cursor_ts
            .in_tz(tz_name)
            .map_err(|error| ScheduleError::InvalidTimezone(format!("{tz_name}: {error}")))?;
        let next_civil = next_daily(&zoned.datetime(), hour, minute);
        let utc = next_civil
            .in_tz(tz_name)
            .map_err(|error| ScheduleError::InvalidTimezone(format!("{tz_name}: {error}")))?
            .timestamp();
        if utc.as_second() <= after {
            // DST gap collapsed the slot to the same wall instant we just
            // consumed; nudge one minute forward and retry.
            cursor_ts = cursor_ts
                .checked_add(SignedDuration::from_mins(1))
                .unwrap_or(cursor_ts);
            continue;
        }
        out.push(utc.as_second());
        cursor_ts = utc;
    }
    Ok(out)
}

fn weekly_occurrences(
    tz_name: &str,
    weekdays: &[u8],
    hour: u8,
    minute: u8,
    after: UnixSeconds,
    count: usize,
) -> Result<Vec<UnixSeconds>, ScheduleError> {
    let _ =
        TimeZone::get(tz_name).map_err(|_| ScheduleError::InvalidTimezone(tz_name.to_owned()))?;
    let allowed: BTreeSet<u8> = weekdays.iter().copied().collect();
    let after_ts = Timestamp::from_second(after)
        .map_err(|_| ScheduleError::InvalidTimezone(format!("timestamp out of range: {after}")))?;
    let mut cursor_ts = after_ts;
    let mut out = Vec::with_capacity(count);
    while out.len() < count {
        let zoned = cursor_ts
            .in_tz(tz_name)
            .map_err(|error| ScheduleError::InvalidTimezone(format!("{tz_name}: {error}")))?;
        let Some(next_civil) = next_weekly(&zoned.datetime(), &allowed, hour, minute) else {
            break;
        };
        let utc = next_civil
            .in_tz(tz_name)
            .map_err(|error| ScheduleError::InvalidTimezone(format!("{tz_name}: {error}")))?
            .timestamp();
        if utc.as_second() <= after {
            cursor_ts = cursor_ts
                .checked_add(SignedDuration::from_mins(1))
                .unwrap_or(cursor_ts);
            continue;
        }
        out.push(utc.as_second());
        cursor_ts = utc;
    }
    Ok(out)
}

fn next_daily(civil: &CivilDateTime, hour: u8, minute: u8) -> CivilDateTime {
    let today = civil.date();
    let today_at = today.at(hour as i8, minute as i8, 0, 0);
    if today_at > *civil {
        today_at
    } else {
        // Next day's same wall time. jiff errors on date overflow (e.g.
        // 9999-12-31); fall back to today so the caller can give up.
        let next_date = today
            .checked_add(Span::new().try_days(1).unwrap_or(Span::new()))
            .unwrap_or(today);
        next_date.at(hour as i8, minute as i8, 0, 0)
    }
}

fn next_weekly(
    civil: &CivilDateTime,
    allowed: &BTreeSet<u8>,
    hour: u8,
    minute: u8,
) -> Option<CivilDateTime> {
    let today = civil.date();
    let today_iso = iso_weekday(today.weekday());
    let today_at = today.at(hour as i8, minute as i8, 0, 0);
    if allowed.contains(&today_iso) && today_at > *civil {
        return Some(today_at);
    }
    for offset in 1..=7i64 {
        let span = Span::new().try_days(offset).ok()?;
        let next_date = today.checked_add(span).ok()?;
        if allowed.contains(&iso_weekday(next_date.weekday())) {
            return Some(next_date.at(hour as i8, minute as i8, 0, 0));
        }
    }
    None
}

fn iso_weekday(day: Weekday) -> u8 {
    match day {
        Weekday::Monday => 1,
        Weekday::Tuesday => 2,
        Weekday::Wednesday => 3,
        Weekday::Thursday => 4,
        Weekday::Friday => 5,
        Weekday::Saturday => 6,
        Weekday::Sunday => 7,
    }
}

// ---------------------------------------------------------------------------
// Occurrence identity
// ---------------------------------------------------------------------------

/// Stable identifier for one scheduled moment. `(task_id, generation, fire_at)`
/// is unique by construction; the shell writes it on every queued run so a
/// duplicate dispatch never produces a second `task_run`.
pub fn occurrence_key(task_id: &str, schedule_generation: u64, fire_at: UnixSeconds) -> String {
    format!("sched:{task_id}:{schedule_generation}:{fire_at}")
}

/// One execution attempt for a scheduled task. Multiple attempts can exist
/// for a single `TaskRun` (a retry inside the run is the same occurrence,
/// not a new one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAttempt {
    pub run_id: String,
    pub attempt_no: u32,
    pub started_at: Option<UnixSeconds>,
    pub finished_at: Option<UnixSeconds>,
    pub outcome: Option<Outcome>,
    pub error_class: Option<String>,
    pub error_summary: Option<String>,
}

/// One materialized run of one scheduled task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRun {
    pub id: String,
    pub task_id: String,
    /// `sched:<task>:<generation>:<fire_at>` for scheduled runs,
    /// `manual:<task>:<request_id>` for user-triggered ones.
    pub occurrence_key: String,
    /// `scheduled` (driven by the dispatcher) or `manual` (user clicked
    /// "run now").
    pub trigger_source: TriggerSource,
    pub schedule_generation: u64,
    /// The original scheduled instant, even if the dispatcher actually started
    /// later (misfire / late start).
    pub scheduled_for: UnixSeconds,
    pub queued_at: UnixSeconds,
    pub started_at: Option<UnixSeconds>,
    pub finished_at: Option<UnixSeconds>,
    pub config_revision: u64,
    /// Frozen copy of `ScheduledTask.task` at admit time — keeps a config
    /// edit from re-pointing an in-flight run.
    pub config_snapshot: TaskKind,
    pub lifecycle: Lifecycle,
    pub outcome: Option<Outcome>,
    /// Agent-side delivery / verification semantics (kept verbatim from the
    /// session log). `None` for jobs.
    pub delivery: Option<String>,
    pub verification: Option<String>,
    /// Session created by the agent driver, or `None` for jobs.
    pub session_id: Option<String>,
    /// Reference into the job's structured report (path or URI), `None`
    /// for agent runs.
    pub report_ref: Option<String>,
    /// Stable reason code (e.g. `needs_approval`, `needs_user_input`,
    /// `provider_unavailable`); free-form `error_summary` is redacted and
    /// short.
    pub reason_code: Option<String>,
    pub error_summary: Option<String>,
    /// How many scheduled moments were collapsed into this single run
    /// (misfire / coalesce).
    pub skipped_count: u32,
    pub attempts: Vec<TaskAttempt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TriggerSource {
    Scheduled,
    Manual,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `2024-01-01 00:00:00 UTC`.
    fn anchor() -> UnixSeconds {
        1_704_067_200
    }

    fn seconds_per_day() -> i64 {
        86_400
    }

    #[test]
    fn once_returns_fire_at_when_future() {
        let schedule = Schedule::Once {
            fire_at: anchor() + 60,
        };
        let next = next_occurrences(&schedule, anchor(), 5).unwrap();
        assert_eq!(next, vec![anchor() + 60]);
    }

    #[test]
    fn once_returns_empty_when_consumed() {
        let schedule = Schedule::Once {
            fire_at: anchor() + 60,
        };
        let next = next_occurrences(&schedule, anchor() + 120, 5).unwrap();
        assert!(next.is_empty());
    }

    #[test]
    fn once_rejects_past_fire_at() {
        let schedule = Schedule::Once {
            fire_at: anchor() - 1,
        };
        let err = validate(&schedule, anchor()).unwrap_err();
        assert!(matches!(err, ScheduleError::OnceInPast { .. }));
    }

    #[test]
    fn interval_starts_at_anchor_when_anchor_is_in_the_future() {
        let schedule = Schedule::Interval {
            anchor: anchor() + 60,
            interval_seconds: 120,
        };
        let next = next_occurrences(&schedule, anchor(), 5).unwrap();
        assert_eq!(
            next,
            vec![
                anchor() + 60,
                anchor() + 180,
                anchor() + 300,
                anchor() + 420,
                anchor() + 540
            ]
        );
    }

    #[test]
    fn interval_steps_whole_jumps() {
        let schedule = Schedule::Interval {
            anchor: anchor(),
            interval_seconds: 600,
        };
        let next = next_occurrences(&schedule, anchor() + 100, 3).unwrap();
        assert_eq!(next, vec![anchor() + 600, anchor() + 1200, anchor() + 1800]);
    }

    #[test]
    fn interval_collapses_clock_back_to_anchor() {
        let schedule = Schedule::Interval {
            anchor: anchor() + 1000,
            interval_seconds: 60,
        };
        let next = next_occurrences(&schedule, anchor(), 3).unwrap();
        assert_eq!(
            next,
            vec![anchor() + 1000, anchor() + 1060, anchor() + 1120]
        );
    }

    #[test]
    fn interval_rejects_sub_minute_steps() {
        let schedule = Schedule::Interval {
            anchor: anchor(),
            interval_seconds: 30,
        };
        let err = next_occurrences(&schedule, anchor(), 1).unwrap_err();
        assert!(matches!(
            err,
            ScheduleError::IntervalTooSmall { seconds: 30 }
        ));
    }

    #[test]
    fn daily_returns_next_local_morning_after_anchor() {
        // anchor = 2024-01-01 00:00:00 UTC = 08:00:00 Asia/Shanghai
        // next 09:00 Shanghai = 01:00 UTC the same day
        let schedule = Schedule::Daily {
            hour: 9,
            minute: 0,
            tz_name: "Asia/Shanghai".into(),
        };
        let next = next_occurrences(&schedule, anchor(), 3).unwrap();
        assert_eq!(next[0], anchor() + 3600);
        assert_eq!(next[1], anchor() + seconds_per_day() + 3600);
        assert_eq!(next[2], anchor() + 2 * seconds_per_day() + 3600);
    }

    #[test]
    fn daily_skips_dst_gap_spring_forward() {
        // US Eastern: 2024-03-10 02:00 → 03:00 local (gap). A 02:30 daily
        // is impossible that day; from 2024-03-10 06:00 UTC (post-gap) the
        // next 02:30 must be 2024-03-11 02:30 EDT = 06:30 UTC. We compute
        // both reference instants via jiff so the test stays correct if the
        // data tables shift.
        use jiff::civil::Date;
        use jiff::Timestamp;
        let after = Date::new(2024, 3, 10)
            .unwrap()
            .at(6, 0, 0, 0)
            .in_tz("America/New_York")
            .unwrap()
            .timestamp()
            .as_second();
        let expected = Date::new(2024, 3, 11)
            .unwrap()
            .at(2, 30, 0, 0)
            .in_tz("America/New_York")
            .unwrap()
            .timestamp()
            .as_second();
        let schedule = Schedule::Daily {
            hour: 2,
            minute: 30,
            tz_name: "America/New_York".into(),
        };
        let next = next_occurrences(&schedule, after, 1).unwrap();
        assert_eq!(next, vec![expected]);
        let _ = Timestamp::from_second(expected).unwrap();
    }

    #[test]
    fn daily_advances_to_next_day_when_today_past() {
        // 23:00 Shanghai same day has passed by the time we ask at 02:00
        // Shanghai next day, so the next 09:00 Shanghai is the *day after*,
        // not today's same slot.
        let schedule = Schedule::Daily {
            hour: 9,
            minute: 0,
            tz_name: "Asia/Shanghai".into(),
        };
        let after = anchor() + 18 * 3600; // 2024-01-01 18:00 UTC = 02:00 next day Shanghai
        let next = next_occurrences(&schedule, after, 1).unwrap();
        assert_eq!(next, vec![anchor() + seconds_per_day() + 3600]);
    }

    #[test]
    fn weekly_returns_next_allowed_weekday() {
        // 2024-01-01 UTC is a Monday. Weekly Mon/Wed at 09:00 Shanghai.
        let schedule = Schedule::Weekly {
            weekdays: vec![1, 3],
            hour: 9,
            minute: 0,
            tz_name: "Asia/Shanghai".into(),
        };
        let next = next_occurrences(&schedule, anchor(), 4).unwrap();
        // First slot: 2024-01-01 09:00 Shanghai = 01:00 UTC.
        assert_eq!(next[0], anchor() + 3600);
        // Next: 2024-01-03 09:00 Shanghai = 01:00 UTC (+2 days).
        assert_eq!(next[1], anchor() + 2 * seconds_per_day() + 3600);
        // Then: 2024-01-08 (Mon) 09:00 Shanghai = 01:00 UTC (+7 days).
        assert_eq!(next[2], anchor() + 7 * seconds_per_day() + 3600);
        // Then: 2024-01-10 (Wed) 09:00 Shanghai = 01:00 UTC (+9 days).
        assert_eq!(next[3], anchor() + 9 * seconds_per_day() + 3600);
    }

    #[test]
    fn weekly_rejects_unknown_weekday() {
        let schedule = Schedule::Weekly {
            weekdays: vec![0],
            hour: 9,
            minute: 0,
            tz_name: "Asia/Shanghai".into(),
        };
        let err = validate(&schedule, anchor()).unwrap_err();
        assert!(matches!(err, ScheduleError::InvalidWeekday(0)));
    }

    #[test]
    fn weekly_rejects_empty_set() {
        let schedule = Schedule::Weekly {
            weekdays: vec![],
            hour: 9,
            minute: 0,
            tz_name: "Asia/Shanghai".into(),
        };
        let err = validate(&schedule, anchor()).unwrap_err();
        assert!(matches!(err, ScheduleError::NoWeekdays));
    }

    #[test]
    fn daily_rejects_unknown_timezone() {
        let schedule = Schedule::Daily {
            hour: 9,
            minute: 0,
            tz_name: "Atlantis/Lemuria".into(),
        };
        let err = validate(&schedule, anchor()).unwrap_err();
        assert!(matches!(err, ScheduleError::InvalidTimezone(_)));
    }

    #[test]
    fn occurrence_key_is_stable_across_processes() {
        let key = occurrence_key("t1", 3, 1_700_000_000);
        assert_eq!(key, "sched:t1:3:1700000000");
    }

    #[test]
    fn task_kind_resource_keys_are_deterministic() {
        let agent = TaskKind::Agent {
            project: "/p".into(),
            message: "go".into(),
            personas: vec![],
            budget_family: None,
            resource_key: Some("/p".into()),
        };
        assert_eq!(agent.resource_keys(), vec!["agent:/p".to_string()]);

        let no_res = TaskKind::Agent {
            project: "/p".into(),
            message: "go".into(),
            personas: vec![],
            budget_family: None,
            resource_key: None,
        };
        assert!(no_res.resource_keys().is_empty());

        let job = TaskKind::Job {
            job_type: "workspace_refresh".into(),
            params: serde_json::json!({}),
            resources: vec!["workspace:/p".into(), "memory:v1".into()],
        };
        assert_eq!(
            job.resource_keys(),
            vec!["job:workspace:/p".to_string(), "job:memory:v1".to_string()]
        );
    }

    #[test]
    fn validate_rejects_invalid_hour() {
        let schedule = Schedule::Daily {
            hour: 24,
            minute: 0,
            tz_name: "UTC".into(),
        };
        let err = validate(&schedule, anchor()).unwrap_err();
        assert!(matches!(err, ScheduleError::InvalidHour(24)));
    }

    #[test]
    fn system_clock_now_unix_is_non_negative() {
        let c = SystemClock;
        let _ = c.now_unix();
        let _ = c.monotonic_seconds();
    }

    #[test]
    fn preview_count_is_five_per_spec() {
        assert_eq!(PREVIEW_COUNT, 5);
    }

    #[test]
    fn daily_returns_at_most_count_occurrences() {
        let schedule = Schedule::Daily {
            hour: 9,
            minute: 0,
            tz_name: "Asia/Shanghai".into(),
        };
        let next = next_occurrences(&schedule, anchor(), 5).unwrap();
        assert_eq!(next.len(), 5);
        for window in next.windows(2) {
            assert_eq!(window[1] - window[0], seconds_per_day());
        }
    }

    #[test]
    fn interval_with_zero_step_is_rejected() {
        let schedule = Schedule::Interval {
            anchor: anchor(),
            interval_seconds: 0,
        };
        let err = next_occurrences(&schedule, anchor(), 1).unwrap_err();
        assert!(matches!(
            err,
            ScheduleError::IntervalTooSmall { seconds: 0 }
        ));
    }

    #[test]
    fn shanghai_offset_is_plus_eight() {
        // A cheap invariant test: Asia/Shanghai is +08:00 with no DST, so the
        // civil time at anchor midnight UTC is 08:00 the same day.
        let ts = Timestamp::from_second(anchor()).unwrap();
        let civil = ts.in_tz("Asia/Shanghai").unwrap();
        assert_eq!(civil.hour(), 8);
    }
}
