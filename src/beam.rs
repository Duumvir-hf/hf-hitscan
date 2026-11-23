//! Phase 3 — the beam. [`damage::try_fire`](crate::damage) already knows the whole geometry of a
//! shot, so it hands this module **one line**: muzzle → where the beam stopped. The render pass then
//! draws the live lines for their lifetime.
//!
//! ```text
//! Gfx_DrawWorldLine 0x140248ba0   the game's own line primitive (Hud_DrawAimRay uses it)
//! HGE               0x143a119e0   the interface object; +0xd68 = application seconds (Timer_GetTime)
//! ```
//!
//! # Two lines per beam, and a fade is a colour
//!
//! `Gfx_DrawWorldLine` takes an **ARGB** colour and the game encodes alpha in the top byte, so the
//! fade is a different colour value, not another argument. A glowing beam is drawn twice: the glow
//! first (`width × 3`, `visual.glow_color`), then the main line (`visual.width`, `visual.color`).
//! `linear` multiplies the configured alpha by `1 − age/seconds`; `none` keeps it constant.
//!
//! # The clock is the game's, and a reset is not a ghost
//!
//! The lifetime is measured on the `HGE` application clock, which freezes while the game is paused
//! and restarts with a new battle (hazard H11). A beam whose age is **negative** is from before a
//! clock reset and is dropped rather than drawn for the rest of the session; wall-clock time is
//! never used, because it would keep running while paused.
//!
//! # Where this runs
//!
//! Only from the draw tick ([`draw_tick`], hooked at `BattleHud_DrawCombatMarkers`'s entry — the
//! battle HUD's marker renderer, which also draws the enemy's pre-fire laser), and from [`spawn`] on
//! the fire path. The tick's first act is "is the ring empty?", so a frame with no live beam returns
//! after that check: no body walk, no clock read, no draw call. Beams are drawn only for shots the mod
//! handled: a shot that was forwarded to vanilla `FireShell` never reaches [`spawn`].
//!
//! # Colour, width, lifetime (config)
//!
//! `visual.color`, `width`, `glow`, `glow_color`, `seconds` and `fade` are already parsed and
//! validated by [`crate::config`]; phase 3 adds no key. `width` is in the primitive's own units (the
//! game's aim line is `1.0`), and `seconds == 0` draws nothing.

use std::sync::{Mutex, MutexGuard};

use crate::{
    config::{Fade, ResolvedType},
    mem, read, readable, Vec2,
};

/// The `HGE` interface object — a **pointer** stored here, not the object.
const HGE: usize = 0x1_43a1_19e0;
const _: () = assert!(HGE == 0x143a119e0);
/// `Timer_GetTime` (`0x14017ef00`): `MOVSS XMM0, [RCX + 0xd68]; RET` — application seconds.
const HGE_TIME: usize = 0xd68;

/// `Gfx_DrawWorldLine` (1.163). `Hud_DrawAimRay` calls it as
/// `(from, to, width, colour, 1, 0, 0.0, 1, 1.0)` with `alreadyScreen = 0`, i.e. world coordinates;
/// the enemy pre-fire laser in `BattleHud_DrawCombatMarkers` passes `param_5 = 3`, `argb2 = 0` and a
/// per-call second colour instead (a gradient). This mod copies the aim-line form (solid line).
const DRAW_WORLD_LINE: usize = 0x1_4024_8ba0;
const _: () = assert!(DRAW_WORLD_LINE == 0x140248ba0);

