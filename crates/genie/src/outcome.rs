//! How an agent's run ended, and what follows — one policy for turns and live sessions.
//!
//! The turn path (`runtime::finish`) and the session path (`sessions::settled`,
//! `sessions::on_exit`) only say what happened ([`Outcome`]). This module counts the
//! failures in a row and the backoff before the next start, sets the member's state on the
//! board, gives up after `maxAttempts` and tells the orchestrator. Each path keeps its own
//! mechanics: turn rows, mail leases, tokens, the process.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::runtime::{self, AgentKey};
use crate::state::App;

/// How a run ended.
#[derive(Debug, Clone, Copy)]
pub enum Outcome<'a> {
    /// The run ended without failing (a step genie aborted — an interrupt, a stuck step — counts here).
    Ran,
    /// The model or the harness reported an error; `log` is the end of the run's log.
    Failed { error: &'a str, log: &'a str },
    /// The live session's process ended unexpectedly; `at_work` while a run was going.
    Crashed { code: Option<i32>, stderr: &'a str, at_work: bool },
    /// The live session was stopped on purpose (idle, settings changed, team stopped).
    Stopped,
}

/// What [`record`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    /// Failures in a row, this one included.
    pub failures: u32,
    /// This failure was the last allowed: the agent stopped trying.
    pub gave_up: bool,
    /// When the agent may start again.
    pub retry_in: Duration,
}

/// Failures in a row of each agent, and when it may start again.
#[derive(Default)]
pub struct Attempts(Mutex<HashMap<AgentKey, (u32, Instant)>>);

impl Attempts {
    fn with<T>(&self, f: impl FnOnce(&mut HashMap<AgentKey, (u32, Instant)>) -> T) -> T {
        f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn failures(&self, key: &AgentKey) -> u32 {
        self.with(|m| m.get(key).map(|b| b.0).unwrap_or(0))
    }

    /// Not waiting out a backoff.
    pub fn may_start(&self, key: &AgentKey) -> bool {
        self.with(|m| m.get(key).is_none_or(|(_, until)| *until <= Instant::now()))
    }

    /// How long until the agent may start again; `None` if it may now.
    pub fn wait_for(&self, key: &AgentKey) -> Option<Duration> {
        self.with(|m| m.get(key).and_then(|(_, until)| until.checked_duration_since(Instant::now())))
    }

    /// A start that did not get as far as a run (no prepared turn, no process): it counts
    /// and waits, but nobody is told — a missing LiteLLM key is fixed by a person, and
    /// [`Attempts::retry_now`] lets the agent go at once.
    pub fn failed_to_start(&self, key: &AgentKey) -> u32 {
        self.fail(key)
    }

    fn fail(&self, key: &AgentKey) -> u32 {
        self.with(|m| {
            let n = m.get(key).map(|x| x.0 + 1).unwrap_or(1);
            m.insert(key.clone(), (n, Instant::now() + backoff(n)));
            n
        })
    }

    pub fn clear(&self, key: &AgentKey) {
        self.with(|m| m.remove(key));
    }

    /// Agents waiting out a backoff may start at once; their failures still count.
    pub fn retry_now(&self) {
        let now = Instant::now();
        self.with(|m| m.values_mut().for_each(|b| b.1 = now));
    }
}

/// Wait before the next start after `n` failures in a row: 1s after the first,
/// then 10s, 20s, 40s… up to 10 minutes.
pub fn backoff(n: u32) -> Duration {
    match n {
        0 => Duration::ZERO,
        1 => Duration::from_secs(1),
        n => Duration::from_secs((5u64 << (n - 1).min(7)).min(600)),
    }
}

/// Record how a run of `key` ended (blocking: it writes the board and the orchestrator's mail).
pub fn record(app: &App, key: &AgentKey, task: Option<&str>, outcome: Outcome) -> Verdict {
    let max = app.cfg.runtime.max_attempts.max(1);
    let failures = match outcome {
        Outcome::Ran | Outcome::Stopped => {
            app.attempts.clear(key);
            0
        }
        Outcome::Failed { .. } | Outcome::Crashed { .. } => app.attempts.fail(key),
    };
    let gave_up = failures >= max;
    match outcome {
        Outcome::Failed { error, log } if gave_up => give_up(app, key, task, error, failures, &tail(log)),
        Outcome::Crashed { code, stderr, .. } if gave_up => {
            give_up(app, key, task, &format!("the session ended unexpectedly, exit {code:?}"), failures, &tail(stderr))
        }
        // It restarts with the same conversation: a note in its own mailbox has it resume.
        Outcome::Crashed { code, at_work: true, .. } => {
            let _ = runtime::note_to_self(
                app,
                key,
                &format!("[genie] Your session restarted after a crash (exit {code:?}). Continue where you left off."),
            );
        }
        _ => {
            if let AgentKey::Member { project, team, member } = key {
                let _ = app.with_tracker(project, |t| t.bus().member_idle(team, member));
            }
        }
    }
    Verdict { failures, gave_up, retry_in: backoff(failures) }
}

fn give_up(app: &App, key: &AgentKey, task: Option<&str>, error: &str, failures: u32, tail: &str) {
    let text = match key {
        AgentKey::Member { project, team, member } => {
            let _ = app.with_tracker(project, |t| t.bus().member_gave_up(team, member, error, failures));
            format!(
                "{} failed {failures} runs in a row ({error}). Its mail is kept. Restart it (`genie team restart {team} {member}`) or replace it.\n{tail}",
                key.label()
            )
        }
        // The orchestrator keeps trying with backoff; jobs have their own attempts (`finish_job`).
        AgentKey::Orchestrator { .. } => {
            eprintln!(
                "genie runtime: {}: the orchestrator failed {failures} times in a row; retrying with backoff: {error}",
                key.project()
            );
            return;
        }
        AgentKey::Job { .. } => return,
    };
    let _ = runtime::tell_orchestrator(app, key.project(), task, &text);
}

/// The end of a log, for a notice.
fn tail(log: &str) -> String {
    let n = log.chars().count();
    log.chars().skip(n.saturating_sub(600)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_agent_says_how_long_it_waits() {
        let attempts = Attempts::default();
        let key = AgentKey::Orchestrator { project: "shop".into() };
        assert_eq!(attempts.wait_for(&key), None);
        attempts.failed_to_start(&key);
        assert!(attempts.wait_for(&key).is_some_and(|d| d <= Duration::from_secs(1)));
        attempts.retry_now();
        assert_eq!(attempts.wait_for(&key), None);
    }

    #[test]
    fn backoff_starts_at_once_and_levels_off_at_ten_minutes() {
        let secs: Vec<u64> = (0..=9).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, vec![0, 1, 10, 20, 40, 80, 160, 320, 600, 600]);
    }
}
