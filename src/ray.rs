//! The ray — `Hud_DrawAimRay`'s own scan (`0x140248490`), returning every hit instead of the
//! nearest one.
//!
//! ```text
//! body array       0x143a33c90   the Body* array ITSELF (no dereference), count at 0x143a51150
//! per body         IsActive (vtable +0x78), class byte (+0x28) & 0xf == 0xf,
//!                  b2Body (+0x178) non-null, m_layer (+0x1f0) non-zero
//! fixtures         first at *(b2Body+0x70), next at fixture+8, shape at fixture+0x18,
//!                  transform = *(fixture+0x10) + 0xc
//! cast             shape vtable +0x20 = RayCast(out, in, transform, 0);
//!                  in = {p1, p2, 1.0} in b2 units (endpoints × 0.05), out = {normal, fraction}
//! ```
//!
//! # A hit is a *fixture*, and a fixture belongs to a part — read `m_userData` (+0x40)
//!
//! This is the single most important fact about hitting a HighFleet ship. A ship is **one Frame
//! body** plus a handful of free bodies. Most parts (armour, hull, ammo, blocks, FCS, …) are
//! rigidly mounted, and `Body::Reflect` gives a mounted part **no `b2Body` of its own**: it creates
//! one polygon **fixture on the Frame's body** instead (`Body_CreateFixture 0x14004b530`). Only
//! parts whose master is `GlobalNode_` — engines, guns, legs, anything torn off — get their own
//! body. So walking the body array and using the array body as the thing that was hit finds the
//! Frame (`partIndex -1`) and the gimballed parts, and nothing else.
//!
//! The mapping back to the part lives in the fixture: `Body_CreateFixture` puts the part's own
//! `Body*` into `b2FixtureDef.userData`, and `b2Fixture::Create` copies it to `b2Fixture.m_userData`
//! (verified in the 1.163 program on 2026-11-23). The fixture layout there is
//! `{ density +0x00, next +0x08, body +0x10, shape +0x18, friction +0x20, restitution +0x24,
//!   proxies +0x28, proxyCount +0x30, filter +0x34, isSensor +0x3a, userData +0x40 }`, which is also
//! why the HUD's transform argument is `*(fixture+0x10)+0xc` — that is `m_body` and its `m_xf`.
//!
//! [`Hit::part`] is therefore `m_userData` when it is set (mounted part) and the array body when it
//! is not (free body, terrain, shell, effect), and the damage chain only ever touches that part.
//!
//! # Where the checks are, and why (measured cost)
//!
//! The phase 1 probe validated every pointer it touched and cost **3.0–5.6 ms per shot** on
//! 136–205 bodies; the raycasts themselves were cheap. The `VirtualQuery` calls were the cost.
//! Phase 2 therefore validates exactly one thing per array slot — that the slot's `Body` is a
//! readable allocation of [`crate::body::SCAN_EXTENT`] bytes, which covers every field read here —
//! and then trusts what Box2D owns: the `b2Body` behind a validated body, its fixture chain, and
//! the shapes' vtables. That is the same trust the game's own aim ray extends to all 15 000 slots
//! every frame. The world cannot change under the scan: it runs synchronously from `FireGun` and
//! this mod destroys nothing until the chain has collected its hits. A fixture's `m_userData` is a
//! loose pointer, so it *is* validated (`readable` for a mounted part) before it is returned.
//!
//! # Own parts
//!
//! `Hud_DrawAimRay` hides the shooter's parts behind the `+0x224` flag. This scan instead skips
//! every body owned by the shooter's own creature (`+0xb8 == shooter`) and every fixture whose part
//! has that owner: the same set for **parts**, without a write and without hazard H9 (a leaked flag
//! would hide a part from the player's aim ray for the rest of the battle).
//!
//! Shells, missiles and debris are **not** owned by a creature: `FireShell` and the death path attach
//! them to `GlobalNode_` (`(*GlobalNode_ + 0x50)(GlobalNode_, body, 0, 0)`). They are therefore not
//! excluded (your own old shells can be hit by a later laser) and, since 0.7.0, they take damage.

use std::mem::size_of;

use crate::{
    body::{self, B2_BODY, CLASS, LAYER, OWNER, SCAN_EXTENT, VT_IS_ACTIVE, VT_RAY_CAST},
    mem::usize_at,
    read, readable, readable_extent, Vec2,
};