/// `undefined Gfx_DrawWorldLine(Vec2 from, Vec2 to, float width, uint argb, undefined4 param_5,
/// char alreadyScreen, float cornerField8, uint argb2, float param_9)`.
///
/// The ABI was read out of the 1.163 program on 2026-11-23, not assumed: at the entry the two
/// `Vec2`s arrive in `RDX`/`RCX` (the body stores `RDX` then `RCX` into `[RSP+0x20]`/`[RSP+0x28]`),
/// the width in `XMM2` (`MOVAPS XMM14, XMM2`), the colour in `R9D` (`MOV EBX, R9D`), and the five
/// trailing arguments on the stack — `[RSP+0x28]` `param_5`, `[RSP+0x30]` `alreadyScreen`,
/// `[RSP+0x38]` `cornerField8`, `[RSP+0x40]` `argb2`, `[RSP+0x48]` `param_9` — exactly what an
/// `extern "C"` declaration produces on x86_64-windows.
type FnDrawWorldLine = unsafe extern "C" fn(Vec2, Vec2, f32, u32, u32, u8, f32, u32, f32);

/// How many beams can be live at once. A ring: the oldest is dropped when it is full.
pub(crate) const CAPACITY: usize = 32;

/// One live beam, as handed over by the damage chain.
#[derive(Clone, Copy, Debug)]
struct Beam {
    from: Vec2,
    to: Vec2,
    color: u32,
    glow: bool,
    glow_color: u32,
    width: f32,
    seconds: f32,
    fade: Fade,
    /// The application clock when the shot was fired.
    born: f32,
}

/// The live beams. Shared with the render pass, which may not be the game thread.
static BEAMS: Mutex<Vec<Beam>> = Mutex::new(Vec::new());

fn beams() -> MutexGuard<'static, Vec<Beam>> {
    // A panic while the lock is held must not disable the beam; the data is a plain list.
    BEAMS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The game's application clock in seconds, or `None` while the `HGE` object is not usable.
///
/// Pausing freezes it; a battle restart resets it (hazard H11). There is deliberately no
/// wall-clock fallback: a beam must not age while the game is paused.
pub(crate) fn clock() -> Option<f32> {
    let hge = mem::usize_at(HGE)?;
    if hge < 0x1_0000 || !readable(hge + HGE_TIME, std::mem::size_of::<f32>()) {
        return None;
    }
    // SAFETY: the four bytes at `hge + 0xd68` were just proved readable.
    let time = unsafe { read::<f32>(hge + HGE_TIME) };
    (time.is_finite() && time >= 0.0).then_some(time)
}

/// Record one beam for the draw tick.
///
/// Called by [`crate::damage::try_fire`] once per shot it handled — never for a shot that was
/// forwarded to vanilla `FireShell`.
pub(crate) fn spawn(laser: &ResolvedType, from: Vec2, to: Vec2) {
    if !(laser.seconds > 0.0) {
        // A zero lifetime would be dropped by the first tick; do not queue it at all.
        return;
    }
    let Some(born) = clock() else {
        log::debug!("hf_hitscan: no application clock; the beam is not drawn");
        return;
    };
    let mut beams = beams();
    if beams.len() >= CAPACITY {
        beams.remove(0);
    }
    beams.push(Beam {
        from,
        to,
        color: laser.color,
        glow: laser.glow,
        glow_color: laser.glow_color,
        width: laser.width,
        seconds: laser.seconds,
        fade: laser.fade,
        born,
    });
}

/// True while a beam of `seconds` born `age` seconds ago is still drawn.
///
/// A negative age means the clock was reset (a new battle) since the shot: drop it rather than
/// leaving a ghost beam in the world for the rest of the session (H11).
fn visible(age: f32, seconds: f32) -> bool {
    age >= 0.0 && age < seconds
}

/// The alpha factor at `age`: `linear` fades `1 → 0` over the lifetime, `none` stays at `1`.
fn fade_at(fade: Fade, age: f32, seconds: f32) -> f32 {
    match fade {
        Fade::Linear => 1.0 - age / seconds,
        Fade::None => 1.0,
    }
}

/// `color` with its configured alpha scaled by `fade` — the game's `(int)(alpha * 255) << 24 | rgb`.
fn with_fade(color: u32, fade: f32) -> u32 {
    let alpha = (color >> 24) as f32 * fade;
    // `as u32` saturates, so a negative or NaN factor is 0 and the top byte is never clobbered.
    ((alpha as u32).min(255) << 24) | (color & 0x00FF_FFFF)
}

