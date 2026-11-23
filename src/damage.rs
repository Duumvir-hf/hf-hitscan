//! The damage chain. A shot whose loaded ammo is bound in `shells` no longer spawns a shell: the
//! muzzle is rebuilt exactly as `FireShell` builds it, the HUD's ray is cast from it, and the
//! bodies it crosses take damage, nearest first.
//!
//! ```
//! gate(gun, dir_local, ammo_pos):
//!     ammo   = table[ammo_pos]                       (re-read every shot, H3)
//!     item   = string_at(ammo + 0x08)
//!     laser  = config.shells[item]                   (snapshot from init, H18)
//!     if none: forward to vanilla FireShell
//!     sound::launch(ammo, gun, owner)                (phase 4 — the shot's own bang)
//!
//!     o, p, d, muzzle = FireShell's own three steps (vtable +0xe8, muzzle = o + 10·d)
//!     d = rotate(d, HGE.Random_Float(-spread, +spread))     phase 5.5, §5.13; spread 0 = no draw
//!     hits = ray::cast(shooter, muzzle, d, laser.range)      nearest first, one entry per body
//!
//!     for (body, fraction) in hits:
//!         dist = fraction * laser.range
//!         dmg  = laser.damage - laser.damage_decay_per_unit * dist   ; <= 0 ends the beam
//!         owner = *(usize*)(body + 0xb8)
//!         part  = *(i32*)(body + 0x188); class = *(u8*)(body + 0x28); health = *(f32*)(body + 0x2c8)
//!         if part invalid and owner == 0 and health >= 10000: break   land: the beam ends here
//!         if part invalid and class == 0x1f: continue   a ship Frame: never damaged, never stops
//!         if owner is not a live object: continue        freed memory, built terrain, …
//!         if health <= 0 or healthLock != 0: continue    killed earlier in this same chain (H10/H16)
//!         category = part valid ? GetStats(part).m_category : 0     (free objects have none)
//!         applied = Body_ApplyDamage(body, dmg, direct = !ignore_armor_threshold, 0, 0, 0)
//!         blast here?                                impact.at first/last, right after the call
//!         if !laser.pierce: break
//!         if category & laser.stop_mask: break     damage first, then stop
//!         if max_parts != 0 and damaged >= max_parts: break
//!     return true                                   FireGun reports "fired"
//! ```
//!
//! Decisions that override the roadmap text, most of them measured in play:
//!
//! * the muzzle transform is vtable `+0xe8` and `d` is **not** normalised (phase 1: `|d| = 1.0000`
//!   and `o == gun+0x128`);
//! * terrain is an **ownerless non-part** body with effectively indestructible health (`+0xb8 == 0`,
//!   no part index, `health >= 10000` — the level's `lnd_*` bodies carry 100000). The owner alone is
//!   not a test: debris and missiles are spawned with a null owner too (`Node_AddChild` sets their
//!   *master*), and treating them as land ended the beam on wrecks and made missiles unkillable;
//! * a body with a valid `partIndex` is a **ship part**; a body without one but with a live owner is
//!   a **free object** — a missile in flight, a shell, debris — and takes damage like a part, only
//!   without a category (`Body_ApplyDamage` on such a body is exactly what vanilla's own blast area
//!   damage does). It never stops the beam;
//! * a **ship Frame** (`class 0x1f`, `Frame_Constructor`/`CreateFrameFragments`) is skipped and does
//!   not stop the beam: its mounted parts are the fixtures that take the hits;
//! * own bodies are excluded by owner in [`crate::ray`], not by the `+0x224` flag (H9);
//! * the blast runs **as soon as the chain has finished with its target**, not after the whole
//!   chain: a later part's death could reap the body in between, which is why the explosion
//!   sometimes did not appear.
//!
//! Nothing else changes: `FireGun` already spent the round, set the cooldown and started the reload
//! before this call. Phase 3 adds one line per handled shot to [`crate::beam`] — the beam is drawn
//! by the render tick, never here. Phase 4 adds the shot's launch sound ([`crate::sound`]), played
//! before the chain; there is still no muzzle flash and no particle. Phase 5.5 adds the per-type
//! `spread` ([`rotate`]): one uniform roll of the firing direction, in radians like vanilla.

use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use crate::{
    ammo, ammo::Ammo, beam, body, config::BlastAt, config::ResolvedType, mem, ray, read, sound,
    CONFIG, Vec2,
};

