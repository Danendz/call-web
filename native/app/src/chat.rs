//! The chat over the video, kept on the UI thread, like the web's useFeed: every message and reaction, then what the
//! other person is saying right now as it comes. A finished spoken line takes the place of its live text. Their
//! spoken lines in the other language are translated on this device as they come (`translate.rs`), then by the server
//! once finished. A run of the same reaction from one side grows in place (❤️ ×5), like the web's useReactionRuns;
//! reactions aren't stored, so they're gone with the next reset.
//!
//! Messages come in as [`ChatEvent`]s and update the list in place: a new line is inserted before the live row, a
//! translation rewrites one, so each row keeps its own crossfade. What a row says depends on your language, so a
//! language change rewrites them all.

use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use engine::{ChatEvent, ChatMessage, Lang, Pair, Reaction, TranslatorEvent};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::reactions::{Combo, next_combo};
use crate::readable::{has_other_script, readable};
use crate::translate::LiveTranslation;
use crate::ui::{AppWindow, ChatRow};

// Only the latest spoken lines are translated on this device: what's in view as they come
const QUICK: usize = 8;
// The chat keeps this many reaction bubbles; older ones go
const MAX_RUNS: usize = 50;

/// What a row shows: a message by its index, or a run of reactions by its id
#[derive(Debug, Clone, Copy, PartialEq)]
enum Line {
    Message(usize),
    Reaction(u64),
}

struct Run {
    id: u64,
    emoji: Reaction,
    mine: bool,
    count: u32,
    /// How many messages there were when it came: a message since ends it
    after: usize,
}

/// One side's reactions: its combo, and the run it's growing
#[derive(Default)]
struct Side {
    combo: Option<Combo>,
    run: Option<u64>,
}

struct Live {
    id: String,
    text: String,
    /// The last translation made here, kept until the next is in, so growing text doesn't flash the original
    quick: Option<String>,
}

pub struct Chat {
    /// One row per line, then the live row while there's live text
    rows: Rc<VecModel<ChatRow>>,
    lines: Vec<Line>,
    messages: Vec<ChatMessage>,
    runs: Vec<Run>,
    next_run: u64,
    mine: Side,
    theirs: Side,
    /// By message: the last translation made here
    quick: Vec<Option<String>>,
    live: Option<Live>,
    /// "Live subtitles" in Settings: off, there's no live row and nothing is translated here
    live_on: bool,
    my_id: String,
    lang: Lang,
    translation: LiveTranslation,
}

impl Chat {
    pub fn new(ui: &AppWindow, lang: Lang) -> Self {
        let rows = Rc::new(VecModel::default());
        ui.set_chat(ModelRc::from(rows.clone()));
        Self {
            rows,
            lines: Vec::new(),
            messages: Vec::new(),
            runs: Vec::new(),
            next_run: 0,
            mine: Side::default(),
            theirs: Side::default(),
            quick: Vec::new(),
            live: None,
            live_on: true,
            my_id: String::new(),
            lang,
            translation: LiveTranslation::default(),
        }
    }

    pub fn event(&mut self, ui: &AppWindow, event: ChatEvent) {
        match event {
            ChatEvent::Reset { my_id, messages } => {
                self.my_id = my_id;
                self.quick = vec![None; messages.len()];
                self.messages = messages;
                self.live = None;
                self.runs.clear();
                self.mine = Side::default();
                self.theirs = Side::default();
                self.lines = (0..self.messages.len()).map(Line::Message).collect();
                let rows = (0..self.messages.len()).filter_map(|i| self.message_row(i)).collect::<Vec<_>>();
                self.rows.set_vec(rows);
            }
            ChatEvent::Added(message) => {
                // The line finishing the live text takes its row, and the translation made here so far
                let finishes = self.live.as_ref().is_some_and(|l| message.live_id.as_ref() == Some(&l.id));
                let quick = if finishes { self.live.take().and_then(|l| l.quick) } else { None };
                self.messages.push(message);
                self.quick.push(quick);
                let index = self.messages.len() - 1;
                // In the live row's place, which a finished line takes over
                let at = self.lines.len();
                self.lines.push(Line::Message(index));
                if let Some(row) = self.message_row(index) {
                    if finishes { self.rows.set_row_data(at, row) } else { self.rows.insert(at, row) }
                }
            }
            ChatEvent::Live { id, text } => {
                self.live_text(id, text);
                return;
            }
            ChatEvent::Reaction { emoji, mine } => {
                self.reaction(emoji, mine);
                return;
            }
            ChatEvent::Translated { id, translation } => {
                let Some(index) = self.messages.iter().position(|m| m.id == id) else { return };
                let Some(message) = self.messages.get_mut(index) else { return };
                message.translation = Some(translation);
                if let Some(at) = self.position(Line::Message(index)) {
                    self.update(at);
                }
            }
        }
        let others = self.messages.iter().filter(|m| m.from != self.my_id).count();
        ui.set_others_count(i32::try_from(others).unwrap_or(i32::MAX));
    }

    /// Grows the side's run when it's the same reaction again in quick succession, with no message since
    fn reaction(&mut self, emoji: Reaction, mine: bool) {
        let after = self.messages.len();
        let side = if mine { &mut self.mine } else { &mut self.theirs };
        let combo = next_combo(side.combo, emoji, Instant::now());
        side.combo = Some(combo);
        let growing = side.run.filter(|_| combo.count > 1);
        if let Some(run) = growing.and_then(|id| self.runs.iter_mut().find(|r| r.id == id && r.after == after)) {
            run.count = combo.count;
            let id = run.id;
            if let Some(at) = self.position(Line::Reaction(id)) {
                self.update(at);
            }
            return;
        }

        if self.runs.len() >= MAX_RUNS {
            let oldest = self.runs.remove(0);
            if let Some(at) = self.position(Line::Reaction(oldest.id)) {
                self.lines.remove(at);
                self.rows.remove(at);
            }
        }
        let id = self.next_run;
        self.next_run += 1;
        let side = if mine { &mut self.mine } else { &mut self.theirs };
        side.run = Some(id);
        self.runs.push(Run { id, emoji, mine, count: 1, after });
        let at = self.lines.len();
        self.lines.push(Line::Reaction(id));
        if let Some(row) = self.reaction_row(id) {
            self.rows.insert(at, row);
        }
    }

