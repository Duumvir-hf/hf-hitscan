//! `Body` / `Stats` layout and the four engine calls the fire path makes — all verified against the
//! 1.163 Ghidra program on 2026-11-23.
//!
//! `Body_ApplyDamage` at `0x1400519d0` was re-read before writing this file:
//!
//! ```c
//! undefined8 Body_ApplyDamage(Body *this, float damage, char directHit, undefined8 param_4,
//!                             char hullContact, char defer)
//! ```
//!
//! It returns 0 for `damage == 0`, a locked part (`+0x2d0`), a dead part, or damage below the
//! armour threshold (`GetStats(partIndex)->m_mdl_armor`, i.e. `Stats+0xc4`) on a direct hit, and 1
//! when it wrote health. `directHit = 0` is the blast-style path (threshold bypassed) — that is the
//! config's `ignore_armor_threshold`. Its player-ship ×0.6 is the game's own rule and is left alone.

use std::mem::size_of;

use crate::{
    mem::{i32_at, u32_at, usize_at},
    read, readable, Vec2,
};

/// Owner `Creature*` (`Body_ApplyDamage`'s `this->base.owner.node`). `0` = terrain.
pub(crate) const OWNER: usize = 0xb8;
/// The body's class word (`Frame_Constructor` writes `0x1f`, everything else `0x0f`).
pub(crate) const CLASS: usize = 0x28;
/// A ship **Frame** (and a frame fragment) — see [`CLASS`]. A frame takes no damage from a shell:
/// its mounted parts are the fixtures that get hit, and vanilla never damages the frame itself.
pub(crate) const FRAME_CLASS: u8 = 0x1f;
pub(crate) const OID: usize = 0x190;
pub(crate) const B2_BODY: usize = 0x178;
pub(crate) const PART_INDEX: usize = 0x188;
/// `m_layer` — non-zero means "this body has a physics layer" (`Hud_DrawAimRay`'s filter).
pub(crate) const LAYER: usize = 0x1f0;
pub(crate) const HEALTH: usize = 0x2c8;
pub(crate) const HEALTH_LOCK: usize = 0x2d0;
/// Incendiary dose the thermal pass consumes into `+0x2ec`; the blast routine writes it here.
pub(crate) const INCENDIARY: usize = 0x2f0;
/// `Body.explosivePower` — what `Body_Explode` turns into a blast radius and damage.
pub(crate) const EXPLOSIVE_POWER: usize = 0x2dc;

/// A body with at least this much health is **the level's land**, not a projectile or debris.
///
/// Measured: every `lnd_close_*` body has `health == 100000.0` (phase 1 and the 12:23 play log),
/// while nothing damageable in the game comes close — a missile is 10, a shell 1, the ruin
/// fragments `Body_DieExplode` spawns are 100 and its debris particles 1000, and the toughest ship
/// part is a few hundred. Owner (`+0xb8 == 0`) alone is *not* a terrain test: debris and missiles
/// are spawned with a null owner too (`Node_AddChild` sets their *master*, not their owner), which
/// is exactly what made a beam stop on a wreck and a missile unkillable.
pub(crate) const TERRAIN_HEALTH_MIN: f32 = 10_000.0;

/// Everything the ray scan reads from a body. One `readable` of this size covers it (the body is a
/// single `0xa40` allocation).
pub(crate) const SCAN_EXTENT: usize = LAYER + size_of::<u32>();
/// The damage chain also reads health, health max and the incendiary dose.
pub(crate) const DAMAGE_EXTENT: usize = INCENDIARY + size_of::<f32>();

/// Vtable slots.
pub(crate) const VT_IS_ACTIVE: usize = 0x78;
pub(crate) const VT_LOCAL_TO_WORLD: usize = 0xe8;
pub(crate) const VT_RAY_CAST: usize = 0x20;

/// `Stats.m_category`.
pub(crate) const STATS_CATEGORY: usize = 0xb0;

/// `Stats *GetStats(int partOID)`.
pub(crate) const GET_STATS: usize = 0x1_4028_1e00;
const _: () = assert!(GET_STATS == 0x140281e00);
/// `Body_ApplyDamage` (`vtable +0x210`).
pub(crate) const APPLY_DAMAGE: usize = 0x1_4005_19d0;
const _: () = assert!(APPLY_DAMAGE == 0x1400519d0);
/// `Body_Explode` (`0x140029b60`): the game's own blast at a body, the routine `Body_DieExplode`
/// calls when a body with `explosivePower != 0` dies.
pub(crate) const EXPLODE: usize = 0x1_4002_9b60;
const _: () = assert!(EXPLODE == 0x140029b60);
/// `FireShell`'s muzzle offset (`0x140352818`).
pub(crate) const MUZZLE_OFFSET: f32 = 10.0;

/// Image base of the exe (the mod refuses to start on any other base).
const IMAGE_BASE: usize = 0x1_4000_0000;
/// A generous span: the module is a few tens of MB, and no heap or stack address is anywhere near
/// this range. Used only to tell a live `Body` from freed heap memory.
const IMAGE_SPAN: usize = 0x1000_0000;