static SHOT: AtomicU64 = AtomicU64::new(0);

/// Try to fire `gun`'s loaded ammo as a laser.
///
/// `true` means the shot was handled — the caller must **not** call vanilla `FireShell`. `false`
/// means "not a laser / cannot handle it safely", and the caller forwards to vanilla.
///
/// # Safety
///
/// `gun` and `dir` are `FireGun`'s live arguments; the gate runs inside its frame.
pub(crate) unsafe fn try_fire(gun: *mut u8, dir: *const Vec2, ammo_position: i32) -> bool {
    let Some(config) = CONFIG.get() else {
        return false;
    };
    let Some(ammo) = Ammo::at(ammo_position) else {
        log::warn!("hf_hitscan: ammo_pos {ammo_position} is outside the ammo table; vanilla shell");
        return false;
    };
    let Some(laser) = config.laser_for_item(&ammo.item) else {
        ammo::note_resolution(&ammo, None);
        return false;
    };
    ammo::note_resolution(&ammo, Some(&laser.name));

    let Some(dir_local) = mem::vec2_at(dir as usize) else {
        log::warn!("hf_hitscan: the fire direction pointer {dir:p} is not readable; vanilla shell");
        return false;
    };
    let gun = gun as usize;
    let Some((muzzle, d)) = body::muzzle(gun, dir_local) else {
        log::warn!("hf_hitscan: degenerate fire direction; vanilla shell");
        return false;
    };

    let shot = SHOT.fetch_add(1, Ordering::Relaxed) + 1;
    let verbose = crate::verbose();
    let started = Instant::now();

    // Phase 5.5: vanilla's cannon scatter — **one** uniform roll in `[-spread, +spread]` radians,
    // applied as a rotation of the firing direction before the scan (roadmap §5.13). The muzzle does
    // not move, `|d|` is preserved, and `spread == 0` draws no number at all, so a config without the
    // key is bit-for-bit the 0.8.x path. The rotated `d` is what the scan, every distance, the blast
    // and the beam use.
    let scatter = if laser.spread > 0.0 {
        match sound::random_float(-laser.spread, laser.spread) {
            Some(angle) => angle,
            None => {
                // The HGE interface is unreadable: vanilla would have crashed in the same call.
                // A straight shot is the only sane fallback — logged, because it changes the aim.
                if verbose {
                    log::warn!(
                        "hf_hitscan: shot {shot}: HGE.Random_Float is unreachable; firing without \
                         spread"
                    );
                }
                0.0
            }
        }
    } else {
        0.0
    };
    let d = if scatter == 0.0 { d } else { rotate(d, scatter) };

    let shooter = mem::usize_at(gun + body::OWNER).unwrap_or(0);
    if shooter == 0 && verbose {
        log::warn!(
            "hf_hitscan: shot {shot}: the gun has no owner creature; the shooter's own bodies are \
             not excluded from the ray"
        );
    }

    // Phase 4: the launch sound `Body_InitializeCannonShell` would have played, from the same ammo
    // record and at the shot rather than after the damage work. It has its own `catch_unwind`: a
    // panic here must not reach the gate, which would then fire a vanilla shell as well.
    let _ = catch_unwind(AssertUnwindSafe(|| sound::launch(&ammo, gun, shooter)));

    let scan_started = Instant::now();
    let (hits, stats) = ray::cast(shooter, muzzle, d, laser.range);
    let scan_micros = scan_started.elapsed().as_micros();

    let mut tally = Tally::default();
    let mut total_damage = 0.0f32;
    let mut stop = Stop::AllHits;
    let mut stop_distance = 0.0f32;
    // Wall time spent inside the game's own calls, which is what a long shot really costs.
    let mut apply_micros = 0u128;
    // The two ends of the beam, for `impact.at`: the part it entered and the last part it touched.
    let mut first_part = 0usize;
    let mut first_distance = 0.0f32;
    let mut last_part = 0usize;
    let mut last_distance = 0.0f32;
    // The blast is run as soon as the chain has finished with the body it belongs to (see below),
    // so these track it for the summary line.
    let mut blast = "no".to_owned();
    let mut blast_done = false;
    // Where the beam actually went, for the summary line: the first and last body it touched.
    let mut first_hit = String::new();
    let mut last_hit = String::new();
    // A piercing type whose ray found almost nothing is the "it stopped on the engine" report, so
    // every hit of such a shot is printed in full.
    let short_piercing = laser.pierce && hits.len() <= 3;
    for (index, hit) in hits.iter().enumerate() {
        let distance = hit.fraction * laser.range;
        let Some(damage) = damage_at(laser, distance) else {
            stop = if distance > laser.range {
                Stop::Range
            } else {
                Stop::Spent
            };
            stop_distance = distance;
            break;
        };
        let part_body = hit.part;
        // Until something has been damaged, every verdict is printed: a shot that does nothing has
        // to explain itself in the log (a wreck, a frame or a block is not a bug).
        let explain = verbose && tally.damaged == 0;
        if !ray::still_valid(part_body) {
            tally.dead += 1;
            if explain {
                log::info!(
                    "hf_hitscan: shot {shot}: hit {}: dist {distance:.1} body {part_body:#x} \
                     vanished or inactive; skipped",
                    index + 1
                );
            }
            continue;
        }
        // SAFETY: `still_valid` proved `body::DAMAGE_EXTENT` readable.
        let owner = unsafe { read::<usize>(part_body + body::OWNER) };
        let class = unsafe { read::<u8>(part_body + body::CLASS) };
        let health = unsafe { read::<f32>(part_body + body::HEALTH) };
        let part = body::part_index(part_body);
        // A **part** is a body whose index the game can resolve to `Stats`; `GetStats` is null for
        // a free body (and for a stale index), which is what tells a missile from a part of a design.
        let part_category = if (0..4096).contains(&part) {
            body::category_of(part)
        } else {
            None
        };
        let kind = classify(part_category.is_some(), owner == 0, health, class);
        // Land is **not** "anything with a null owner": debris and missiles are spawned with a null
        // owner too (`Node_AddChild` sets their master, not their owner), so the old owner test made
        // a beam end on a wreck and a missile unkillable. The land bodies carry 100000 hp; nothing
        // damageable comes near it (see `body::TERRAIN_HEALTH_MIN`).
        if kind == HitKind::Terrain {
            tally.terrain += 1;
            // Terrain takes no damage. It ends the beam unless the type asks to go through it.
            if laser.through_terrain {
                if verbose {
                    let oid = mem::string_at(part_body + body::OID).unwrap_or_default();
                    log::info!(
                        "hf_hitscan: shot {shot}: hit {}: dist {distance:.1} terrain (class \
                         {class:#04x}, owner {owner:#x}, health {health:.1}, oid {oid:?}); \
                         through_terrain, the beam carries on",
                        index + 1
                    );
                }
                continue;
            }
            stop = Stop::Terrain;
            stop_distance = distance;
            if verbose {
                let oid = mem::string_at(part_body + body::OID).unwrap_or_default();
                log::info!(
                    "hf_hitscan: shot {shot}: hit {}: dist {distance:.1} terrain (class \
                     {class:#04x}, partIndex {part}, owner {owner:#x}, health {health:.1}, oid \
                     {oid:?}); the beam ends",
                    index + 1
                );
            }
            break;
        }
        if kind == HitKind::Frame {
            // A ship's Frame (and the fragments of one): vanilla never damages it — its mounted
            // parts are the fixtures that take hits — and it must not stop the beam.
            tally.not_parts += 1;
            if explain {
                log::info!(
                    "hf_hitscan: shot {shot}: hit {}: dist {distance:.1} body {part_body:#x} is a \
                     ship Frame (class {class:#04x}); skipped",
                    index + 1
                );
            }
            continue;
        }
        // A free-object hit is rare and is the one thing the log must never hide ("did the laser
        // touch the missile?"), so every verdict on one is printed, not only before the first
        // damaged body. A piercing shot with three hits or fewer prints all of them too: that is
        // the "it stopped on the engine" case.
        let loud = explain || kind == HitKind::FreeObject || short_piercing;
        if owner != 0 && !body::plausible(owner) {
            // A fixture whose userData is not really a body (a freed allocation, a terrain segment
            // built by another path) must never reach `Body_ApplyDamage` — but a **null** owner is
            // fine: `Body_ApplyDamage` only uses the creature for the player's ×0.6 rule and skips a
            // null one, so an ownerless part is still damageable.
            tally.dead += 1;
            if loud {
                log::info!(
                    "hf_hitscan: shot {shot}: hit {}: dist {distance:.1} body {part_body:#x} has an \
                     owner {owner:#x} that is not a live object; skipped",
                    index + 1
                );
            }
            continue;
        }
        if !body::alive(part_body) {
            tally.dead += 1;
            if loud {
                log::info!(
                    "hf_hitscan: shot {shot}: hit {}: dist {distance:.1} body {part_body:#x} \
                     (partIndex {part}) is dead or locked; skipped",
                    index + 1
                );
            }
            continue;
        }
        // A body with a valid part index is a ship part; anything else with a live owner is a free
        // object — a missile in flight, a shell, debris. `Body_ApplyDamage` works on those (vanilla's
        // blasts damage them the same way), and they have no category, so no `stop_at` can match.
        let category = match kind {
            HitKind::Part => {
                tally.parts += 1;
                part_category.unwrap_or(0)
            }
            HitKind::FreeObject => {
                tally.objects += 1;
                0
            }
            HitKind::Terrain | HitKind::Frame => unreachable!("handled above"),
        };
        if verbose {
            if first_hit.is_empty() {
                first_hit = describe(part_body, distance, kind);
            }
            last_hit = describe(part_body, distance, kind);
        }

        let direct = !laser.ignore_armor_threshold;
        let is_first = first_part == 0;
        if is_first {
            first_part = part_body;
            first_distance = distance;
        }
        last_part = part_body;
        last_distance = distance;
        // Whether the chain will stop on this body, known before the damage call: `first`/`last`
        // picks the blast target, and a `last` blast has to run here, while the body is still the
        // one the beam just touched.
        let predicted_last = !continues(laser, category, tally.damaged + 1);
        let blast_here = laser.explosion
            && !blast_done
            && blast_target_here(laser.blast_at, is_first, predicted_last);
        // The death path clears the body's `reflected` byte and `Body_Explode` refuses a body that
        // is not reflected — the `FIZZLED` case in the logs, i.e. every blast on a part the beam
        // itself kills. When this hit is lethal, explode **first**, while the body is still alive.
        let lethal = damage >= health;
        if blast_here && lethal {
            let blast_started = Instant::now();
            // SAFETY: `still_valid` and `alive` proved the body readable and alive.
            let outcome = unsafe { body::explode(part_body, laser.explosive_power) };
            apply_micros += blast_started.elapsed().as_micros();
            blast_done = true;
            blast = blast_label(distance, laser.blast_at, outcome);
        }
        let apply_started = Instant::now();
        // SAFETY: `still_valid` proved the body readable and alive.
        let applied = unsafe { body::apply_damage(part_body, damage, direct) };
        apply_micros += apply_started.elapsed().as_micros();
        if applied != 0 {
            tally.damaged += 1;
            total_damage += damage;
            if laser.incendiary != 0.0 {
                // SAFETY: covered by the same `DAMAGE_EXTENT` proof; the dose is a plain float.
                unsafe {
                    let dose = read::<f32>(part_body + body::INCENDIARY);
                    (part_body as *mut u8)
                        .add(body::INCENDIARY)
                        .cast::<f32>()
                        .write_unaligned(dose + laser.incendiary);
                }
            }
        } else {
            tally.blocked += 1;
        }
        // A free-object hit is rare and is the one thing a damage log must never hide ("did the
        // laser touch the missile?"), so it prints even when the shot has already damaged something.
        if loud {
            let oid = mem::string_at(part_body + body::OID).unwrap_or_default();
            let verdict = if applied != 0 { "applied" } else { "blocked by armour" };
            if kind == HitKind::Part {
                log::info!(
                    "hf_hitscan: shot {shot}: hit {}: dist {distance:.1} part {part} oid {oid:?} \
                     cat {category:#010x} damage {damage:.2} {verdict} (health {health:.1})",
                    index + 1
                );
            } else {
                log::info!(
                    "hf_hitscan: shot {shot}: hit {}: dist {distance:.1} free body {part_body:#x} \
                     (class {class:#04x}, owner {owner:#x}, oid {oid:?}) damage {damage:.2} \
                     {verdict} (health {health:.1})",
                    index + 1
                );
            }
        }
        // The blast for a body this hit does **not** kill runs right after the damage call: a body
        // the chain goes on to damage (or an explosion that reaps bodies) can be gone before the
        // loop ends — that was the original "the explosion sometimes does not spawn" report.
        if blast_here && !lethal {
            let blast_started = Instant::now();
            // SAFETY: `still_valid` proved the body readable; it survived the damage call above.
            let outcome = unsafe { body::explode(part_body, laser.explosive_power) };
            apply_micros += blast_started.elapsed().as_micros();
            blast_done = true;
            blast = blast_label(distance, laser.blast_at, outcome);
        }
        if !continues(laser, category, tally.damaged) {
            stop = if !laser.pierce {
                Stop::NoPierce
            } else if category & laser.stop_mask != 0 {
                Stop::StopCategory
            } else {
                Stop::MaxParts
            };
            stop_distance = distance;
            break;
        }
    }

    // Fallback for a `last` target the loop never predicted (the beam ended at the end of the hit
    // list, at `range` or on spent damage): the body was touched microseconds ago, so this is the
    // same call 0.4.x-0.6.x made for every shot.
    if laser.explosion && !blast_done {
        let (target, target_distance, where_) = match laser.blast_at {
            BlastAt::First => (first_part, first_distance, "first"),
            BlastAt::Last => (last_part, last_distance, "last"),
        };
        if target != 0 {
            if ray::still_valid(target) {
                let blast_started = Instant::now();
                // SAFETY: `still_valid` proved the body readable.
                let outcome = unsafe { body::explode(target, laser.explosive_power) };
                apply_micros += blast_started.elapsed().as_micros();
                blast = blast_label(target_distance, laser.blast_at, outcome);
            } else if verbose {
                log::warn!(
                    "hf_hitscan: shot {shot}: the {where_} blast target is no longer a live Body; \
                     no blast"
                );
            }
        }
    }

    // Where the beam ends is decided here, from what the chain already measured — the draw pass
    // never re-derives geometry:
    //   * terrain (`owner == 0`) ended the beam -> the terrain hit;
    //   * otherwise the last part the beam reached (for `pierce: false` that is the one part it
    //     hit, for a piercing type the far side, and for `max_parts`/`stop_at` the part it stopped
    //     on);
    //   * nothing was hit -> the full `range`, because the round is already spent and the player
    //     needs to see where it went.
    let beam_end = match stop {
        Stop::Terrain => stop_distance,
        _ if last_part != 0 => last_distance,
        _ => laser.range,
    };
    let beam_to = Vec2 {
        x: muzzle.x + d.x * beam_end,
        y: muzzle.y + d.y * beam_end,
    };
    beam::spawn(laser, muzzle, beam_to);

    if verbose {
        let stopped = if matches!(stop, Stop::AllHits) {
            stop.name().to_owned()
        } else {
            format!("{} at {stop_distance:.1} u", stop.name())
        };
        // A scatter the log does not print is indistinguishable from a miss: the angle is the
        // difference between "the laser was wrong" and "the laser was pointed 0.03 rad off".
        let scatter_note = if laser.spread > 0.0 {
            format!(" (scatter {scatter:+.4} rad, spread {:.4})", laser.spread)
        } else {
            String::new()
        };
        log::info!(
            "hf_hitscan: shot {shot}: {gun:?} + {item:?} -> {laser:?}: {hits} hit(s) \
             [{parts} part, {objects} object, {not_parts} frame, {terrain} terrain, {dead} dead], \
             {damaged} damaged, {blocked} blocked for {total:.1}; first {first_hit}, last \
             {last_hit}; stopped: {stopped}; beam to {beam_end:.1} u{scatter_note}; blast {blast}; \
             {slots} \
             slot(s) / {candidates} candidate(s) / {casts} cast(s) in {scan_micros} us (chain \
             {chain} us, of which {apply_micros} us in the game's damage/blast calls)",
            gun = mem::string_at(gun + body::OID).unwrap_or_else(|| "?".to_owned()),
            item = ammo.item,
            laser = laser.name,
            hits = hits.len(),
            parts = tally.parts,
            objects = tally.objects,
            not_parts = tally.not_parts,
            terrain = tally.terrain,
            dead = tally.dead,
            damaged = tally.damaged,
            blocked = tally.blocked,
            total = total_damage,
            first_hit = if first_hit.is_empty() { "-" } else { first_hit.as_str() },
            last_hit = if last_hit.is_empty() { "-" } else { last_hit.as_str() },
            scatter_note = scatter_note.as_str(),
            blast = blast.as_str(),
            slots = stats.slots,
            candidates = stats.candidates,
            casts = stats.casts,
            chain = started.elapsed().as_micros(),
        );
    }
    true
}

