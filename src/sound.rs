//! Phase 4 — the launch sound.
//!
//! A mapped ammo no longer calls `FireShell`, so `Body_InitializeCannonShell` (`0x140052a80`) never
//! runs — and *that* is where a cannon's firing sound is played. Every laser shot was silent. This
//! module plays the same sound vanilla would have played, with the same variant selection, from the
//! ammo record the shot already has.
//!
//! # The recipe, read out of the 1.163 program on 2026-11-23
//!
//! `Body_InitializeCannonShell` ends with two nested branches and three `playSFXSound_` calls:
//!
//! ```c
//! if (player) { if (ZOOM > 3) goto far; else player-close }   // 0x140052dd3, call 0x140052e71
//! else        { if (ZOOM > 3) goto far; else enemy-close  }   // 0x140052e7b, call 0x140052f11
//! far:                                                        // call 0x140052fa4
//!
//! player-close: ammo+0xf0, 3 variants, kind 0x0b, node = *(usize*)0x147fc6f88 (GlobalNode_)
//! enemy-close:  ammo+0x110, 4 variants, kind 0x02, node = the shot's Body
//! far:          ammo+0x130, 4 variants, kind 0x02, node = the shot's Body
//! ```
//!
//! In every branch the name is
//! `Sound_BuildName(<the field's buffer>, 1, variants, "_")` — `useEngineNumber != 0` makes the game
//! draw a random variant (`HGE` vtable `+0x108`), and the result is `<set>_<2-digit number>` in a
//! global string that must be used immediately (`0x1401b1fb0`, verified by decompile, together with
//! the global buffer at `0x1403c16b0`).
//!
//! The volume is `HGE.Random_Float(75.0, 100.0)` (vtable `+0x110`, `RCX = HGE`, `XMM1 = min`,
//! `XMM2 = max` — read from the call site, not guessed; the two bounds are the globals at
//! `0x140354850` / `0x1403524e0`, both confirmed by reading them: `75.0` and `100.0`). Only the
//! player-close branch multiplies it by `0.5`, and only when `FUN_140073340(owner)` is non-zero:
//!
//! ```text
//! 140073340  CMP  dword ptr [RCX+0x48], 0        ; the owner node's active flag
//! 140073344  JZ   0x140073357                    ; -> AL = 0
//! 140073346  MOVSS XMM0, [0x140354834]           ; 40.0
//! 14007334e  COMISS XMM0, [RCX+0x15f8]           ; 40.0 <= creature+0x15f8 ?
//! 140073355  JBE  0x140073359                    ; -> return with AL untouched (vanilla quirk)
//! 140073357  XOR  AL, AL                         ; false
//! 140073359  RET
//! ```
//!
//! Vanilla keeps `XMM7 = 0.5` when the call returns non-zero and loads `1.0` (`0x1403524d8`)
//! otherwise. The mod calls the same function and applies the same factor — the AL quirk above is
//! vanilla's own, so the outcome is whatever the game decides for that owner.
//!
//! # What is not done here
//!
//! The four arguments `playSFXSound_` gets are exactly vanilla's: both float pointers are `NULL`,
//! `loop = 0`, no option volume, no pan — a plain one-shot that never needs a BASS channel. There
//! is **no rate limit and no clamp** (user decision, 2026-11-23: vanilla plays one sound per shot
//! with no throttle, so this mod does too). The node of the two `kind 2` branches is the firing
//! gun's `Body*` instead of the freshly created shell body, because this mod creates no shell; the
//! gun is at the same place at t = 0 and is a valid `Node`.
//!
//! The mod's own entry bytes check is the one `hf_gun_mechanics` uses: if `playSFXSound_` does not
//! start with `48 8b c4 48 89 58 08 48`, the sound is reported once and skipped. The **shot** is
//! never affected — [`crate::damage::try_fire`] calls this from `catch_unwind` and ignores the
//! result.

use std::{
    collections::BTreeSet,
    sync::{Mutex, Once, PoisonError},
};

use crate::{ammo::Ammo, mem, readable, readable_extent};

/// `playSFXSound_` — the game's own sound player (`hf_notes/docs/sound.md`).
const PLAY_SFX: usize = 0x1_402e_30e0;
const _: () = assert!(PLAY_SFX == 0x1402e30e0);
/// Its first eight bytes: the build check, so a wrong game build is reported instead of called.
const PLAY_SFX_ENTRY: [u8; 8] = [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48];

