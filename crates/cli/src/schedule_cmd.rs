//! `kodo schedule <verb>` — terminal surface for the scheduler.
//!
//! First-version scope: read-only listing + lifecycle (run-now / cancel /
//! enable / disable / archive). The GUI owns the create / edit flow
//! because the schedule rule (Daily / Weekly / Interval / Once) wants a
//! real form; the CLI is the diagnostic companion you can reach for over
//! SSH or in a CI box without a display.
//!
//! Storage lives in `<state_dir>/scheduler/v1/scheduler.sqlite`, the same
//! path the desktop reads from — so a `kodo schedule list` reflects the
//! GUI's view of the world. We boot a fresh `Scheduler` with the system
//! clock; this CLI never runs the dispatcher, so no tick thread is
//! started and no Agent is launched here.

use std::sync::Arc;

use kodo_core::scheduler::{SystemClock, TaskState};
use kodo_core::settings;
use kodo_shell::{Scheduler, SchedulerError};

use crate::args::{ScheduleCmd, ScheduleVerb};

pub const USAGE: &str = "\
用法: kodo schedule <verb> [选项]

查看与控制本地 Scheduler 存储。GUI 与本 CLI 共享同一份数据库；本端只读 +
生命周期操作（run-now / cancel / enable / disable / archive），新建 / 编辑
请在桌面端 Kodo 完成。

子命令:
  list [--limit N] [--include-archived]   列出任务（默认排除已归档）
  show <task-id>                         显示一个任务的时间规则与配置
  history <task-id> [--limit N]          列出该任务的运行历史
  run-now <task-id>                      立即入队一次执行（独立 occurrence_key）
  cancel <task-id> <run-id>              取消一个仍在队列或执行中的运行
  enable <task-id>                       恢复（从 paused 到 enabled）
  disable <task-id>                      暂停（从 enabled 到 paused）
  archive <task-id>                      归档（先 pause 再 archive）
  job-types                              列出已注册的 job_type
";

pub fn run(cmd: ScheduleCmd) -> i32 {
    if cmd.verb.is_none() {
        print!("{USAGE}");
        return 0;
    }
    let Some(scheduler) = open_scheduler() else {
        return 1;
    };
    match cmd.verb.unwrap() {
        ScheduleVerb::List => run_list(&scheduler, cmd.limit, cmd.include_archived),
        ScheduleVerb::Show => match cmd.task_id.as_deref() {
            Some(id) => run_show(&scheduler, id),
            None => {
                eprintln!("show 需要 <task-id>");
                2
            }
        },
        ScheduleVerb::History => match cmd.task_id.as_deref() {
            Some(id) => run_history(&scheduler, id, cmd.limit),
            None => {
                eprintln!("history 需要 <task-id>");
                2
            }
        },
        ScheduleVerb::RunNow => match cmd.task_id.as_deref() {
            Some(id) => run_now(&scheduler, id),
            None => {
                eprintln!("run-now 需要 <task-id>");
                2
            }
        },
        ScheduleVerb::Cancel => match (cmd.task_id.as_deref(), cmd.run_id.as_deref()) {
            (Some(task_id), Some(run_id)) => run_cancel(&scheduler, task_id, run_id),
            _ => {
                eprintln!("cancel 需要 <task-id> <run-id>");
                2
            }
        },
        ScheduleVerb::Enable => match cmd.task_id.as_deref() {
            Some(id) => set_enabled(&scheduler, id, true),
            None => {
                eprintln!("enable 需要 <task-id>");
                2
            }
        },
        ScheduleVerb::Disable => match cmd.task_id.as_deref() {
            Some(id) => set_enabled(&scheduler, id, false),
            None => {
                eprintln!("disable 需要 <task-id>");
                2
            }
        },
        ScheduleVerb::Archive => match cmd.task_id.as_deref() {
            Some(id) => run_archive(&scheduler, id),
            None => {
                eprintln!("archive 需要 <task-id>");
                2
            }
        },
        ScheduleVerb::JobTypes => {
            for t in scheduler.jobs().types() {
                println!("{t}");
            }
            0
        }
    }
}