/// Rotate `v` by `angle` radians — vanilla's cannon scatter (§5.13), applied to the **firing
/// direction only**: the muzzle does not move and the rotation preserves `|d|`, so a shot keeps its
/// configured reach. `0` is the identity and is never called in that case.
pub(crate) fn rotate(v: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2 {
        x: v.x * cos - v.y * sin,
        y: v.x * sin + v.y * cos,
    }
}

/// How many hits fell into each bucket, for the per-shot log line.
#[derive(Default)]
struct Tally {
    /// Hits that were live ship parts.
    parts: usize,
    /// Hits that were free objects with a live owner: missiles, shells, debris.
    objects: usize,
    /// Hits that were a ship Frame (`class 0x1f`).
    not_parts: usize,
    /// Hits that were the level's ground (`pierce.through_terrain` lets the beam carry on).
    terrain: usize,
    /// Hits that were already dead, locked, or gone.
    dead: usize,
    /// Bodies the game actually damaged.
    damaged: usize,
    /// Bodies the game refused (below the armour threshold).
    blocked: usize,
}

/// `"oid"@123.4 (joint)` / `"oid"@123.4 (mounted)` / `free body` / `frame` — for the summary line.
///
/// `joint` is a body with its **own** b2 body: an engine, a gun, a leg, a free body. `mounted` is a
/// part whose fixture lives on the ship's Frame (armour, truss, bridge, …). That distinction is what
/// tells a "the beam stopped on the engine" report apart from a plain miss.
fn describe(part_body: usize, distance: f32, kind: HitKind) -> String {
    let oid = mem::string_at(part_body + body::OID).unwrap_or_default();
    let joint = if body::has_own_body(part_body) {
        "joint"
    } else {
        "mounted"
    };
    match kind {
        HitKind::Part => format!("{oid:?}@{distance:.1} ({joint})"),
        HitKind::FreeObject => format!("free body@{distance:.1}"),
        HitKind::Frame => format!("frame@{distance:.1}"),
        HitKind::Terrain => format!("terrain@{distance:.1}"),
    }
}

