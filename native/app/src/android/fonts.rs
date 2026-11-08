//! A Chinese font for Slint. Its font library finds Android's fallback fonts only through the `postScriptName` that
//! AOSP's fonts.xml puts on each font, and many phone makers list their own CJK fonts without one: every Chinese
//! character then draws as a box. So the font Android itself uses for zh-Hans is looked up here, and handed to Slint
//! behind the UI font.

use std::path::{Path, PathBuf};

const FONTS_XML: &str = "/system/etc/fonts.xml";
const FONT_DIRS: [&str; 3] = ["/system/fonts", "/product/fonts", "/system/product/fonts"];
/// When fonts.xml names no zh-Hans font: what phones have shipped under their own names
const KNOWN_CJK: [&str; 7] = [
    "NotoSansCJK-Regular.ttc",
    "NotoSansSC-Regular.otf",
    "HarmonyOS_Sans_SC_Regular.ttf",
    "MiSansVF.ttf",
    "DroidSansChinese.ttf",
    "DroidSansFallbackFull.ttf",
    "DroidSansFallback.ttf",
];

/// Sets `SLINT_DEFAULT_FONT` and `SLINT_FONT_PATH` when Slint wouldn't find a Chinese font on its own. Must run
/// before Slint lays out any text.
pub fn init() {
    let xml = std::fs::read_to_string(FONTS_XML).unwrap_or_default();
    let doc = roxmltree::Document::parse(&xml).ok();
    let families = doc.iter().flat_map(|doc| doc.root_element().children()).filter(|n| n.has_tag_name("family"));
    let mut ui = None;
    let mut cjk = None;
    for family in families {
        // Older fonts.xml list Roboto's weights from Thin up
        let mut fonts = family.children().filter(|n| n.has_tag_name("font"));
        let regular = fonts.clone().find(|n| n.attribute("weight").is_none_or(|w| w == "400"));
        let Some(font) = regular.or_else(|| fonts.next()) else { continue };
        let Some(file) = font.text().map(str::trim).and_then(find) else { continue };
        if ui.is_none() && family.attribute("name") == Some("sans-serif") {
            ui = Some(file);
        } else if cjk.is_none() && family.attribute("lang").is_some_and(|l| l.split(',').any(|l| l == "zh-Hans")) {
            // What Slint can already find needs no help; anything else would also change which face of a
            // collection draws Chinese
            if font.attribute("postScriptName").is_some() && file.starts_with(FONT_DIRS[0]) {
                return;
            }
            cjk = Some(file);
        }
    }
    let Some(cjk) = cjk.or_else(|| KNOWN_CJK.iter().find_map(|name| find(name))) else {
        tracing::warn!("no Chinese font found");
        return;
    };
    tracing::info!("Chinese font: {}", cjk.display());
    // SAFETY: nothing else reads or writes the environment yet: this runs first in `android_main`, before Slint
    // and the engine start their threads.
    unsafe {
        // The font path replaces Slint's sans-serif, so the UI font goes first and Chinese falls back to the next
        if let Some(ui) = ui {
            std::env::set_var("SLINT_DEFAULT_FONT", ui);
        }
        std::env::set_var("SLINT_FONT_PATH", cjk);
    }
}

fn find(name: &str) -> Option<PathBuf> {
    FONT_DIRS.iter().map(|dir| Path::new(dir).join(name)).find(|path| path.is_file())
}
