//! A picture of the call, like the web's `screenshot.ts`: the stage's video at its full resolution, your tile in its
//! corner, and a small date stamp, saved as a JPEG in `Pictures/Call Danya/` (on Android through the gallery, which is
//! the only way in there). Drawn with Skia, which the renderer already links, from the frames the window is showing.

#[cfg(not(target_os = "android"))]
use std::path::{Path, PathBuf};

use crate::video::Pixels;
use chrono::{Datelike, Local};
use engine::Lang;
use skia_safe::{
    AlphaType, Color, Color4f, ColorType, Data, Font, FontMgr, FontStyle, Image, ImageInfo, Paint, RRect, Rect,
    SamplingOptions, canvas::SrcRectConstraint, image_filters, images, jpeg_encoder, surfaces,
};

/// One video in the picture, as it shows on screen.
pub struct Shot {
    pub frame: Pixels,
    /// Clockwise degrees, a multiple of 90
    pub rotation: i32,
    /// Your own camera, flipped like a mirror as the screen shows it
    pub mirrored: bool,
}

pub struct Picture {
    pub main: Shot,
    /// Your own camera, in the bottom-right corner where its tile sits
    pub tile: Option<Shot>,
    /// "4:05", how long the call has run
    pub elapsed: String,
    pub lang: Lang,
}

/// Draws and saves; returns where it went: a file path, or on Android the gallery's URI. Slow enough (encoding a
/// JPEG) to keep off the UI thread.
pub fn save(picture: &Picture) -> Result<String, String> {
    let jpeg = draw(picture).ok_or("couldn't draw the picture")?;
    store(&Local::now().format("call-%Y%m%d-%H%M").to_string(), jpeg.as_bytes())
}

#[cfg(not(target_os = "android"))]
fn store(stem: &str, jpeg: &[u8]) -> Result<String, String> {
    let dir = pictures_dir().ok_or("no Pictures folder")?.join("Call Danya");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = free_name(&dir, stem);
    std::fs::write(&path, jpeg).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// The gallery numbers a name that's taken by itself.
#[cfg(target_os = "android")]
fn store(stem: &str, jpeg: &[u8]) -> Result<String, String> {
    crate::android::save_picture(stem, jpeg).ok_or_else(|| "the gallery didn't take it".to_owned())
}

/// Opens the picture in the phone's photo viewer.
#[cfg(target_os = "android")]
pub fn show(saved: &str) {
    crate::android::open_picture(saved);
}

/// Shows the picture selected in Finder or Explorer.
#[cfg(not(target_os = "android"))]
pub fn show(saved: &str) {
    let path = Path::new(saved);
    let opened = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(path).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn()
    };
    if let Err(e) = opened {
        tracing::warn!("couldn't show the picture: {e}");
    }
}

#[cfg(not(target_os = "android"))]
fn pictures_dir() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(target_os = "windows") { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join("Pictures"))
}

/// call-20261107-1554.jpg, then -2, -3… when two land in the same minute.
#[cfg(not(target_os = "android"))]
fn free_name(dir: &Path, stem: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.jpg"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.jpg"));
        n += 1;
    }
    path
}

fn image(frame: &Pixels) -> Option<Image> {
    let frame = frame.to_buffer()?;
    let (w, h) = (i32::try_from(frame.width()).ok()?, i32::try_from(frame.height()).ok()?);
    let info = ImageInfo::new((w, h), ColorType::RGBA8888, AlphaType::Unpremul, None);
    images::raster_from_data(&info, Data::new_copy(frame.as_bytes()), frame.width() as usize * 4)
}