/// A `Body` whose first word is a vtable **inside the module image**.
///
/// [`crate::readable`] cannot tell a live body from freed heap memory (Windows keeps heap pages
/// committed), but a freed `Body`'s first word is allocator metadata, not a code pointer in the
/// image. This is the check used before `Body_ApplyDamage` is called on a part, because that call
/// goes through the part's own vtable.
pub(crate) fn plausible(body: usize) -> bool {
    if !crate::readable(body, std::mem::size_of::<usize>()) {
        return false;
    }
    // SAFETY: the first word of the body was just proved readable.
    let vtable = unsafe { read::<usize>(body) };
    (IMAGE_BASE..IMAGE_BASE + IMAGE_SPAN).contains(&vtable)
}

pub(crate) type FnLocalToWorld = unsafe extern "C" fn(*mut u8, *mut Vec2);
pub(crate) type FnGetStats = unsafe extern "C" fn(i32) -> *const u8;
pub(crate) type FnApplyDamage = unsafe extern "C" fn(*mut u8, f32, u8, u64, u8, u8) -> u8;
pub(crate) type FnExplode = unsafe extern "C" fn(*mut u8) -> u8;

/// The `m_category` bit mask of a ship part, or `None` for a non-part (`partIndex < 0`) or an
/// index outside any plausible ship (`GetStats` is the game's own lookup; do not feed it garbage).
pub(crate) fn category_of(part: i32) -> Option<u32> {
    if !(0..4096).contains(&part) {
        return None;
    }
    // SAFETY: `GetStats` is a pure lookup by part index and returns null for an unknown part.
    let stats: FnGetStats = unsafe { std::mem::transmute(GET_STATS) };
    let stats = unsafe { stats(part) };
    if stats.is_null() || !readable(stats as usize, STATS_CATEGORY + 4) {
        return None;
    }
    u32_at(stats as usize + STATS_CATEGORY)
}

/// `FireShell`'s local-point → world-point transform (`vtable +0xe8`).
pub(crate) fn local_to_world(body: usize, point: Vec2) -> Option<Vec2> {
    let entry = usize_at(body).and_then(|vtable| usize_at(vtable + VT_LOCAL_TO_WORLD))?;
    if entry == 0 || !readable(entry, 1) {
        return None;
    }
    // SAFETY: the slot is `Body`'s local-point→world-point transform, called with the same two
    // arguments `FireShell` uses.
    let transform: FnLocalToWorld = unsafe { std::mem::transmute(entry) };
    let mut point = point;
    unsafe { transform(body as *mut u8, &mut point) };
    point.x.is_finite().then_some(point)
}

/// The muzzle and the raw (unnormalised) world direction, exactly as `FireShell` builds them:
/// `o = vtbl+0xe8(gun, {0,0})`, `p = vtbl+0xe8(gun, *dir_local)`, `d = p − o`,
/// `muzzle = o + 10·d`.
///
/// Measured in play (phase 1): `o` equals `gun+0x128` and `|d| = 1.0000 ± 0.00004`, which is why the
/// direction is used as the game uses it and not normalised here.
pub(crate) fn muzzle(gun: usize, dir_local: Vec2) -> Option<(Vec2, Vec2)> {
    let origin = local_to_world(gun, Vec2 { x: 0.0, y: 0.0 })?;
    let far = local_to_world(gun, dir_local)?;
    let d = Vec2 {
        x: far.x - origin.x,
        y: far.y - origin.y,
    };
    let length = (d.x * d.x + d.y * d.y).sqrt();
    if !(length > 1e-6) || !length.is_finite() {
        return None;
    }
    let muzzle = Vec2 {
        x: origin.x + MUZZLE_OFFSET * d.x,
        y: origin.y + MUZZLE_OFFSET * d.y,
    };
    Some((muzzle, d))
}

/// `Body_ApplyDamage(body, damage, directHit, 0, hullContact = 0, defer = 0)`.
///
/// # Safety
///
/// `body` must be a live body whose [`DAMAGE_EXTENT`] bytes are readable.
pub(crate) unsafe fn apply_damage(body: usize, damage: f32, direct: bool) -> u8 {
    // SAFETY: the install-time constants are the 1.163 functions, and the caller checked the body.
    let apply: FnApplyDamage = unsafe { std::mem::transmute(APPLY_DAMAGE) };
    unsafe { apply(body as *mut u8, damage, direct as u8, 0, 0, 0) }
}