fn open_scheduler() -> Option<Scheduler> {
    let state_dir = settings::state_dir()?;
    let scheduler = match Scheduler::open(&state_dir, Arc::new(SystemClock)) {
        Ok(s) => s,
        Err(error) => {
            eprintln!("打开 Scheduler 存储失败：{error}");
            return None;
        }
    };
    Some(scheduler)
}

fn run_list(scheduler: &Scheduler, limit: Option<usize>, include_archived: bool) -> i32 {
    match scheduler.store().list_tasks(include_archived) {
        Ok(tasks) => {
            let limit = limit.unwrap_or(tasks.len());
            for task in tasks.iter().take(limit) {
                println!(
                    "{id:<28} {state:<8} {revision:<3} {next:<12} {title}",
                    id = task.id,
                    state = task_state_label(task.state),
                    revision = task.revision,
                    next = match task.next_fire_at {
                        Some(ts) => ts.to_string(),
                        None => "—".to_owned(),
                    },
                    title = task.title,
                );
            }
            0
        }
        Err(error) => {
            eprintln!("list 失败：{error}");
            1
        }
    }
}

fn run_show(scheduler: &Scheduler, task_id: &str) -> i32 {
    match scheduler.store().get_task(task_id) {
        Ok(Some(task)) => {
            println!("id                 {}", task.id);
            println!("title              {}", task.title);
            println!("state              {}", task_state_label(task.state));
            println!("revision           {}", task.revision);
            println!("schedule_generation {}", task.schedule_generation);
            println!("timezone           {}", task.timezone);
            println!("misfire_policy     {}", misfire_label(task.misfire_policy));
            println!("overlap_policy     {}", overlap_label(task.overlap_policy));
            println!("grace_seconds      {}", task.grace_seconds);
            if let Some(t) = task.timeout_seconds {
                println!("timeout_seconds    {t}");
            } else {
                println!("timeout_seconds    —");
            }
            println!(
                "next_fire_at       {}",
                task.next_fire_at
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "—".to_owned())
            );
            println!("archived           {}", task.archived);
            println!("schedule           {}", schedule_label(&task.schedule));
            println!("task               {}", task_kind_label(&task.task));
            0
        }
        Ok(None) => {
            println!("(not found)");
            0
        }
        Err(error) => {
            eprintln!("show 失败：{error}");
            1
        }
    }
}

fn run_history(scheduler: &Scheduler, task_id: &str, limit: Option<usize>) -> i32 {
    let limit = limit.unwrap_or(20);
    match scheduler.store().list_runs(task_id, limit) {
        Ok(runs) => {
            if runs.is_empty() {
                println!("(no runs)");
                return 0;
            }
            for run in runs {
                println!(
                    "{id:<32} {lifecycle:<10} {outcome:<12} {scheduled_for:<12} {reason}",
                    id = run.id,
                    lifecycle = lifecycle_label(run.lifecycle),
                    outcome = run.outcome.map(outcome_label).unwrap_or("—"),
                    scheduled_for = run.scheduled_for,
                    reason = run.reason_code.as_deref().unwrap_or("—"),
                );
            }
            0
        }
        Err(error) => {
            eprintln!("history 失败：{error}");
            1
        }
    }
}

fn run_now(scheduler: &Scheduler, task_id: &str) -> i32 {
    // `enqueue_manual` uses `manual:<task_id>:<request_id>` as the
    // dedup key, so we have to give it a unique request id per call.
    let request_id = format!("cli-{}", unix_now_nano());
    match scheduler.enqueue_manual(task_id, &request_id) {
        Ok(dispatched) => {
            println!("queued {}", dispatched.run.id);
            0
        }
        Err(error) => {
            eprintln!("run-now 失败：{error}");
            scheduler_error_code(error)
        }
    }
}