    fn position(&self, line: Line) -> Option<usize> {
        self.lines.iter().position(|&l| l == line)
    }

    fn live_text(&mut self, id: String, text: String) {
        let index = self.lines.len();
        // Ids come from the other side and may repeat, so only the live text being shown is matched
        let same = self.live.as_ref().is_some_and(|l| l.id == id);
        if text.is_empty() || !self.live_on {
            if same || (!self.live_on && self.live.is_some()) {
                self.drop_live();
            }
            return;
        }
        match &mut self.live {
            Some(live) => {
                if !same {
                    live.quick = None;
                }
                live.id = id;
                live.text = text;
                self.update(index);
            }
            None => {
                self.live = Some(Live { id, text, quick: None });
                if let Some(row) = self.live_row() {
                    self.rows.push(row);
                }
            }
        }
    }

    fn drop_live(&mut self) {
        if self.live.take().is_some() && self.rows.row_count() > self.lines.len() {
            self.rows.remove(self.lines.len());
        }
    }

    pub fn set_live_on(&mut self, on: bool) {
        self.live_on = on;
        if !on {
            self.drop_live();
        }
    }

    /// Starts or stops translating here, for `want`'s direction
    pub fn sync_translation(
        &mut self,
        want: Option<Pair>,
        data_dir: Option<&Path>,
        on_event: impl Fn(Pair, TranslatorEvent) + Send + Clone + 'static,
    ) {
        self.translation.sync(want, data_dir, on_event);
    }

    pub fn translation_event(&mut self, pair: Pair, event: TranslatorEvent) {
        if self.translation.event(pair, event) {
            self.refresh();
        }
    }

    pub fn set_lang(&mut self, lang: Lang) {
        if lang == self.lang {
            return;
        }
        // Translations into the language before don't count
        self.lang = lang;
        self.quick.iter_mut().for_each(|q| *q = None);
        if let Some(live) = &mut self.live {
            live.quick = None;
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        for at in 0..=self.lines.len() {
            self.update(at);
        }
    }

    /// Rewrites the row at `at`, the live one past the lines, if what it says changed
    fn update(&mut self, at: usize) {
        let row = match self.lines.get(at) {
            Some(&Line::Message(index)) => self.message_row(index),
            Some(&Line::Reaction(id)) => self.reaction_row(id),
            None => self.live_row(),
        };
        let Some(row) = row else { return };
        if at < self.rows.row_count() && self.rows.row_data(at).as_ref() != Some(&row) {
            self.rows.set_row_data(at, row);
        }
    }

    fn reaction_row(&self, id: u64) -> Option<ChatRow> {
        let run = self.runs.iter().find(|r| r.id == id)?;
        let text = if run.count > 1 {
            format!("{} ×{}", run.emoji.emoji(), run.count)
        } else {
            run.emoji.emoji().to_string()
        };
        Some(ChatRow {
            first: SharedString::from(&text),
            text: text.into(),
            mine: run.mine,
            reaction: true,
            ..ChatRow::default()
        })
    }

    /// Their spoken line in the other language, which may be translated here
    fn foreign(&self, m: &ChatMessage) -> bool {
        m.spoken == Some(true) && m.from != self.my_id && has_other_script(&m.text, self.lang)
    }

    fn message_row(&mut self, index: usize) -> Option<ChatRow> {
        let m = self.messages.get(index)?;
        let (text, original) = readable(m, &self.my_id, self.lang);
        let (text, original, source) = (text.to_string(), original.map(str::to_string), m.text.clone());
        let mine = m.from == self.my_id;
        let spoken = m.spoken == Some(true);
        let foreign = self.foreign(m);
        let recent = foreign && self.messages.get(index..)?.iter().filter(|m| self.foreign(m)).count() <= QUICK;
        let found = if recent && self.live_on { self.translation.lookup(&source) } else { None };
        let quick = self.quick.get_mut(index)?;
        if found.is_some() {
            *quick = found;
        }
        let kept = quick.clone();
        // One still waiting for its translation here waits the few ms it takes, rather than flash the original
        let pending = recent && self.live_on && self.translation.ready() && kept.is_none();
        let first = kept.unwrap_or_else(|| text.clone());
        Some(ChatRow {
            translated: text != source && text != first,
            first: first.into(),
            text: text.into(),
            original: original.unwrap_or_default().into(),
            mine,
            spoken,
            live: false,
            pending,
            reaction: false,
        })
    }

    fn live_row(&mut self) -> Option<ChatRow> {
        let live = self.live.as_mut()?;
        let foreign = has_other_script(&live.text, self.lang);
        if foreign && let Some(found) = self.translation.lookup(&live.text) {
            live.quick = Some(found);
        }
        let pending = foreign && self.translation.ready() && live.quick.is_none();
        let first = live.quick.clone().unwrap_or_else(|| live.text.clone());
        Some(ChatRow {
            first: SharedString::from(&first),
            text: first.into(),
            translated: false,
            original: SharedString::default(),
            mine: false,
            spoken: true,
            live: true,
            pending,
            reaction: false,
        })
    }
}