/// `char *Sound_BuildName(char *soundName, uint number, int useEngineNumber, char *nameInfix)`.
/// Formats `<soundName><infix><2-digit number>` into its global string and returns that `c_str`.
const BUILD_NAME: usize = 0x1_401b_1fb0;
const _: () = assert!(BUILD_NAME == 0x1401b1fb0);
/// The infix vanilla passes (`"_"` at `0x140352954`).
const INFIX: &[u8] = b"_\0";

/// `ZOOM_LEVEL` (`int`): vanilla treats `> 3` as "far".
const ZOOM_LEVEL: usize = 0x1_403c_11cc;
const _: () = assert!(ZOOM_LEVEL == 0x1403c11cc);
/// The sound node the stock radio and the player-close cannon branch use — read the pointer.
const GLOBAL_NODE: usize = 0x1_47fc_6f88;
const _: () = assert!(GLOBAL_NODE == 0x147fc6f88);

/// The `HGE` interface object (a pointer stored here) and its `Random_Float(min, max)` slot.
const HGE: usize = 0x1_43a1_19e0;
const _: () = assert!(HGE == 0x143a119e0);
const HGE_RANDOM_FLOAT: usize = 0x110;

/// The volume bounds `Body_InitializeCannonShell` reads (`75.0` / `100.0`).
const VOLUME_MIN: usize = 0x1_4035_4850;
const _: () = assert!(VOLUME_MIN == 0x140354850);
const VOLUME_MAX: usize = 0x1_4035_24e0;
const _: () = assert!(VOLUME_MAX == 0x1403524e0);
const VOLUME_FALLBACK: (f32, f32) = (75.0, 100.0);

/// The owner test whose non-zero result halves the player-close volume.
const OWNER_FACTOR: usize = 0x1_4007_3340;
const _: () = assert!(OWNER_FACTOR == 0x140073340);

/// `Profile *Get_Profile_(void)` and `bool Profile_IsPlayerShip(Profile *, Creature *)`.
const GET_PROFILE: usize = 0x1_4029_2a90;
const _: () = assert!(GET_PROFILE == 0x140292a90);
const IS_PLAYER_SHIP: usize = 0x1_4029_3920;
const _: () = assert!(IS_PLAYER_SHIP == 0x140293920);

/// `Ammo` sound fields (`EscadraString` each, 0x20 bytes apart). `shell_in (+0xd0)` is the
/// magazine-load sound and is **not** part of firing.
pub(crate) const SHELL_OUT: usize = 0xf0;
pub(crate) const SHELL_ENEMY: usize = 0x110;
pub(crate) const SHELL_FAR: usize = 0x130;

/// Vanilla's variant counts: `shell_out` has three clips, the distant/enemy sets four.
const SHELL_OUT_VARIANTS: u32 = 3;
const DISTANT_VARIANTS: u32 = 4;

/// `playSFXSound_` kinds: the radio/global node category, and the shot's own body.
const KIND_GLOBAL: i32 = 0xb;
const KIND_SHELL: i32 = 2;

/// A built name is short; this bounds the read of the game's global buffer.
const NAME_MAX: usize = 128;

/// Image base and a generous span, to tell a vtable slot from a data address (same rule as
/// [`crate::body::plausible`]).
const IMAGE_BASE: usize = 0x1_4000_0000;
const IMAGE_SPAN: usize = 0x1000_0000;

/// `playSFXSound_`'s 13 arguments, exactly as `hf_gun_mechanics\src\sound.rs` proved them in play
/// (and as the vanilla cannon calls pass them: both float pointers `NULL`, no loop).
type FnPlaySfx = unsafe extern "C" fn(
    i32,       // kind
    *const u8, // name (NUL-terminated, used immediately)
    usize,     // node
    f32,       // base volume
    f32,       // pitch
    *mut f32,  // volume multiplier
    *mut f32,  // pitchScale
    u8,        // loop flag
    i32,       // option-volume selector
    u8,
    u8,
    f32, // pan
    u8,
) -> f32;

type FnBuildName = unsafe extern "C" fn(*const u8, u32, u32, *const u8) -> *mut u8;
/// `HGE::Random_Float(this, min, max) -> float` (`RCX`, `XMM1`, `XMM2`).
type FnRandomFloat = unsafe extern "C" fn(usize, f32, f32) -> f32;
type FnOwnerFactor = unsafe extern "C" fn(usize) -> u8;
type FnGetProfile = unsafe extern "C" fn() -> usize;
type FnIsPlayerShip = unsafe extern "C" fn(usize, usize) -> u8;

/// Which of `Body_InitializeCannonShell`'s three sound branches a shot takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Branch {
    /// The local player, zoomed in: `shell_out`, `kind 0xb`, global node, volume may be halved.
    PlayerClose,
    /// Another ship, zoomed in: `shell_enemy`, `kind 2`, the gun's body.
    EnemyClose,
    /// Any ship, zoomed out: `shell_far`, `kind 2`, the gun's body.
    Far,
}