/// Why the chain stopped walking the hit list.
#[derive(Clone, Copy)]
enum Stop {
    /// Every hit in the list was handled.
    AllHits,
    /// The next hit was beyond `range`.
    Range,
    /// Damage had decayed to zero, so the beam was spent.
    Spent,
    /// A terrain body (`owner == 0`).
    Terrain,
    /// `pierce` is off: the first part ended the beam.
    NoPierce,
    /// A `stop_at` category ended the beam.
    StopCategory,
    /// `max_parts` reached.
    MaxParts,
}

impl Stop {
    fn name(self) -> &'static str {
        match self {
            Stop::AllHits => "the end of the hit list",
            Stop::Range => "range",
            Stop::Spent => "spent damage",
            Stop::Terrain => "terrain",
            Stop::NoPierce => "pierce off",
            Stop::StopCategory => "stop_at",
            Stop::MaxParts => "max_parts",
        }
    }
}

/// What a hit body is, from the fields the chain reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HitKind {
    /// The level's land: ownerless, no part index, effectively indestructible. Ends the beam.
    Terrain,
    /// A ship Frame (`class 0x1f`) or a fragment of one: never damaged, never stops the beam.
    Frame,
    /// A body with a valid `partIndex`: a part of a ship design.
    Part,
    /// A body with no part index that is **not** terrain: a missile, a shell, debris.
    FreeObject,
}