fn draw(picture: &Picture) -> Option<Data> {
    let main = image(&picture.main.frame)?;
    let turned = picture.main.rotation % 180 != 0;
    let (w, h) = if turned { (main.height(), main.width()) } else { (main.width(), main.height()) };
    let mut surface = surfaces::raster_n32_premul((w, h))?;
    let canvas = surface.canvas();
    let (wf, hf) = (w as f32, h as f32);
    let short = wf.min(hf);
    let margin = (short * 0.03).round();
    draw_cover(canvas, &main, &picture.main, Rect::from_xywh(0.0, 0.0, wf, hf));

    if let Some(tile) = &picture.tile
        && let Some(small) = image(&tile.frame)
    {
        let long = short * 0.3;
        let aspect = small.width() as f32 / small.height() as f32;
        let (tw, th) = if aspect >= 1.0 { (long, long / aspect) } else { (long * aspect, long) };
        let rect = Rect::from_xywh(wf - tw - margin, hf - th - margin, tw, th);
        canvas.save();
        canvas.clip_rrect(RRect::new_rect_xy(rect, tw.min(th) * 0.08, tw.min(th) * 0.08), None, true);
        draw_cover(canvas, &small, tile, rect);
        canvas.restore();
    }

    let stamp = [date(picture.lang), Local::now().format("%H:%M").to_string(), picture.elapsed.clone()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let size = (short * 0.03).round().max(14.0);
    let font = Font::new(typeface(picture.lang)?, size);
    let mut paint = Paint::new(Color4f::from(Color::from_argb(217, 255, 255, 255)), None);
    paint.set_anti_alias(true);
    let blur = (short * 0.01).round().max(1.0) / 2.0;
    paint.set_image_filter(image_filters::drop_shadow(
        (0.0, 0.0),
        (blur, blur),
        Color4f::from(Color::from_argb(153, 0, 0, 0)),
        None,
        None,
        None,
    ));
    // Bottom left, where no tile sits
    canvas.draw_str(&stamp, (margin, hf - margin), &font, &paint);

    let options = jpeg_encoder::Options { quality: 92, ..Default::default() };
    jpeg_encoder::encode_image(None, &surface.image_snapshot(), &options)
}

/// Fills the box with the image, cropping what overflows, turned and mirrored as the screen shows it.
fn draw_cover(canvas: &skia_safe::Canvas, image: &Image, shot: &Shot, rect: Rect) {
    let rotation = shot.rotation;
    let (iw, ih) = (image.width() as f32, image.height() as f32);
    // The box as the unturned image sees it
    let (bw, bh) = if rotation % 180 != 0 { (rect.height(), rect.width()) } else { (rect.width(), rect.height()) };
    let scale = (bw / iw).max(bh / ih);
    let (sw, sh) = (bw / scale, bh / scale);
    let src = Rect::from_xywh((iw - sw) / 2.0, (ih - sh) / 2.0, sw, sh);
    canvas.save();
    canvas.translate((rect.center_x(), rect.center_y()));
    // Before the turn, so it flips the upright picture
    if shot.mirrored {
        canvas.scale((-1.0, 1.0));
    }
    canvas.rotate(rotation as f32, None);
    let dst = Rect::from_xywh(-bw / 2.0, -bh / 2.0, bw, bh);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    canvas.draw_image_rect_with_sampling_options(
        image,
        Some((&src, SrcRectConstraint::Fast)),
        dst,
        SamplingOptions::from(skia_safe::CubicResampler::mitchell()),
        &paint,
    );
    canvas.restore();
}

/// "7 Nov 2026", or "2026年11月7日" in Chinese.
fn date(lang: Lang) -> String {
    let now = Local::now();
    match lang {
        Lang::Zh => format!("{}年{}月{}日", now.year(), now.month(), now.day()),
        Lang::En => now.format("%-d %b %Y").to_string(),
    }
}

/// A system font that has the stamp's characters: the Chinese date needs a CJK font.
fn typeface(lang: Lang) -> Option<skia_safe::Typeface> {
    let fonts = FontMgr::new();
    let families: &[&str] = match lang {
        Lang::Zh => &["PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC"],
        Lang::En => &["Helvetica Neue", "Segoe UI", "Arial"],
    };
    families
        .iter()
        .find_map(|family| fonts.match_family_style(family, FontStyle::bold()))
        .or_else(|| fonts.legacy_make_typeface(None, FontStyle::bold()))
}