pub(crate) const BODY_ARRAY: usize = 0x1_43a3_3c90;
const _: () = assert!(BODY_ARRAY == 0x143a33c90);
pub(crate) const BODY_COUNT: usize = 0x1_43a5_1150;
const _: () = assert!(BODY_COUNT == 0x143a51150);
/// `__05` — game units → b2 units.
pub(crate) const WORLD_TO_B2: f32 = 0.05;
/// Sanity bound for a corrupt count; a real fight is a few thousand.
const BODY_SLOTS_MAX: i32 = 20_000;
/// A fixture walk longer than this is a corrupt list, not a real body.
///
/// This has to be generous: a ship Frame carries **one fixture per rigidly mounted part**, so a
/// capital ship's frame can legitimately have a few hundred fixtures. (Phase 1's 108–225 casts per
/// shot were mostly these.)
const FIXTURES_PER_BODY_MAX: usize = 8192;

const B2_FIXTURE_LIST: usize = 0x70;
const FIXTURE_NEXT: usize = 0x08;
const FIXTURE_TRANSFORM: usize = 0x10;
const FIXTURE_SHAPE: usize = 0x18;
/// `b2Fixture.m_userData` — the part `Body*` that owns this fixture (see the module docs).
const FIXTURE_USER_DATA: usize = 0x40;
/// `b2Fixture` is 0x48 bytes; the user data must be inside it.
const _: () = assert!(FIXTURE_USER_DATA + 8 <= 0x48);
/// `b2Transform*` = `*(fixture + 0x10) + 0xc`.
const TRANSFORM_OFFSET_IN_OWNER: usize = 0x0c;

type FnIsActive = unsafe extern "C" fn(*mut u8) -> u8;
type FnRayCast = unsafe extern "C" fn(
    shape: *mut u8,
    output: *mut RayCastOutput,
    input: *const RayCastInput,
    transform: *const u8,
    child_index: i32,
) -> u8;

/// `b2RayCastInput` = `{ p1, p2, maxFraction }`, 0x14 bytes.
#[repr(C)]
struct RayCastInput {
    p1: Vec2,
    p2: Vec2,
    max_fraction: f32,
}

/// `b2RayCastOutput` = `{ normal, fraction }`, 0xc bytes.
#[repr(C)]
struct RayCastOutput {
    normal: Vec2,
    fraction: f32,
}

const _: () = assert!(size_of::<RayCastInput>() == 0x14);
const _: () = assert!(size_of::<RayCastOutput>() == 0x0c);

pub(crate) struct Hit {
    /// The part that owns the hit fixture: `m_userData` for a mounted part, the array body itself
    /// for a free body (terrain, shell, effect).
    pub(crate) part: usize,
    pub(crate) fraction: f32,
}

/// What one scan cost, for hazard H14.
#[derive(Default)]
pub(crate) struct Stats {
    pub(crate) slots: usize,
    pub(crate) candidates: usize,
    pub(crate) casts: usize,
}

