//! Updates with Velopack, from our own server (`/updates/*`, served from the releases bucket by server/updates.ts;
//! GitHub can be slow or blocked in China). An update downloads quietly and installs when the app quits, or right
//! away from the start screen's button. Only an installed app updates: a dev build has nothing to do here.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use url::Url;
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, sources::HttpSource};

const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);

#[derive(Clone)]
pub struct Updater {
    manager: UpdateManager,
    ready: Arc<Mutex<Option<UpdateInfo>>>,
}

impl Updater {
    /// Checks now and every few hours, downloading anything new; `on_progress` gets the percent while it downloads
    /// and `on_ready` the version once it's on disk. None when the app isn't installed by Velopack (a dev build or a
    /// bare binary).
    pub fn start(
        server: &Url,
        on_progress: impl Fn(i16) + Send + Sync + 'static,
        on_ready: impl Fn(String) + Send + 'static,
    ) -> Option<Self> {
        let feed = server.join("updates").ok()?;
        let manager = match UpdateManager::new(HttpSource::new(feed.as_str()), None, None) {
            Ok(manager) => manager,
            Err(e) => {
                tracing::info!("updates off: {e}");
                return None;
            }
        };
        tracing::info!("version {}", manager.get_current_version_as_string());
        let updater = Self { manager, ready: Arc::default() };
        std::thread::spawn({
            let updater = updater.clone();
            move || {
                loop {
                    match updater.fetch(&on_progress) {
                        Ok(Some(version)) => {
                            on_ready(version);
                            // Nothing newer will be installed before this one, so stop checking
                            return;
                        }
                        Ok(None) => {}
                        Err(e) => tracing::warn!("update check failed: {e}"),
                    }
                    std::thread::sleep(CHECK_EVERY);
                }
            }
        });
        Some(updater)
    }

    /// Downloads an update if there is one, returning its version.
    fn fetch(&self, on_progress: &(impl Fn(i16) + Sync)) -> Result<Option<String>, velopack::Error> {
        let UpdateCheck::UpdateAvailable(info) = self.manager.check_for_updates()? else { return Ok(None) };
        let version = info.TargetFullRelease.Version.clone();
        tracing::info!("downloading update {version}");
        let (progress, percents) = std::sync::mpsc::channel();
        // The sender is dropped when the download ends, which ends the loop
        std::thread::scope(|scope| {
            scope.spawn(move || percents.iter().for_each(on_progress));
            self.manager.download_updates(&info, Some(progress))
        })?;
        if let Ok(mut ready) = self.ready.lock() {
            *ready = Some(*info);
        }
        Ok(Some(version))
    }

    /// Installs the downloaded update and starts the new version. Ends the process.
    pub fn restart_now(&self) {
        let Some(info) = self.ready.lock().ok().and_then(|ready| ready.clone()) else { return };
        if let Err(e) = self.manager.apply_updates_and_restart(&info) {
            tracing::warn!("update failed: {e}");
        }
    }

    /// Hands a downloaded update to Velopack's updater, which installs it once this process has exited. Quitting
    /// with ⌘Q on macOS ends the process without getting here; Velopack then installs it at the next launch.
    pub fn install_on_exit(&self) {
        let Some(info) = self.ready.lock().ok().and_then(|ready| ready.clone()) else { return };
        if let Err(e) = self.manager.wait_exit_then_apply_updates(&info, true, false, Vec::<String>::new()) {
            tracing::warn!("update failed: {e}");
        }
    }
}