impl Branch {
    fn name(self) -> &'static str {
        match self {
            Branch::PlayerClose => "player close",
            Branch::EnemyClose => "enemy close",
            Branch::Far => "far",
        }
    }

    fn key(self) -> u8 {
        match self {
            Branch::PlayerClose => 0,
            Branch::EnemyClose => 1,
            Branch::Far => 2,
        }
    }
}

/// Vanilla's split: `zoom > 3` is far whatever the side is, otherwise player-close / enemy-close.
pub(crate) fn branch_for(player: bool, zoom: i32) -> Branch {
    if zoom > 3 {
        Branch::Far
    } else if player {
        Branch::PlayerClose
    } else {
        Branch::EnemyClose
    }
}

/// The ammo field and variant count of one branch.
pub(crate) fn field_for(branch: Branch) -> (usize, u32) {
    match branch {
        Branch::PlayerClose => (SHELL_OUT, SHELL_OUT_VARIANTS),
        Branch::EnemyClose => (SHELL_ENEMY, DISTANT_VARIANTS),
        Branch::Far => (SHELL_FAR, DISTANT_VARIANTS),
    }
}

/// The volume vanilla would have passed: the random 75..100, halved on the player-close branch when
/// the owner test says so. A non-finite roll falls back to the midpoint; nothing else is clamped.
pub(crate) fn volume(random: f32, owner_halves: bool) -> f32 {
    let random = if random.is_finite() {
        random
    } else {
        (VOLUME_FALLBACK.0 + VOLUME_FALLBACK.1) * 0.5
    };
    if owner_halves {
        random * 0.5
    } else {
        random
    }
}

fn in_image(address: usize) -> bool {
    (IMAGE_BASE..IMAGE_BASE + IMAGE_SPAN).contains(&address)
}

static REPORTED: Once = Once::new();

/// Entry bytes of `playSFXSound_`, so a wrong game build is reported instead of called.
fn site_looks_right() -> bool {
    let right = readable(PLAY_SFX, PLAY_SFX_ENTRY.len()) && {
        // SAFETY: the range was just proved readable.
        let current =
            unsafe { std::slice::from_raw_parts(PLAY_SFX as *const u8, PLAY_SFX_ENTRY.len()) };
        current == PLAY_SFX_ENTRY
    };
    if !right {
        REPORTED.call_once(|| {
            log::warn!(
                "hf_hitscan: playSFXSound_ entry bytes differ; the launch sound is off (the shot \
                 is unaffected)"
            )
        });
    }
    right
}

/// `Profile_IsPlayerShip(Get_Profile_(), gun_owner)`.
fn is_player_ship(owner: usize) -> bool {
    if owner < 0x1_0000 || !readable(GET_PROFILE, 1) || !readable(IS_PLAYER_SHIP, 1) {
        return false;
    }
    // SAFETY: both addresses are the 1.163 functions (checked at compile time), called with the
    // arguments vanilla passes.
    let get_profile: FnGetProfile = unsafe { std::mem::transmute(GET_PROFILE) };
    let profile = unsafe { get_profile() };
    if profile < 0x1_0000 {
        return false;
    }
    let is_player: FnIsPlayerShip = unsafe { std::mem::transmute(IS_PLAYER_SHIP) };
    unsafe { is_player(profile, owner) != 0 }
}

/// `FUN_140073340(gun_owner)`: vanilla halves the player-close volume when it is non-zero.
fn owner_halves(owner: usize) -> bool {
    if owner < 0x1_0000 || !readable(OWNER_FACTOR, 1) {
        return false;
    }
    // SAFETY: the function is the 25-byte 1.163 leaf read for this build.
    let factor: FnOwnerFactor = unsafe { std::mem::transmute(OWNER_FACTOR) };
    unsafe { factor(owner) != 0 }
}

/// `HGE::Random_Float(min, max)`, called exactly as `Body_InitializeCannonShell` calls it.
///
/// Phase 5.5 reuses it for cannon scatter (`damage::try_fire`), so the shot's angle comes from the
/// game's own generator, exactly like vanilla's (`Body_InitializeCannonShell`'s 13th argument).
pub(crate) fn random_float(min: f32, max: f32) -> Option<f32> {
    let hge = mem::usize_at(HGE)?;
    let vtable = mem::usize_at(hge)?;
    let entry = mem::usize_at(vtable + HGE_RANDOM_FLOAT)?;
    if !in_image(entry) {
        return None;
    }
    // SAFETY: the slot is the HGE interface's `Random_Float`, whose ABI was read at the call site.
    let random: FnRandomFloat = unsafe { std::mem::transmute(entry) };
    let value = unsafe { random(hge, min, max) };
    value.is_finite().then_some(value)
}