/// Walk the body array and cast every fixture, nearest hit first, one entry per body.
pub(crate) fn cast(shooter: usize, muzzle: Vec2, d: Vec2, range: f32) -> (Vec<Hit>, Stats) {
    let mut stats = Stats::default();
    let mut hits: Vec<Hit> = Vec::new();

    let Some(count) = crate::mem::i32_at(BODY_COUNT) else {
        return (hits, stats);
    };
    let count = count.clamp(0, BODY_SLOTS_MAX) as usize;
    if count == 0 {
        return (hits, stats);
    }
    // One query for the whole array; per-slot queries would be 15 000 syscalls.
    let count = readable_extent(BODY_ARRAY, count * size_of::<usize>()) / size_of::<usize>();
    if count == 0 {
        log::warn!("hf_hitscan: the body array {BODY_ARRAY:#x} is not readable; the ray is skipped");
        return (hits, stats);
    }

    let input = RayCastInput {
        p1: Vec2 {
            x: muzzle.x * WORLD_TO_B2,
            y: muzzle.y * WORLD_TO_B2,
        },
        p2: Vec2 {
            x: (muzzle.x + d.x * range) * WORLD_TO_B2,
            y: (muzzle.y + d.y * range) * WORLD_TO_B2,
        },
        max_fraction: 1.0,
    };

    let mut actives = ActiveCache::default();
    let mut shapes = ShapeCache::default();

    for slot in 0..count {
        let Some(body) = usize_at(BODY_ARRAY + slot * size_of::<usize>()) else {
            continue;
        };
        // The one check per body: it is a readable allocation covering every field read below.
        if body == 0 || !readable(body, SCAN_EXTENT) {
            continue;
        }
        stats.slots += 1;

        // SAFETY: `SCAN_EXTENT` bytes at `body` were just proved readable.
        let class = unsafe { read::<u8>(body + CLASS) };
        if class & 0xf != 0xf {
            continue;
        }
        let owner = unsafe { read::<usize>(body + OWNER) };
        if shooter != 0 && owner == shooter {
            continue;
        }
        let b2 = unsafe { read::<usize>(body + B2_BODY) };
        if b2 == 0 {
            continue;
        }
        if unsafe { read::<u32>(body + LAYER) } == 0 {
            continue;
        }
        if !actives.is_active(body) {
            continue;
        }
        stats.candidates += 1;

        // SAFETY: Box2D owns `b2` and its fixture chain; the body was validated and nothing this
        // mod does can destroy it during the scan.
        let mut fixture = unsafe { read::<usize>(b2 + B2_FIXTURE_LIST) };
        let mut walked = 0usize;
        while fixture != 0 && walked < FIXTURES_PER_BODY_MAX {
            walked += 1;
            let next = unsafe { read::<usize>(fixture + FIXTURE_NEXT) };
            let shape = unsafe { read::<usize>(fixture + FIXTURE_SHAPE) };
            let transform_owner = unsafe { read::<usize>(fixture + FIXTURE_TRANSFORM) };
            if shape != 0 && transform_owner != 0 {
                if let Some(ray_cast) = shapes.ray_cast_of(shape) {
                    let transform = (transform_owner + TRANSFORM_OFFSET_IN_OWNER) as *const u8;
                    let mut output = RayCastOutput {
                        normal: Vec2 { x: 0.0, y: 0.0 },
                        fraction: 0.0,
                    };
                    stats.casts += 1;
                    // SAFETY: the slot was read from a shape's own vtable; all three pointers
                    // belong to Box2D objects the game created.
                    let hit =
                        unsafe { ray_cast(shape as *mut u8, &mut output, &input, transform, 0) };
                    if hit != 0
                        && output.fraction.is_finite()
                        && output.fraction >= 0.0
                        && output.fraction <= 1.0
                    {
                        // A mounted part has no body of its own; its fixture's userData is the
                        // part. A free body points at itself.
                        let mut part = unsafe { read::<usize>(fixture + FIXTURE_USER_DATA) };
                        if part == 0 {
                            part = body;
                        } else if part != body {
                            // The only unproven pointer in the walk: validate it, then keep the
                            // same own-body exclusion as the array walk.
                            if !readable(part, body::DAMAGE_EXTENT) {
                                part = 0;
                            } else if shooter != 0 && unsafe { read::<usize>(part + OWNER) } == shooter
                            {
                                part = 0;
                            }
                        }
                        if part != 0 {
                            hits.push(Hit {
                                part,
                                fraction: output.fraction,
                            });
                        }
                    }
                }
            }
            fixture = next;
        }
    }

    hits.sort_by(|a, b| a.fraction.total_cmp(&b.fraction));
    // A part can own several fixtures, and another body can sit between two of them, so this is a
    // first-occurrence pass rather than `dedup_by`. Since the fix that resolves fixtures to their
    // parts, the same part usually appears once.
    let mut unique: Vec<Hit> = Vec::with_capacity(hits.len());
    for hit in hits {
        if !unique.iter().any(|kept| kept.part == hit.part) {
            unique.push(hit);
        }
    }
    (unique, stats)
}

/// `IsActive` with the vtable and the entry resolved once per distinct vtable instead of once per
/// body — there are one or two body vtables in a battle, so this removes ~2 queries per slot.
#[derive(Default)]
struct ActiveCache {
    known: Vec<(usize, Option<FnIsActive>)>,
}

