//! The on-device speech model's files: where they're kept, whether they're all there, and downloading them from our
//! own server (`/updates/*`, the releases bucket; GitHub and Hugging Face are slow or blocked in China). A download
//! picks up where it stopped, and a file only gets its final name once its SHA-256 matches, so a file under its
//! final name is a whole, checked one.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};
use url::Url;

/// SenseVoice-Small, int8: Mandarin and English (and mixed), with punctuation.
const NAME: &str = "sensevoice-small-2024-07-17";

pub(crate) struct ModelFile {
    /// Its name in the model's folder, which sherpa-onnx is pointed at
    pub local: &'static str,
    /// Its name under `/updates/`, versioned so it's cached for good
    remote: &'static str,
    size: u64,
    sha256: &'static str,
}

pub(crate) const MODEL: ModelFile = ModelFile {
    local: "model.int8.onnx",
    remote: "sensevoice-small-int8-2024-07-17.onnx",
    size: 239_233_841,
    sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
};

pub(crate) const TOKENS: ModelFile = ModelFile {
    local: "tokens.txt",
    remote: "sensevoice-small-2024-07-17-tokens.txt",
    size: 315_894,
    sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
};

const FILES: [&ModelFile; 2] = [&MODEL, &TOKENS];

// Tries in a row that get nowhere before the download gives up
const RETRIES: u32 = 5;
const RETRY_AFTER: Duration = Duration::from_secs(3);

/// The whole download, for the UI.
pub const MODEL_SIZE: u64 = MODEL.size + TOKENS.size;

/// Where the on-device model stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelStatus {
    Missing,
    /// Percent done
    Downloading(u8),
    Ready,
    /// The last download stopped on an error; what's done so far is kept for the next try
    Failed,
}

/// The model's folder in the app's data folder.
pub(crate) fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join(NAME)
}

pub(crate) fn is_ready(dir: &Path) -> bool {
    FILES.iter().all(|f| is_whole(dir, f))
}

/// Removes the model and anything half downloaded.
pub(crate) fn delete(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Downloads what's missing into `dir`, on the calling thread. `progress` gets the percent each time it changes.
/// Returns Ok(false) when `cancel` stopped it.
pub(crate) fn download(
    server: &Url,
    dir: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(u8),
) -> Result<bool, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        // A stalled read gives up, and the next try resumes
        .timeout_recv_body(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut done: u64 = FILES.iter().filter(|f| is_whole(dir, f)).map(|f| f.size).sum();
    let mut shown = u8::MAX;
    let mut report = |done: u64| {
        let percent = (done * 100 / MODEL_SIZE).min(100) as u8;
        if percent != shown {
            shown = percent;
            progress(percent);
        }
    };
    report(done);
    for file in FILES {
        if is_whole(dir, file) {
            continue;
        }
        let url = server.join(&format!("updates/{}", file.remote)).map_err(|e| e.to_string())?;
        let part = dir.join(format!("{}.part", file.local));
        // A dropped connection is tried again from where it stopped, as long as each try gets further
        let mut failures = 0;
        loop {
            let before = part_len(&part);
            match fetch(&agent, &url, &part, file, cancel, &mut |n| report(done + n)) {
                Ok(true) => break,
                Ok(false) => return Ok(false),
                Err(e) => {
                    failures = if part_len(&part) > before { 1 } else { failures + 1 };
                    if failures > RETRIES {
                        return Err(e);
                    }
                    tracing::info!("model download dropped, trying again: {e}");
                    std::thread::sleep(RETRY_AFTER);
                    if cancel.load(Ordering::Relaxed) {
                        return Ok(false);
                    }
                }
            }
        }
        if sha256(&part).map_err(|e| e.to_string())? != file.sha256 {
            // Corrupt somewhere: start this file over next time
            let _ = fs::remove_file(&part);
            return Err(format!("{} didn't match its checksum", file.remote));
        }
        fs::rename(&part, dir.join(file.local)).map_err(|e| e.to_string())?;
        done += file.size;
        report(done);
    }
    Ok(true)
}

fn part_len(part: &Path) -> u64 {
    fs::metadata(part).map(|m| m.len()).unwrap_or(0)
}

fn is_whole(dir: &Path, file: &ModelFile) -> bool {
    fs::metadata(dir.join(file.local)).is_ok_and(|m| m.len() == file.size)
}

/// Appends the rest of `file` to `part`, asking only for the bytes it doesn't have yet.
fn fetch(
    agent: &ureq::Agent,
    url: &Url,
    part: &Path,
    file: &ModelFile,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<bool, String> {
    let mut have = part_len(part);
    if have == file.size {
        // Whole, but stopped before its checksum
        return Ok(true);
    }
    if have > file.size {
        have = 0;
    }
    let mut request = agent.get(url.as_str());
    if have > 0 {
        request = request.header("range", format!("bytes={have}-"));
    }
    let response = request.call().map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let mut out = match status {
        206 => OpenOptions::new().append(true).open(part),
        // The whole file: the server ignored the range, or there was nothing yet
        200 => {
            have = 0;
            File::create(part)
        }
        _ => return Err(format!("{} answered {status}", file.remote)),
    }
    .map_err(|e| e.to_string())?;
    progress(have);
    let mut body = response.into_body().into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let n = body.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        out.write_all(buf.get(..n).unwrap_or_default()).map_err(|e| e.to_string())?;
        have += n as u64;
        progress(have);
    }
    out.flush().map_err(|e| e.to_string())?;
    if have != file.size {
        return Err(format!("{} ended at {have} of {} bytes", file.remote, file.size));
    }
    Ok(true)
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(buf.get(..n).unwrap_or_default());
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