/// The per-frame draw tick: drop the expired beams, then draw the live ones.
///
/// Runs inside the HUD's draw pass, so a beam is submitted where the game submits its own world
/// lines. An empty ring returns right after the empty check — the tick adds nothing to an idle frame.
pub(crate) fn draw_tick() {
    let mut beams = beams();
    if beams.is_empty() {
        return;
    }
    let Some(now) = clock() else {
        return;
    };
    beams.retain(|beam| visible(now - beam.born, beam.seconds));
    for beam in beams.iter() {
        let age = now - beam.born;
        let fade = fade_at(beam.fade, age, beam.seconds);
        if beam.glow {
            draw_line(
                beam.from,
                beam.to,
                beam.width * 3.0,
                with_fade(beam.glow_color, fade),
            );
        }
        draw_line(beam.from, beam.to, beam.width, with_fade(beam.color, fade));
    }
}

/// One `Gfx_DrawWorldLine` call with the argument values `Hud_DrawAimRay` uses (the aim-line form).
///
/// Must only be called from the render pass: it draws a quad through HGE's renderer (vtable `+0x2e0`,
/// `Gfx_RenderQuad`).
fn draw_line(from: Vec2, to: Vec2, width: f32, argb: u32) {
    // SAFETY: `DRAW_WORLD_LINE` is the 1.163 `Gfx_DrawWorldLine` (verified in the program), the
    // declaration above matches its ABI, and the two points are world coordinates
    // (`alreadyScreen = 0`).
    let draw: FnDrawWorldLine = unsafe { std::mem::transmute(DRAW_WORLD_LINE) };
    unsafe { draw(from, to, width, argb, 1, 0, 0.0, 1, 1.0) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_addresses_are_the_ones_read_from_the_1_163_program() {
        assert_eq!(HGE, 0x143a119e0);
        assert_eq!(HGE_TIME, 0xd68);
        assert_eq!(DRAW_WORLD_LINE, 0x140248ba0);
    }

    #[test]
    fn a_beam_is_visible_from_its_birth_until_its_lifetime_ends() {
        assert!(visible(0.0, 0.08));
        assert!(visible(0.079, 0.08));
        assert!(!visible(0.08, 0.08), "the lifetime is exclusive");
        assert!(!visible(-0.001, 0.08), "a clock reset is not a ghost beam (H11)");
        assert!(!visible(0.0, 0.0), "seconds 0 draws nothing");
    }

    #[test]
    fn the_fade_is_linear_or_constant() {
        assert!((fade_at(Fade::Linear, 0.0, 0.08) - 1.0).abs() < f32::EPSILON);
        assert!((fade_at(Fade::Linear, 0.04, 0.08) - 0.5).abs() < f32::EPSILON);
        assert!((fade_at(Fade::None, 0.04, 0.08) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_fade_scales_the_configured_alpha_and_leaves_the_rgb_alone() {
        // "#FF3322" at full life is exactly the configured ARGB.
        assert_eq!(with_fade(0xFFFF_3322, 1.0), 0xFFFF_3322);
        assert_eq!(with_fade(0xFFFF_3322, 0.5), 0x7FFF_3322);
        // The default glow colour is the same RGB at 20 % alpha.
        assert_eq!(with_fade(0x33FF_3322, 1.0), 0x33FF_3322);
        // A fade past the end and a NaN factor cannot corrupt the colour byte order.
        assert_eq!(with_fade(0xFFFF_3322, -1.0), 0x00FF_3322);
        assert_eq!(with_fade(0xFFFF_3322, f32::NAN), 0x00FF_3322);
        assert_eq!(with_fade(0xFFFF_3322, 2.0), 0xFFFF_3322, "alpha stays a byte");
    }
}