impl ActiveCache {
    fn is_active(&mut self, body: usize) -> bool {
        // SAFETY: covered by the caller's `SCAN_EXTENT` proof.
        let vtable = unsafe { read::<usize>(body) };
        let entry = match self.known.iter().find(|(seen, _)| *seen == vtable) {
            Some((_, entry)) => *entry,
            None => {
                let entry = usize_at(vtable + VT_IS_ACTIVE)
                    .filter(|entry| *entry != 0 && readable(*entry, 1))
                    // SAFETY: the entry is a readable code address from the object's own vtable.
                    .map(|entry| unsafe { std::mem::transmute::<usize, FnIsActive>(entry) });
                self.known.push((vtable, entry));
                entry
            }
        };
        match entry {
            // SAFETY: resolved from the body's own vtable and checked readable.
            Some(is_active) => unsafe { is_active(body as *mut u8) != 0 },
            None => false,
        }
    }
}

/// The shape `RayCast` slot, resolved once per distinct shape vtable (polygon vs circle).
#[derive(Default)]
struct ShapeCache {
    known: Vec<(usize, Option<FnRayCast>)>,
}

impl ShapeCache {
    fn ray_cast_of(&mut self, shape: usize) -> Option<FnRayCast> {
        // SAFETY: `shape` comes from a fixture of an already validated body.
        let vtable = unsafe { read::<usize>(shape) };
        if let Some((_, entry)) = self.known.iter().find(|(seen, _)| *seen == vtable) {
            return *entry;
        }
        let entry = usize_at(vtable + VT_RAY_CAST)
            .filter(|entry| *entry != 0 && readable(*entry, 1))
            // SAFETY: the entry is the shape's own `RayCast` from its vtable.
            .map(|entry| unsafe { std::mem::transmute::<usize, FnRayCast>(entry) });
        self.known.push((vtable, entry));
        entry
    }
}

/// Re-check a hit part immediately before damaging it (hazard H10/H16): a previous hit in the same
/// chain can have killed this part, and an explosion can reap bodies from the array.
///
/// `IsActive` is deliberately **not** part of this test: a mounted part has no `b2Body` of its own,
/// and the only properties the damage chain needs are "this memory is still a live `Body`" (see
/// [`body::plausible`], which rejects freed heap memory) and the game's own `health > 0` /
/// `healthLock == 0` guard, re-read in [`body::alive`].
pub(crate) fn still_valid(part: usize) -> bool {
    readable(part, body::DAMAGE_EXTENT) && body::plausible(part)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_abi_matches_box2d() {
        assert_eq!(size_of::<RayCastInput>(), 0x14, "p1, p2, maxFraction");
        assert_eq!(size_of::<RayCastOutput>(), 0x0c, "normal, fraction");
        assert_eq!(size_of::<Vec2>(), 8);
    }

    #[test]
    fn the_addresses_are_the_ones_in_the_roadmap() {
        assert_eq!(BODY_ARRAY, 0x143a33c90);
        assert_eq!(BODY_COUNT, 0x143a51150);
    }

    /// The `b2Fixture` layout read from the 1.163 program — this is what turns a fixture hit into a
    /// part, so a wrong `m_userData` offset would damage the wrong body (or the frame, i.e. nothing).
    #[test]
    fn the_fixture_fields_are_the_ones_in_the_1_163_fixture() {
        assert_eq!(FIXTURE_NEXT, 0x08, "m_next");
        assert_eq!(FIXTURE_TRANSFORM, 0x10, "m_body (its m_xf is +0xc)");
        assert_eq!(FIXTURE_SHAPE, 0x18, "m_shape");
        assert_eq!(FIXTURE_USER_DATA, 0x40, "m_userData = the part Body");
        assert!(FIXTURE_USER_DATA + size_of::<usize>() <= 0x48, "a fixture is 0x48 bytes");
    }

    #[test]
    fn the_ray_endpoints_are_scaled_into_b2_units() {
        let muzzle = Vec2 { x: 1000.0, y: -500.0 };
        let d = Vec2 { x: 0.0, y: 1.0 };
        let range = 3000.0;
        assert_eq!(muzzle.x * WORLD_TO_B2, 50.0);
        assert_eq!(muzzle.y * WORLD_TO_B2, -25.0);
        assert_eq!((muzzle.y + d.y * range) * WORLD_TO_B2, 125.0);
    }
}
