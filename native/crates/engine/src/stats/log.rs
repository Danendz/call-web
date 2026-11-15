//! "Record call stats" in Settings' Debug group: each call's stats once a second as a line of JSON, in a file of its
//! own, so calls with different settings or networks can be compared afterwards.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use super::CallStats;

/// Older files are deleted past this many, so a forgotten switch can't fill the disk (an hour's call is ~3 MB).
const KEEP: usize = 30;

/// The folder in the app's data folder the files go into.
pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("call-stats")
}

/// What the call was set to, written on every line: it can change mid-call.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Conditions {
    pub video_codec: crate::VideoCodec,
    pub screen_kbps: u64,
    pub force_relay: bool,
    pub sharing: bool,
    pub camera_on: bool,
    /// FlexFEC was on on this side when the connection was made
    pub fec: bool,
}

#[derive(Serialize)]
struct Line<'a> {
    /// Seconds since the file was started
    t: f64,
    conditions: &'a Conditions,
    #[serde(flatten)]
    stats: &'a CallStats,
}

pub(crate) struct Recorder {
    file: BufWriter<File>,
    started: Instant,
}

impl Recorder {
    pub fn start(data_dir: &Path) -> Option<Self> {
        let dir = dir(data_dir);
        let open = || -> std::io::Result<File> {
            fs::create_dir_all(&dir)?;
            prune(&dir, KEEP - 1);
            // Sorts by name in the order calls happened
            let name = chrono::Local::now().format("%Y-%m-%d %H-%M-%S.jsonl").to_string();
            File::create(dir.join(name))
        };
        match open() {
            Ok(file) => Some(Self { file: BufWriter::new(file), started: Instant::now() }),
            Err(e) => {
                tracing::warn!("couldn't start the stats log: {e}");
                None
            }
        }
    }

    /// False once the file can't be written to, so the caller stops trying.
    pub fn write(&mut self, stats: &CallStats, conditions: &Conditions) -> bool {
        let line = Line { t: (self.started.elapsed().as_secs_f64() * 10.0).round() / 10.0, conditions, stats };
        // Flushed every line, so a crash loses at most the second it happened in
        let written = serde_json::to_writer(&mut self.file, &line)
            .map_err(std::io::Error::from)
            .and_then(|()| self.file.write_all(b"\n"))
            .and_then(|()| self.file.flush());
        if let Err(e) = written {
            tracing::warn!("couldn't write the stats log: {e}");
            return false;
        }
        true
    }
}

/// Deletes the oldest logs until `keep` are left. Only touches `.jsonl` files, the only kind this folder gets.
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut logs: Vec<PathBuf> =
        entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "jsonl")).collect();
    logs.sort();
    let extra = logs.len().saturating_sub(keep);
    for old in logs.iter().take(extra) {
        if let Err(e) = fs::remove_file(old) {
            tracing::warn!("couldn't delete an old stats log: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_the_newest_logs_and_nothing_else_is_touched() {
        let dir = std::env::temp_dir().join(format!("call-stats-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for name in ["2026-01-01 10-00-00.jsonl", "2026-01-02 10-00-00.jsonl", "2026-01-03 10-00-00.jsonl", "notes.txt"]
        {
            fs::write(dir.join(name), "").unwrap();
        }
        prune(&dir, 2);
        let mut left: Vec<String> =
            fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(left, ["2026-01-02 10-00-00.jsonl", "2026-01-03 10-00-00.jsonl", "notes.txt"]);
    }
}