/// Spawn the game's own blast at `part`, as if that part were an explosive body of `power`.
///
/// `Body_Explode` reads `explosivePower (+0x2dc)` and `armorPiercing (+0x2e0)`, returns immediately
/// when both are 0 (or when the body is not reflected), and otherwise applies blast damage + knock-back
/// around the body, spawns the particles and queues secondary ignition. Stamping the field and
/// restoring it means the part does **not** stay explosive: nothing else will detonate it later.
///
/// The blast is spawned **exactly once, at the configured power** — `impact.explosive_power` is the
/// only lever. The radius is `2·√power` **game units**, so `20` (a vanilla 130 mm HE shell) is 8.9 u and
/// `100` is 20 u; `Body_Explode` attaches its particles to the bodies it damages, so a radius smaller
/// than the gap between two parts can draw nothing. 0.7.2 had a 4× retry for that case (its victim
/// counter read 0 on every blast, so it fired on all of them and quadrupled the configured power); it
/// was removed in 0.8.2 — the power the config names is the power the game gets.
///
/// Returns `Body_Explode`'s own return value: `false` when its guard refused to run.
///
/// # Safety
///
/// `part` must be a live body whose [`DAMAGE_EXTENT`] bytes are readable.
pub(crate) unsafe fn explode(part: usize, power: f32) -> bool {
    let field = part as *mut u8;
    // SAFETY: the caller proved the body readable, which covers `+0x2dc`.
    let previous = unsafe { read::<f32>(part + EXPLOSIVE_POWER) };
    // SAFETY: `+0x2dc` is inside the same allocation.
    unsafe { field.add(EXPLOSIVE_POWER).cast::<f32>().write_unaligned(power) };
    let explode: FnExplode = unsafe { std::mem::transmute(EXPLODE) };
    // SAFETY: the explode routine is the 1.163 `Body_Explode` and `part` is a live body.
    let ran = unsafe { explode(part as *mut u8) } != 0;
    // SAFETY: restoring the field the mod borrowed.
    unsafe {
        field
            .add(EXPLOSIVE_POWER)
            .cast::<f32>()
            .write_unaligned(previous)
    };
    ran
}

/// `health (+0x2c8) > 0` and not invulnerable (`+0x2d0`), read after [`DAMAGE_EXTENT`] was proved
/// readable. Callers re-check immediately before every [`apply_damage`] (hazard H10/H16: a previous
/// hit in the same chain can have killed this part).
pub(crate) fn alive(body: usize) -> bool {
    // SAFETY: the caller proved `DAMAGE_EXTENT` readable, which covers both fields.
    let health = unsafe { read::<f32>(body + HEALTH) };
    let lock = unsafe { read::<u8>(body + HEALTH_LOCK) };
    health > 0.0 && lock == 0
}

/// `partIndex (+0x188)`; `-1` means the body is not a ship part (a shell in flight, an effect body).
pub(crate) fn part_index(body: usize) -> i32 {
    i32_at(body + PART_INDEX).unwrap_or(-1)
}

/// Does this body own a **b2 body of its own**?
///
/// `false` for a rigidly mounted part (its fixture lives on the ship's Frame) and `true` for a
/// gimballed/free body — an engine, a gun, a leg, a shell, a missile, debris. Used by the per-shot
/// log to say whether the beam ended on a joint part.
pub(crate) fn has_own_body(body: usize) -> bool {
    usize_at(body + B2_BODY).is_some_and(|b2| b2 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_addresses_are_the_ones_in_the_roadmap() {
        assert_eq!(GET_STATS, 0x140281e00);
        assert_eq!(APPLY_DAMAGE, 0x1400519d0);
        assert_eq!(VT_LOCAL_TO_WORLD, 0xe8);
        assert_eq!(MUZZLE_OFFSET, 10.0);
    }

    /// The scan's single body query must cover every field the scan reads.
    #[test]
    fn the_body_extents_cover_what_is_read() {
        assert!(SCAN_EXTENT >= LAYER + 4);
        assert!(SCAN_EXTENT >= B2_BODY + 8);
        assert!(DAMAGE_EXTENT >= HEALTH_LOCK + 1);
        assert!(DAMAGE_EXTENT <= 0xa40, "a body is a 0xa40 allocation");
    }

    #[test]
    fn a_negative_part_index_is_not_a_part() {
        assert_eq!(category_of(-1), None);
        assert_eq!(category_of(-200), None);
    }

    /// `Frame_Constructor` / `Frame_CopyConstructor` / `CreateFrameFragments` are the only writers of
    /// the class word with `0x1f`; every other body (parts, shells, missiles, terrain) is `0x0f`.
    #[test]
    fn a_frame_is_the_class_word_0x1f() {
        assert_eq!(CLASS, 0x28);
        assert_eq!(FRAME_CLASS, 0x1f);
        assert_ne!(FRAME_CLASS & 0xf, 0, "a frame passes the aim ray's `& 0xf == 0xf` filter");
    }

    #[test]
    fn the_muzzle_is_the_origin_plus_ten_times_the_raw_direction() {
        let origin = Vec2 { x: 100.0, y: 200.0 };
        let far = Vec2 { x: 100.0, y: 201.0 };
        let d = Vec2 {
            x: far.x - origin.x,
            y: far.y - origin.y,
        };
        let muzzle = Vec2 {
            x: origin.x + MUZZLE_OFFSET * d.x,
            y: origin.y + MUZZLE_OFFSET * d.y,
        };
        assert_eq!(muzzle.x, 100.0);
        assert_eq!(muzzle.y, 210.0);
    }
}