/// Classify one hit.
///
/// `is_part` — the body's `partIndex` resolves to `Stats` (`GetStats` is the game's own lookup and
/// returns null for a free body). `owner_zero` — `*(usize*)(body + 0xb8) == 0`. `health` — the
/// body's current health, which is what separates land from a wreck (see
/// [`body::TERRAIN_HEALTH_MIN`]). `class` — the body's class byte.
pub(crate) fn classify(is_part: bool, owner_zero: bool, health: f32, class: u8) -> HitKind {
    if is_part {
        HitKind::Part
    } else if owner_zero && health >= body::TERRAIN_HEALTH_MIN {
        HitKind::Terrain
    } else if class == body::FRAME_CLASS {
        HitKind::Frame
    } else {
        HitKind::FreeObject
    }
}

/// Whether this body is the one `impact.at` charges with the blast.
///
/// `first` is the body the beam entered; `last` is the body the chain stops on, which the loop
/// predicts from the same [`continues`] rule it will apply a moment later.
pub(crate) fn blast_target_here(at: BlastAt, first: bool, last: bool) -> bool {
    match at {
        BlastAt::First => first,
        BlastAt::Last => last,
    }
}

/// The summary line's word for where the blast went.
fn blast_where(at: BlastAt) -> &'static str {
    match at {
        BlastAt::First => "first",
        BlastAt::Last => "last",
    }
}