fn run_cancel(scheduler: &Scheduler, _task_id: &str, run_id: &str) -> i32 {
    match scheduler.request_cancel(run_id) {
        Ok(true) => {
            println!("cancelling {run_id}");
            0
        }
        Ok(false) => {
            println!("{run_id} 不在队列或执行中（可能已经结束）");
            0
        }
        Err(error) => {
            eprintln!("cancel 失败：{error}");
            scheduler_error_code(error)
        }
    }
}

fn set_enabled(scheduler: &Scheduler, task_id: &str, enabled: bool) -> i32 {
    match scheduler.set_enabled(task_id, enabled) {
        Ok(task) => {
            println!(
                "{} {}",
                if enabled { "enabled" } else { "disabled" },
                task.id
            );
            0
        }
        Err(error) => {
            eprintln!("set_enabled 失败：{error}");
            scheduler_error_code(error)
        }
    }
}

fn run_archive(scheduler: &Scheduler, task_id: &str) -> i32 {
    // Archive wants a paused task — the store refuses an enabled one
    // (its resources might still be live). Pause first; the store
    // re-reads on each call so a previously-paused task is a no-op.
    if let Err(error) = scheduler.set_enabled(task_id, false) {
        eprintln!("pause 失败：{error}");
        return scheduler_error_code(error);
    }
    match scheduler.archive_task(task_id) {
        Ok(()) => {
            println!("archived {task_id}");
            0
        }
        Err(error) => {
            eprintln!("archive 失败：{error}");
            scheduler_error_code(error)
        }
    }
}

fn scheduler_error_code(error: SchedulerError) -> i32 {
    // Mirror the GUI mapping: unknown-task / unknown-job / revision
    // conflict are user errors (exit 2), the rest are storage / runtime
    // problems (exit 1). The error `Display` is a stable code string
    // already, so we re-print it for the user to correlate with the
    // GUI's inline messages.
    match error {
        SchedulerError::UnknownTask
        | SchedulerError::UnknownJob
        | SchedulerError::RevisionConflict
        | SchedulerError::InvalidSchedule
        | SchedulerError::AlreadyFinished
        | SchedulerError::NoLiveRun
        | SchedulerError::ResourceBusy => 2,
        SchedulerError::StorageUnavailable => 1,
    }
}

fn task_state_label(state: TaskState) -> &'static str {
    match state {
        TaskState::Enabled => "enabled",
        TaskState::Paused => "paused",
        TaskState::Finished => "finished",
        TaskState::Archived => "archived",
    }
}

fn misfire_label(policy: kodo_core::scheduler::MisfirePolicy) -> &'static str {
    use kodo_core::scheduler::MisfirePolicy::*;
    match policy {
        LatestOnce => "latestOnce",
        Skip => "skip",
        Coalesce => "coalesce",
    }
}

fn overlap_label(policy: kodo_core::scheduler::OverlapPolicy) -> &'static str {
    use kodo_core::scheduler::OverlapPolicy::*;
    match policy {
        Skip => "skip",
        QueueLatest => "queueLatest",
        Allow => "allow",
    }
}

fn schedule_label(schedule: &kodo_core::scheduler::Schedule) -> String {
    use kodo_core::scheduler::Schedule::*;
    match schedule {
        Once { fire_at } => format!("once@{fire_at}"),
        Interval {
            anchor,
            interval_seconds,
        } => format!("interval@{anchor}/{interval_seconds}s"),
        Daily {
            hour,
            minute,
            tz_name,
        } => format!("daily@{hour:02}:{minute:02} {tz_name}"),
        Weekly {
            weekdays,
            hour,
            minute,
            tz_name,
        } => {
            let days: Vec<String> = weekdays.iter().map(weekday_label).collect();
            format!("weekly@{} {hour:02}:{minute:02} {tz_name}", days.join(","))
        }
    }
}

fn weekday_label(day: &u8) -> String {
    match day {
        1 => "Mon".into(),
        2 => "Tue".into(),
        3 => "Wed".into(),
        4 => "Thu".into(),
        5 => "Fri".into(),
        6 => "Sat".into(),
        7 => "Sun".into(),
        other => format!("?{other}"),
    }
}