/// The NUL-terminated name behind `pointer`, bounded to [`NAME_MAX`] bytes of the game's own global
/// buffer. Used for the log line only; the pointer itself is what is played.
fn c_name(pointer: *const u8) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let address = pointer as usize;
    let extent = readable_extent(address, NAME_MAX);
    if extent == 0 {
        return String::new();
    }
    // SAFETY: `readable_extent` proved the range readable.
    let bytes = unsafe { std::slice::from_raw_parts(pointer, extent) };
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// What one ammo resolved to, once per (item, branch) — the play check has to be verifiable from
/// the log even when the audio device is off.
static LOGGED: Mutex<BTreeSet<(String, u8)>> = Mutex::new(BTreeSet::new());

fn first_time(item: &str, branch: Branch) -> bool {
    let mut logged = LOGGED.lock().unwrap_or_else(PoisonError::into_inner);
    logged.insert((item.to_ascii_lowercase(), branch.key()))
}

/// Play the launch sound vanilla's `Body_InitializeCannonShell` would have played for this shot.
///
/// Called by [`crate::damage::try_fire`] on the handled path only — a shot forwarded to vanilla
/// `FireShell` plays its own sound there and must not be doubled.
///
/// `gun` is the firing part (its `Body*` is the node of the two distant branches) and `owner` is
/// `*(gun + 0xb8)`, the creature the player test and the volume factor use.
pub(crate) fn launch(ammo: &Ammo, gun: usize, owner: usize) {
    if !site_looks_right() {
        return;
    }
    let player = is_player_ship(owner);
    let zoom = mem::i32_at(ZOOM_LEVEL).unwrap_or(0);
    let branch = branch_for(player, zoom);
    let (field, variants) = field_for(branch);
    // One line per (ammo, branch) whatever happens below: the play check must be readable from the
    // log, but dozens of lines per second would be a frame cost (see the README's cost section).
    let log_once = first_time(&ammo.item, branch);

    // The field is passed as the game's own buffer address — no copy, no `CString`; a name that is
    // not NUL-terminated inside the record is refused rather than scanned.
    let Some(buffer) = ammo.sound_name(field) else {
        if log_once {
            log::warn!(
                "hf_hitscan: ammo {:?}: its {} sound field is unreadable; the shot has no launch \
                 sound",
                ammo.item,
                branch.name()
            );
        }
        return;
    };

    let (min, max) = (
        mem::f32_at(VOLUME_MIN).unwrap_or(VOLUME_FALLBACK.0),
        mem::f32_at(VOLUME_MAX).unwrap_or(VOLUME_FALLBACK.1),
    );
    let roll = random_float(min, max).unwrap_or((min + max) * 0.5);
    let value = volume(roll, branch == Branch::PlayerClose && owner_halves(owner));

    let node = match branch {
        Branch::PlayerClose => mem::usize_at(GLOBAL_NODE).unwrap_or(0),
        Branch::EnemyClose | Branch::Far => gun,
    };
    if node < 0x1_0000 {
        return;
    }
    let kind = match branch {
        Branch::PlayerClose => KIND_GLOBAL,
        Branch::EnemyClose | Branch::Far => KIND_SHELL,
    };

    // SAFETY: the buffer came out of a validated ammo record and the infix is this module's own
    // NUL-terminated constant. The returned pointer is the c_str of a global string that is only
    // valid until the next call, so it is used immediately and never stored.
    let build: FnBuildName = unsafe { std::mem::transmute(BUILD_NAME) };
    // SAFETY: `site_looks_right` proved the entry point and the declaration is `hf_gun_mechanics`'s
    // (proved in play).
    let play: FnPlaySfx = unsafe { std::mem::transmute(PLAY_SFX) };
    let mut built = String::new();
    let mut result = -1.0f32;
    let mut attempts = 0u32;
    // Attempt 0 is vanilla's own call: `useEngineNumber != 0` makes the game pick a random variant
    // of the set. Some sets have fewer clips than the branch asks for — `shell_out_small_far` has
    // three files while the far branch asks for four (`ITEM_AMMO_130MM_INC` names it) — so if that
    // pick is not in `sound.res` the remaining variants are tried by number instead of playing
    // nothing. `playSFXSound_` returns -1 for a name it cannot resolve.
    for attempt in 0..=variants {
        // SAFETY: `buffer` is a NUL-terminated name inside a validated record; `Sound_BuildName`
        // copies it into its own global string and the pointer is used before the next call.
        let name = unsafe {
            if attempt == 0 {
                build(buffer as *const u8, 1, variants, INFIX.as_ptr())
            } else {
                build(buffer as *const u8, attempt, 0, INFIX.as_ptr())
            }
        };
        if name.is_null() {
            break;
        }
        attempts += 1;
        built = c_name(name);
        // SAFETY: `name` is the game's own freshly built string and `node` is a live Body.
        result = unsafe {
            play(
                kind,
                name,
                node,
                value,
                1.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                0.0,
                0,
            )
        };
        if result > 0.0 {
            break;
        }
    }
    if attempts == 0 {
        if log_once {
            log::warn!(
                "hf_hitscan: ammo {:?}: Sound_BuildName returned null; the shot has no launch \
                 sound",
                ammo.item
            );
        }
        return;
    }

    if log_once {
        log::info!(
            "hf_hitscan: ammo {:?}: launch sound {built:?} (kind {kind:#x}, {}, zoom {zoom}, \
             volume {value:.1}, {result} = {}{tries})",
            ammo.item,
            branch.name(),
            if result > 0.0 { "queued" } else { "dropped" },
            tries = if attempts > 1 {
                format!(", {attempts} variants tried")
            } else {
                String::new()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_addresses_are_the_ones_read_from_the_1_163_program() {
        assert_eq!(PLAY_SFX, 0x1402e30e0);
        assert_eq!(BUILD_NAME, 0x1401b1fb0);
        assert_eq!(ZOOM_LEVEL, 0x1403c11cc);
        assert_eq!(GLOBAL_NODE, 0x147fc6f88);
        assert_eq!(HGE, 0x143a119e0);
        assert_eq!(VOLUME_MIN, 0x140354850);
        assert_eq!(VOLUME_MAX, 0x1403524e0);
        assert_eq!(OWNER_FACTOR, 0x140073340);
        assert_eq!(GET_PROFILE, 0x140292a90);
        assert_eq!(IS_PLAYER_SHIP, 0x140293920);
        assert_eq!(INFIX, b"_\0");
    }

    /// The three branches of `Body_InitializeCannonShell`: far wins at `zoom > 3` for both sides,
    /// otherwise the side picks the set.
    #[test]
    fn the_branch_matches_vanillas_nested_test() {
        for zoom in [-3, 0, 1, 2, 3] {
            assert_eq!(branch_for(true, zoom), Branch::PlayerClose, "player, zoom {zoom}");
            assert_eq!(branch_for(false, zoom), Branch::EnemyClose, "enemy, zoom {zoom}");
        }
        for zoom in [4, 5, 40] {
            assert_eq!(branch_for(true, zoom), Branch::Far, "player, zoom {zoom}");
            assert_eq!(branch_for(false, zoom), Branch::Far, "enemy, zoom {zoom}");
        }
    }

    /// `shell_out` gets three variants, `shell_enemy` and `shell_far` four — and the field offsets
    /// are the `ADD RDI,0xNN` immediates in the three branches.
    #[test]
    fn the_fields_and_variant_counts_are_vanillas() {
        assert_eq!(field_for(Branch::PlayerClose), (0xf0, 3));
        assert_eq!(field_for(Branch::EnemyClose), (0x110, 4));
        assert_eq!(field_for(Branch::Far), (0x130, 4));
        assert_eq!(SHELL_OUT + 0x20, SHELL_ENEMY, "the record's fields are 0x20 apart");
        assert_eq!(SHELL_ENEMY + 0x20, SHELL_FAR);
    }

    /// The owner factor halves the volume; nothing else touches it, and a bad roll is the midpoint.
    #[test]
    fn the_volume_is_the_roll_and_may_be_halved() {
        assert_eq!(volume(75.0, false), 75.0);
        assert_eq!(volume(100.0, false), 100.0);
        assert_eq!(volume(80.0, true), 40.0);
        assert_eq!(volume(f32::NAN, false), 87.5);
        assert_eq!(volume(f32::NAN, true), 43.75);
        assert_eq!(volume(1000.0, false), 1000.0, "no clamp: the game's own roll is used");
    }

    #[test]
    fn the_image_check_rejects_a_slot_outside_the_module() {
        assert!(in_image(PLAY_SFX), "playSFXSound_ is inside the image");
        assert!(in_image(BUILD_NAME), "Sound_BuildName is inside the image");
        assert!(!in_image(0x1_0000), "a heap/stack address is not a vtable slot");
        assert!(!in_image(IMAGE_BASE + IMAGE_SPAN), "past the image span");
    }
}
