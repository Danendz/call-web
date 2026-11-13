//! Reactions flying over the video, kept on the UI thread like the web's Reactions.tsx: each one adds a floater that
//! a timer removes once it has flown, and every fifth of the same in a row plays that reaction's full-screen effect.

use std::cell::Cell;
use std::f32::consts::TAU;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use engine::Reaction;
use slint::{Color, Model, ModelRc, SharedString, Timer, VecModel};

use crate::ui::{AppWindow, Floater, Spark};

/// Taps of the same reaction closer than this make a combo. Each side counts the other's from when they arrive, so
/// the gap leaves room for network jitter. The same as the web's combo.ts.
const COMBO_GAP: Duration = Duration::from_millis(800);
/// Every this many in a row, a reaction that has a big effect plays it.
const BIG_EFFECT_EVERY: u32 = 5;
/// How long one flies, as `FloatingEmoji` draws it
const FLIGHT: Duration = Duration::from_millis(3000);
/// More than this on screen at once and the oldest go first
const MAX_FLOATERS: usize = 30;
/// Long enough for the last spark of any effect to be gone
const EFFECT: Duration = Duration::from_millis(3800);

const CONFETTI: [u32; 6] = [0xff5d73, 0xffc94d, 0x5dd39e, 0x5aa9ff, 0xb57bff, 0xff9a4d];

#[derive(Debug, Clone, Copy, PartialEq)]
struct Combo {
    emoji: Reaction,
    count: u32,
    at: Instant,
}

