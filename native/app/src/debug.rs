//! Settings' Debug group, tools for checking how calls go, and "Sharpen video", whose comparison lives there.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use engine::{Command, Engine};
use slint::ComponentHandle;

use crate::settings::{self, Settings};
use crate::ui::AppWindow;
use crate::video::SlintSink;

/// What the last "Save frame" did, as the Debug group's `frame-saved` shows it.
#[cfg(not(target_os = "android"))]
mod saved {
    pub const DONE: i32 = 1;
    pub const NOTHING_UPSCALED: i32 = 2;
    pub const FAILED: i32 = 3;
}

pub fn wire(ui: &AppWindow, engine: &Rc<Engine>, settings: &Rc<RefCell<Settings>>, sink: &Arc<SlintSink>) {
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

    let fec = settings.borrow().fec;
    ui.set_fec(fec);
    engine.send(Command::SetFec(fec));
    ui.on_set_fec({
        let engine = engine.clone();
        let settings = settings.clone();
        let weak = ui.as_weak();
        move |on| {
            if let Some(ui) = weak.upgrade() {
                ui.set_fec(on);
            }
            engine.send(Command::SetFec(on));
            let mut settings = settings.borrow_mut();
            settings.fec = on;
            settings.save();
        }
    });

    ui.set_can_show_files(cfg!(desktop) && settings::data_dir().is_some());
    ui.on_show_stats_files(|| {
        if let Some(data) = settings::data_dir() {
            show_folder(&engine::stats_log_dir(&data));
        }
    });

    #[cfg(not(target_os = "android"))]
    wire_upscaling(ui, settings, sink);
    #[cfg(target_os = "android")]
    let _ = sink;
}

#[cfg(not(target_os = "android"))]
fn wire_upscaling(ui: &AppWindow, settings: &Rc<RefCell<Settings>>, sink: &Arc<SlintSink>) {
    use std::sync::atomic::Ordering;

    use crate::fsr::{COMPARE, SHARPEN};

    let sharpen = settings.borrow().sharpen_video;
    SHARPEN.store(sharpen, Ordering::Relaxed);
    ui.set_sharpen_video(sharpen);
    ui.on_set_sharpen_video({
        let settings = settings.clone();
        let weak = ui.as_weak();
        move |on| {
            SHARPEN.store(on, Ordering::Relaxed);
            if let Some(ui) = weak.upgrade() {
                ui.set_sharpen_video(on);
            }
            let mut settings = settings.borrow_mut();
            settings.sharpen_video = on;
            settings.save();
        }
    });

    // Not remembered: a split picture is only for looking at it now
    ui.on_set_compare_upscaling({
        let weak = ui.as_weak();
        move |on| {
            COMPARE.store(on, Ordering::Relaxed);
            if let Some(ui) = weak.upgrade() {
                ui.set_compare_upscaling(on);
            }
        }
    });

    ui.on_save_frame({
        let sink = sink.clone();
        let weak = ui.as_weak();
        move || {
            let sink = sink.clone();
            let weak = weak.clone();
            // Reading frames back waits for the GPU, and PNGs take a moment
            std::thread::spawn(move || {
                let result = save_frames(&sink);
                let _ = weak.upgrade_in_event_loop(move |ui| ui.set_frame_saved(result));
            });
        }
    });
}

/// Saves each upscaled feed's last frame three ways into `frames/` in the data folder and shows the folder.
#[cfg(not(target_os = "android"))]
fn save_frames(sink: &SlintSink) -> i32 {
    let frames = sink.upscaled_frames();
    if frames.is_empty() {
        return saved::NOTHING_UPSCALED;
    }
    let Some(dir) = settings::data_dir().map(|data| data.join("frames")) else { return saved::FAILED };
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    let write = || -> Result<(), String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for (feed, [source, before, after]) in &frames {
            for (kind, pixels) in [("1 as it came", source), ("2 bilinear", before), ("3 fsr", after)] {
                let png = png(pixels.clone()).ok_or("couldn't encode a PNG")?;
                std::fs::write(dir.join(format!("{stamp} {feed} {kind}.png")), png).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    };
    match write() {
        Ok(()) => {
            show_folder(&dir);
            saved::DONE
        }
        Err(e) => {
            tracing::warn!("couldn't save the frames: {e}");
            saved::FAILED
        }
    }
}

#[cfg(not(target_os = "android"))]
fn png(mut pixels: slint::SharedPixelBuffer<slint::Rgba8Pixel>) -> Option<Vec<u8>> {
    use skia_safe::{AlphaType, ColorType, ImageInfo, Pixmap, png_encoder};

    let (width, height) = (i32::try_from(pixels.width()).ok()?, i32::try_from(pixels.height()).ok()?);
    let info = ImageInfo::new((width, height), ColorType::RGBA8888, AlphaType::Opaque, None);
    let row = pixels.width() as usize * 4;
    let pixmap = Pixmap::new(&info, pixels.make_mut_bytes(), row)?;
    let mut png = Vec::new();
    png_encoder::encode(&pixmap, &mut png, &png_encoder::Options::default()).then_some(png)
}

/// Opens a folder in Finder or Explorer, made first if nothing was saved in it yet.
fn show_folder(dir: &std::path::Path) {
    if cfg!(mobile) {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!("couldn't make {}: {e}", dir.display());
        return;
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(opener).arg(dir).spawn() {
        tracing::warn!("couldn't show {}: {e}", dir.display());
    }
}
