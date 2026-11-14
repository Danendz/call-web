//! Settings' Debug group: tools for checking how calls go, apart from the settings everyone uses.

use std::cell::RefCell;
use std::rc::Rc;

use engine::{Command, Engine};
use slint::ComponentHandle;

use crate::settings::{self, Settings};
use crate::ui::AppWindow;

pub fn wire(ui: &AppWindow, engine: &Rc<Engine>, settings: &Rc<RefCell<Settings>>) {
    let record = settings.borrow().record_stats;
    ui.set_record_stats(record);
    engine.send(Command::SetStatsLog(record));
    ui.on_set_record_stats({
        let engine = engine.clone();
        let settings = settings.clone();
        let weak = ui.as_weak();
        move |on| {
            if let Some(ui) = weak.upgrade() {
                ui.set_record_stats(on);
            }
            engine.send(Command::SetStatsLog(on));
            let mut settings = settings.borrow_mut();
            settings.record_stats = on;
            settings.save();
        }
    });

    ui.set_can_show_files(cfg!(not(target_os = "android")) && settings::data_dir().is_some());
    ui.on_show_stats_files(show_stats_files);
}

/// Opens the stats logs' folder in Finder or Explorer, made first if no call was recorded yet.
fn show_stats_files() {
    let Some(dir) = settings::data_dir().map(|data| engine::stats_log_dir(&data)) else { return };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("couldn't make the stats folder: {e}");
        return;
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(opener).arg(&dir).spawn() {
        tracing::warn!("couldn't show the stats folder: {e}");
    }
}