/// One side's run of the same reaction, after another one at `at`.
fn next_combo(prev: Option<Combo>, emoji: Reaction, at: Instant) -> Combo {
    let continues = prev.filter(|p| p.emoji == emoji && at.saturating_duration_since(p.at) <= COMBO_GAP);
    Combo { emoji, count: continues.map_or(1, |p| p.count + 1), at }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Effect {
    Hearts,
    Confetti,
    Flames,
}

fn big_effect(emoji: Reaction) -> Option<Effect> {
    match emoji {
        Reaction::Heart => Some(Effect::Hearts),
        Reaction::Party => Some(Effect::Confetti),
        Reaction::Fire => Some(Effect::Flames),
        _ => None,
    }
}

pub struct Reactions {
    floaters: Rc<VecModel<Floater>>,
    sparks: Rc<VecModel<Spark>>,
    mine: Option<Combo>,
    theirs: Option<Combo>,
    next_id: i32,
    effect_playing: Rc<Cell<bool>>,
    rng: Rng,
}

impl Reactions {
    pub fn new(ui: &AppWindow) -> Self {
        let floaters = Rc::new(VecModel::default());
        let sparks = Rc::new(VecModel::default());
        ui.set_floaters(ModelRc::from(floaters.clone()));
        ui.set_sparks(ModelRc::from(sparks.clone()));
        Self {
            floaters,
            sparks,
            mine: None,
            theirs: None,
            next_id: 0,
            effect_playing: Rc::default(),
            rng: Rng::seeded(),
        }
    }

    pub fn add(&mut self, emoji: Reaction, mine: bool) {
        let side = if mine { &mut self.mine } else { &mut self.theirs };
        let combo = next_combo(*side, emoji, Instant::now());
        *side = Some(combo);

        if self.floaters.row_count() >= MAX_FLOATERS {
            self.floaters.remove(0);
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        // Yours rise on the right, theirs on the left, so a back-and-forth reads as one
        let x = if mine { 0.58 } else { 0.12 } + self.rng.range(0.0, 0.3);
        self.floaters.push(Floater {
            id,
            emoji: emoji.emoji().into(),
            x,
            drift: self.rng.range(10.0, 26.0),
            count: i32::try_from(combo.count).unwrap_or(i32::MAX),
        });
        let floaters = Rc::downgrade(&self.floaters);
        Timer::single_shot(FLIGHT + Duration::from_millis(100), move || {
            let Some(floaters) = floaters.upgrade() else { return };
            if let Some(index) = floaters.iter().position(|f| f.id == id) {
                floaters.remove(index);
            }
        });

        #[cfg(target_os = "android")]
        if !mine {
            crate::android::vibrate(25);
        }

        if let Some(effect) = big_effect(emoji).filter(|_| combo.count.is_multiple_of(BIG_EFFECT_EVERY)) {
            self.play(effect);
        }
    }

    /// One effect at a time: another combo meanwhile just flies
    fn play(&mut self, effect: Effect) {
        if self.effect_playing.get() {
            return;
        }
        self.effect_playing.set(true);
        let sparks = self.spawn(effect);
        self.sparks.set_vec(sparks);
        let (sparks, playing) = (Rc::downgrade(&self.sparks), self.effect_playing.clone());
        Timer::single_shot(EFFECT, move || {
            if let Some(sparks) = sparks.upgrade() {
                sparks.clear();
            }
            playing.set(false);
        });
    }

    fn spawn(&mut self, effect: Effect) -> Vec<Spark> {
        let r = &mut self.rng;
        let ms = |s: f32| i64::from((s * 1000.0) as i32);
        match effect {
            Effect::Hearts => (0..46)
                .map(|_| Spark {
                    glyph: "❤️".into(),
                    color: Color::default(),
                    x: r.range(0.0, 1.0),
                    delay: ms(r.range(0.0, 1.1)),
                    life: ms(r.range(2.0, 2.8)),
                    speed: r.range(0.3, 0.6),
                    size: r.range(22.0, 52.0),
                    spin: r.range(-0.6, 0.6),
                    phase: r.range(0.0, TAU),
                    burns: false,
                })
                .collect(),
            Effect::Confetti => (0..150)
                .map(|i| Spark {
                    glyph: SharedString::new(),
                    color: Color::from_argb_encoded(
                        0xff00_0000 | CONFETTI.get(i % CONFETTI.len()).copied().unwrap_or(0xffffff),
                    ),
                    x: r.range(0.0, 1.0),
                    delay: ms(r.range(0.0, 0.7)),
                    life: ms(r.range(2.2, 3.0)),
                    speed: r.range(0.08, 0.3),
                    size: r.range(6.0, 11.0),
                    spin: r.range(-9.0, 9.0),
                    phase: r.range(0.0, TAU),
                    burns: false,
                })
                .collect(),
            Effect::Flames => (0..60)
                .map(|_| Spark {
                    glyph: "🔥".into(),
                    color: Color::default(),
                    x: r.range(-0.05, 1.05),
                    delay: ms(r.range(0.0, 1.2)),
                    life: ms(r.range(1.0, 1.7)),
                    speed: r.range(0.25, 0.5),
                    size: r.range(30.0, 70.0),
                    spin: 0.0,
                    phase: r.range(0.0, TAU),
                    burns: true,
                })
                .collect(),
        }
    }
}

/// Where things fly needn't be unpredictable, just not in step: xorshift, seeded from the clock.
struct Rng(u64);

impl Rng {
    fn seeded() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.subsec_nanos());
        Self(u64::from(nanos) | 1)
    }

    /// In `min..max`
    fn range(&mut self, min: f32, max: f32) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        min + (self.0 >> 40) as f32 / (1u64 << 24) as f32 * (max - min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_reaction_in_quick_succession_counts_up() {
        let t = Instant::now();
        let a = next_combo(None, Reaction::Heart, t);
        let b = next_combo(Some(a), Reaction::Heart, t + Duration::from_millis(500));
        let c = next_combo(Some(b), Reaction::Heart, t + Duration::from_millis(1200));
        assert_eq!((a.count, b.count, c.count), (1, 2, 3));
    }

    #[test]
    fn a_pause_or_another_reaction_starts_over() {
        let t = Instant::now();
        let a = next_combo(None, Reaction::Heart, t);
        let b = next_combo(Some(a), Reaction::Fire, t + Duration::from_millis(100));
        assert_eq!(b.count, 1);
        let c =
            next_combo(Some(b), Reaction::Fire, t + Duration::from_millis(100) + COMBO_GAP + Duration::from_millis(1));
        assert_eq!(c.count, 1);
    }
}