fn task_kind_label(kind: &kodo_core::scheduler::TaskKind) -> String {
    use kodo_core::scheduler::TaskKind::*;
    match kind {
        Agent {
            project,
            message,
            personas,
            budget_family,
            mode,
            resource_key,
        } => {
            let personas = if personas.is_empty() {
                "—".to_owned()
            } else {
                personas.join(",")
            };
            let budget = budget_family.clone().unwrap_or_else(|| "—".to_owned());
            // A task written before the field existed has no mode; it runs on
            // the legacy Code path, so that is what the label says.
            let mode = mode.clone().unwrap_or_else(|| "code".to_owned());
            let resource = resource_key.clone().unwrap_or_else(|| "—".to_owned());
            format!(
                "agent(project={project}, mode={mode}, personas={personas}, budget={budget}, resource={resource}, message={message:?})"
            )
        }
        Job {
            job_type,
            params,
            resources,
        } => format!("job(type={job_type}, resources={resources:?}, params={params})"),
    }
}

fn lifecycle_label(l: kodo_core::scheduler::Lifecycle) -> &'static str {
    use kodo_core::scheduler::Lifecycle::*;
    match l {
        Queued => "queued",
        Running => "running",
        Cancelling => "cancelling",
        Finished => "finished",
    }
}

fn outcome_label(o: kodo_core::scheduler::Outcome) -> &'static str {
    use kodo_core::scheduler::Outcome::*;
    match o {
        Succeeded => "succeeded",
        Partial => "partial",
        Blocked => "blocked",
        Failed => "failed",
        Cancelled => "cancelled",
        TimedOut => "timedOut",
        Interrupted => "interrupted",
        Skipped => "skipped",
    }
}

fn unix_now_nano() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_text_lists_each_verb() {
        for token in [
            "list",
            "show",
            "history",
            "run-now",
            "cancel",
            "enable",
            "disable",
            "archive",
            "job-types",
        ] {
            assert!(USAGE.contains(token), "USAGE missing token {token}");
        }
    }

    #[test]
    fn scheduler_error_code_maps_storage_to_1() {
        assert_eq!(scheduler_error_code(SchedulerError::StorageUnavailable), 1);
    }

    #[test]
    fn scheduler_error_code_maps_user_errors_to_2() {
        for err in [
            SchedulerError::UnknownTask,
            SchedulerError::UnknownJob,
            SchedulerError::RevisionConflict,
            SchedulerError::InvalidSchedule,
            SchedulerError::AlreadyFinished,
            SchedulerError::NoLiveRun,
            SchedulerError::ResourceBusy,
        ] {
            assert_eq!(scheduler_error_code(err), 2, "{err}");
        }
    }

    #[test]
    fn schedule_label_round_trips_each_variant() {
        use kodo_core::scheduler::Schedule::*;
        assert_eq!(
            schedule_label(&Once {
                fire_at: 1_700_000_000
            }),
            "once@1700000000"
        );
        assert_eq!(
            schedule_label(&Interval {
                anchor: 1_700_000_000,
                interval_seconds: 600
            }),
            "interval@1700000000/600s"
        );
        assert_eq!(
            schedule_label(&Daily {
                hour: 9,
                minute: 30,
                tz_name: "Asia/Shanghai".into()
            }),
            "daily@09:30 Asia/Shanghai"
        );
        assert_eq!(
            schedule_label(&Weekly {
                weekdays: vec![1, 3, 5],
                hour: 8,
                minute: 0,
                tz_name: "UTC".into()
            }),
            "weekly@Mon,Wed,Fri 08:00 UTC"
        );
    }

    #[test]
    fn task_kind_label_mentions_agent_when_present() {
        let kind = kodo_core::scheduler::TaskKind::Agent {
            project: "/tmp/p".into(),
            message: "hello".into(),
            personas: vec!["p1".into()],
            mode: None,
            budget_family: None,
            resource_key: Some("/tmp/p".into()),
        };
        let s = task_kind_label(&kind);
        assert!(s.starts_with("agent("), "{s}");
        assert!(s.contains("/tmp/p"), "{s}");
    }
}
