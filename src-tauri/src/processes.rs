//! Recent output and restart details for running dev servers.
//!
//! The desktop UI receives process output as Tauri events, which the gateway
//! cannot see. That left the phone able to show a preview but not *why* it was
//! blank: a failed build looks identical to a slow one.
//!
//! This keeps a bounded tail of each process's output, plus the command that
//! started it, so the phone can read logs and restart a server that died.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// Lines kept per process. Enough to cover a stack trace or a failed build
/// without letting a chatty server grow without bound.
const MAX_LINES: usize = 400;

/// Characters kept per line, so one enormous line cannot blow up memory.
const MAX_LINE: usize = 2000;

#[derive(Clone, serde::Serialize)]
pub struct LogLine {
    /// Monotonic within a process, so a client can ask for "everything after n".
    pub seq: u64,
    /// "stdout" or "stderr".
    pub stream: String,
    pub text: String,
}

#[derive(Clone)]
struct Entry {
    lines: VecDeque<LogLine>,
    next_seq: u64,
    /// Where the process was started and what was run, so it can be restarted.
    project_path: String,
    command: String,
    script_name: String,
}

#[derive(Clone)]
pub struct Registry {
    inner: Arc<Mutex<HashMap<String, Entry>>>,
    /// Fires with the process key whenever a line is appended.
    tx: broadcast::Sender<String>,
}

impl Default for Registry {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(64);
        Registry {
            inner: Arc::new(Mutex::new(HashMap::new())),
            tx,
        }
    }
}

impl Registry {
    /// Record how a process was started, so it can be restarted later.
    pub fn register(&self, key: &str, project_path: &str, script_name: &str, command: &str) {
        let mut map = self.inner.lock().unwrap();
        let e = map.entry(key.to_string()).or_insert_with(|| Entry {
            lines: VecDeque::new(),
            next_seq: 0,
            project_path: String::new(),
            command: String::new(),
            script_name: String::new(),
        });
        e.project_path = project_path.to_string();
        e.command = command.to_string();
        e.script_name = script_name.to_string();
        // A restart should not replay the previous run's output.
        e.lines.clear();
    }

    pub fn append(&self, key: &str, stream: &str, text: &str) {
        let text = text.trim_end_matches(['\n', '\r']);
        let text = if text.chars().count() > MAX_LINE {
            text.chars().take(MAX_LINE).collect::<String>()
        } else {
            text.to_string()
        };

        {
            let mut map = self.inner.lock().unwrap();
            let Some(e) = map.get_mut(key) else { return };
            let seq = e.next_seq;
            e.next_seq += 1;
            e.lines.push_back(LogLine {
                seq,
                stream: stream.to_string(),
                text,
            });
            while e.lines.len() > MAX_LINES {
                e.lines.pop_front();
            }
        }

        // No receivers simply means nobody is watching the logs.
        let _ = self.tx.send(key.to_string());
    }

    /// Lines with `seq` greater than `after`, oldest first.
    ///
    /// `after` of `None` returns the whole retained tail.
    pub fn since(&self, key: &str, after: Option<u64>) -> Vec<LogLine> {
        let map = self.inner.lock().unwrap();
        let Some(e) = map.get(key) else { return Vec::new() };
        e.lines
            .iter()
            .filter(|l| after.map(|a| l.seq > a).unwrap_or(true))
            .cloned()
            .collect()
    }

    /// How the process was started: (project path, script name, command).
    pub fn command_for(&self, key: &str) -> Option<(String, String, String)> {
        let map = self.inner.lock().unwrap();
        map.get(key)
            .filter(|e| !e.command.is_empty())
            .map(|e| (e.project_path.clone(), e.script_name.clone(), e.command.clone()))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    /// Drop a process's retained output, keeping its restart details.
    pub fn clear(&self, key: &str) {
        if let Some(e) = self.inner.lock().unwrap().get_mut(key) {
            e.lines.clear();
        }
    }

    pub fn known_keys(&self) -> Vec<String> {
        self.inner.lock().unwrap().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Registry {
        let r = Registry::default();
        r.register("proj:dev", "D:/proj", "dev", "npm run dev");
        r
    }

    #[test]
    fn keeps_output_in_order() {
        let r = reg();
        r.append("proj:dev", "stdout", "one");
        r.append("proj:dev", "stderr", "two");

        let lines = r.since("proj:dev", None);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "one");
        assert_eq!(lines[0].seq, 0);
        assert_eq!(lines[1].stream, "stderr");
        assert_eq!(lines[1].seq, 1);
    }

    #[test]
    fn since_returns_only_what_is_new() {
        let r = reg();
        for i in 0..5 {
            r.append("proj:dev", "stdout", &format!("line {i}"));
        }
        let tail = r.since("proj:dev", Some(2));
        assert_eq!(tail.len(), 2, "expected seq 3 and 4, got {tail:?}", tail = tail.len());
        assert_eq!(tail[0].seq, 3);
    }

    #[test]
    fn output_is_bounded_but_sequence_keeps_climbing() {
        let r = reg();
        for i in 0..(MAX_LINES + 50) {
            r.append("proj:dev", "stdout", &format!("line {i}"));
        }
        let lines = r.since("proj:dev", None);
        assert_eq!(lines.len(), MAX_LINES, "buffer grew past its cap");
        // Oldest lines were dropped, so the first seq is not 0.
        assert_eq!(lines[0].seq, 50);
        assert_eq!(lines.last().unwrap().seq as usize, MAX_LINES + 49);
    }

    #[test]
    fn a_single_huge_line_is_truncated() {
        let r = reg();
        r.append("proj:dev", "stdout", &"x".repeat(MAX_LINE * 3));
        let lines = r.since("proj:dev", None);
        assert_eq!(lines[0].text.chars().count(), MAX_LINE);
    }

    #[test]
    fn trailing_newlines_are_stripped() {
        let r = reg();
        r.append("proj:dev", "stdout", "hello\r\n");
        assert_eq!(r.since("proj:dev", None)[0].text, "hello");
    }

    #[test]
    fn output_for_an_unknown_process_is_dropped() {
        let r = reg();
        r.append("ghost:dev", "stdout", "noise");
        assert!(r.since("ghost:dev", None).is_empty());
    }

    #[test]
    fn remembers_how_to_restart() {
        let r = reg();
        let (path, script, cmd) = r.command_for("proj:dev").unwrap();
        assert_eq!(path, "D:/proj");
        assert_eq!(script, "dev");
        assert_eq!(cmd, "npm run dev");
        assert!(r.command_for("other:dev").is_none());
    }

    #[test]
    fn restarting_does_not_replay_the_previous_run() {
        let r = reg();
        r.append("proj:dev", "stdout", "from the old run");
        r.register("proj:dev", "D:/proj", "dev", "npm run dev");
        assert!(r.since("proj:dev", None).is_empty(), "stale output survived a restart");
        assert!(r.command_for("proj:dev").is_some(), "restart details were lost");
    }

    #[test]
    fn clearing_keeps_the_restart_details() {
        let r = reg();
        r.append("proj:dev", "stdout", "noise");
        r.clear("proj:dev");
        assert!(r.since("proj:dev", None).is_empty());
        assert!(r.command_for("proj:dev").is_some());
    }
}