/// What the blast did, for the summary line. `Body_Explode` returning `false` means its own guard
/// refused (the body is not reflected) — the mod reports that as `FIZZLED` instead of staying silent.
pub(crate) fn blast_label(distance: f32, at: BlastAt, ran: bool) -> String {
    let end = blast_where(at);
    if ran {
        format!("yes at {distance:.1} u ({end})")
    } else {
        format!("FIZZLED at {distance:.1} u ({end}: Body_Explode refused)")
    }
}

/// Damage a laser of `laser` still carries at `distance`, or `None` when the beam has spent itself
/// there (D1; `damage <= 0` ends the beam, it does not continue at zero).
pub(crate) fn damage_at(laser: &ResolvedType, distance: f32) -> Option<f32> {
    if distance > laser.range {
        return None;
    }
    let damage = laser.damage - laser.damage_decay_per_unit * distance;
    (damage > 0.0).then_some(damage)
}

/// Whether the beam carries on behind a part of `category` that it has just damaged.
///
/// `pierce` off = the first part it reaches ends the beam (`damage first, then stop`); otherwise a
/// `stop_at` category ends it, and `max_parts` (when non-zero) ends it after that many damaged parts.
pub(crate) fn continues(laser: &ResolvedType, category: u32, damaged: usize) -> bool {
    if !laser.pierce {
        return false;
    }
    if category & laser.stop_mask != 0 {
        return false;
    }
    if laser.max_parts != 0 && damaged as i32 >= laser.max_parts {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Fade;

    fn laser(damage: f32, decay: f32, pierce: bool, stop_mask: u32, max_parts: i32) -> ResolvedType {
        ResolvedType {
            name: "test".to_owned(),
            range: 1000.0,
            damage,
            damage_decay_per_unit: decay,
            spread: 0.0,
            pierce,
            stop_mask,
            max_parts,
            through_terrain: false,
            ignore_armor_threshold: true,
            incendiary: 0.0,
            explosion: false,
            blast_at: BlastAt::First,
            explosive_power: 20.0,
            color: 0xFFFF_3322,
            width: 1.5,
            glow: true,
            glow_color: 0x33FF_3322,
            seconds: 0.08,
            fade: Fade::Linear,
        }
    }

    #[test]
    fn damage_decays_with_distance_and_ends_at_zero() {
        let pierce = laser(8.0, 0.0033, true, 64, 12);
        assert_eq!(damage_at(&pierce, 0.0), Some(8.0));
        let at_1000 = damage_at(&pierce, 1000.0).unwrap();
        assert!((at_1000 - 4.7).abs() < 1e-5, "8 - 3.3 = 4.7, got {at_1000}");
    }

    #[test]
    fn a_spent_beam_damages_nothing_past_the_configured_range() {
        let pierce = laser(8.0, 0.0033, true, 64, 12);
        assert_eq!(damage_at(&pierce, 1000.5), None, "hard cutoff at range");
        // decay that reaches zero well inside the range ends the beam there too
        let steep = laser(4.0, 1.0, true, 64, 12);
        assert!((damage_at(&steep, 3.9).unwrap() - 0.1).abs() < 1e-5);
        assert_eq!(damage_at(&steep, 4.0), None, "0 damage ends the beam, it does not continue");
        let zero = laser(0.0, 0.0, true, 64, 12);
        assert_eq!(damage_at(&zero, 0.0), None, "a 0-damage marker never damages");
    }

    #[test]
    fn a_non_piercing_type_stops_at_the_first_part() {
        let blunt = laser(12.0, 0.0, false, 64, 12);
        assert!(!continues(&blunt, 0, 1));
    }

    #[test]
    fn a_stop_category_and_the_part_limit_end_the_beam() {
        let pierce = laser(8.0, 0.0033, true, 64, 12);
        assert!(continues(&pierce, 0x100, 1), "an engine does not stop it");
        assert!(!continues(&pierce, 0x40, 1), "a block takes damage and stops it");
        assert!(!continues(&pierce, 0x100, 12), "max_parts reached");
        let unlimited = laser(8.0, 0.0, true, 0, 0);
        assert!(continues(&unlimited, 0x40, 1000), "0 = unlimited, empty mask");
    }

    /// A hit body is classified by its part index first, then land (ownerless + huge health), then
    /// its class — the order that keeps debris and missiles damageable and a frame out of the way.
    #[test]
    fn a_hit_is_a_part_a_frame_an_object_or_terrain() {
        // A part stays a part even if its owner link is gone: it is still a part of a design.
        assert_eq!(classify(true, true, 1.0, 0x0f), HitKind::Part);
        assert_eq!(classify(true, false, 5.0, 0x0f), HitKind::Part);
        // Land: ownerless, no part index, 100000 hp (`lnd_close_*` in the logs).
        assert_eq!(classify(false, true, 100_000.0, 0x0f), HitKind::Terrain);
        // Ownerless **debris** (the `Body_DieExplode` ruin fragments are 100 hp) and a missile
        // (10 hp, also spawned with a null owner) are free objects: damageable, and the beam passes.
        assert_eq!(classify(false, true, 100.0, 0x0f), HitKind::FreeObject);
        assert_eq!(classify(false, true, 10.0, 0x0f), HitKind::FreeObject);
        // `Frame_Constructor` writes 0x1f; a frame is skipped, never damaged.
        assert_eq!(classify(false, false, 100.0, 0x1f), HitKind::Frame);
        // Everything else is a free body: missile, shell, debris.
        assert_eq!(classify(false, false, 10.0, 0x0f), HitKind::FreeObject);
    }

    /// The blast belongs to the entry body (`first`) or the body the chain stops on (`last`), and
    /// runs as soon as that body has been damaged.
    #[test]
    fn the_blast_runs_on_the_end_the_config_names() {
        assert!(blast_target_here(BlastAt::First, true, false));
        assert!(!blast_target_here(BlastAt::First, false, true));
        assert!(blast_target_here(BlastAt::Last, false, true));
        assert!(!blast_target_here(BlastAt::Last, true, false));
        assert_eq!(blast_where(BlastAt::First), "first");
        assert_eq!(blast_where(BlastAt::Last), "last");
    }

    /// The summary must say whether `Body_Explode` actually ran; the blast is spawned exactly once, at
    /// the configured `explosive_power` (the 0.7.2 4× retry is gone in 0.8.2).
    #[test]
    fn the_blast_label_reports_the_blast_and_the_fizzle() {
        assert_eq!(blast_label(524.7, BlastAt::First, true), "yes at 524.7 u (first)");
        assert_eq!(blast_label(10.0, BlastAt::Last, true), "yes at 10.0 u (last)");
        assert!(blast_label(10.0, BlastAt::First, false).starts_with("FIZZLED"));
    }

    /// Phase 5.5: one rotation of the firing direction, vanilla's unit (radians). `0` is the exact
    /// identity; the length is preserved (a shot keeps its reach); ±90° turns the vector; and a
    /// positive/negative roll really does point the shot the two ways.
    #[test]
    fn spread_rotates_the_direction_without_changing_its_length() {
        let d = Vec2 { x: 1.0, y: 0.0 };
        let same = rotate(d, 0.0);
        assert_eq!((same.x, same.y), (d.x, d.y), "0 is bit-for-bit the same direction");

        let right = rotate(d, std::f32::consts::FRAC_PI_2);
        assert!(right.x.abs() < 1e-6 && (right.y - 1.0).abs() < 1e-6, "{right:?}");
        let left = rotate(d, -std::f32::consts::FRAC_PI_2);
        assert!((left.y + 1.0).abs() < 1e-6, "{left:?}");

        // Vanilla's own magnitudes: 0.05 rad (37/57/100 mm) and 0.02 rad (130 mm+).
        for angle in [0.05f32, -0.05, 0.02, -0.02, 3.1416] {
            let turned = rotate(Vec2 { x: 0.6, y: 0.8 }, angle);
            let length = (turned.x * turned.x + turned.y * turned.y).sqrt();
            assert!((length - 1.0).abs() < 1e-5, "|d| moved with {angle}: {length}");
        }
        let up = rotate(d, 0.05);
        let down = rotate(d, -0.05);
        assert!(up.y > 0.0 && down.y < 0.0, "the sign picks the side");
        assert!(up.x < 1.0, "the aim moves off the axis");
    }
}
