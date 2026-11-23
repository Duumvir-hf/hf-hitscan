//! The ammo table and the fields this mod reads.
//!
//! `0x143a13be0` is a **`*mut *mut Ammo`** whose first word `ammo-extended` replaces, so the
//! pointer is re-read every shot (H3) and never cached across a battle start. Stride `0x188`.
//!
//! Fields used here, all verified in play on 2026-11-23 (phase 1 log):
//!
//! | offset | type | meaning |
//! |---|---|---|
//! | `+0x08` | `EscadraString` | `item_name` — the `shells` key |
//! | `+0xcc` | `u32` | `ammo-extended`'s `behavior` (the patched spawn id) |
//! | `+0x150` | `i32` | projectile logic id (`100` plain shell, `160` prox-fuze cannon shell, …) |
//! | `+0x154` | `i32` | `ammo-extended`'s `caliber` (5/10/15/20/25 … for 37/57/100/130 mm) |
//!
//! Measured: `ITEM_AMMO_130MM` → `+0x154 = 0x14`, `ITEM_AMMO_37MM` → `+0x154 = 5`, both with
//! `+0xcc = 0`, `+0x150 = 100`.
//!
//! # There is no "is this really a cannon shell" test — on purpose
//!
//! Until 0.7.0 the mod refused a mapped ammo whose `+0x154` was not one of `FireShell`'s original
//! cannon cases, or whose logic id was a rocket's, to keep a typo from turning a missile into a
//! laser. `+0x154` is `ammo-extended`'s **caliber**, though: a modded round may carry any value
//! (the user's `ITEM_AMMO_THRANGXNON` is a 20 mm "laser" with caliber `95`), so the rule rejected
//! exactly the custom rounds this mod exists for. The `shells` map is already the explicit
//! opt-in (decision D2: no gun key, no `default` type): **an ammo the config names is a laser,
//! whatever its ids say.** The ids are still logged, once per ammo, for diagnosis.

use std::{
    collections::BTreeSet,
    sync::{Mutex, PoisonError},
};

use crate::{
    mem::{i32_at, string_at, usize_at, u32_at, STRING_MAX},
    read, readable,
};

/// `*mut *mut Ammo`: word 0 = begin, word 1 = end.
pub(crate) const TABLE: usize = 0x1_43a1_3be0;
const _: () = assert!(TABLE == 0x143a13be0);
pub(crate) const STRIDE: usize = 0x188;
const _: () = assert!(STRIDE == 0x188);
pub(crate) const ITEM_NAME: usize = 0x08;
pub(crate) const BEHAVIOR: usize = 0xcc;
pub(crate) const LOGIC: usize = 0x150;
pub(crate) const DISPATCH: usize = 0x154;

/// A live ammo record, validated for this shot.
pub(crate) struct Ammo {
    pub(crate) record: usize,
    pub(crate) item: String,
}

impl Ammo {
    /// The record at `position` in the current table, or `None` when the position is out of range
    /// or the record is not readable.
    pub(crate) fn at(position: i32) -> Option<Self> {
        if !(0..=4096).contains(&position) {
            return None;
        }
        let begin = usize_at(TABLE)?;
        if begin == 0 {
            return None;
        }
        let record = begin.checked_add(position as usize * STRIDE)?;
        if !readable(record, STRIDE) {
            return None;
        }
        let item = string_at(record + ITEM_NAME)?;
        Some(Self { record, item })
    }

    pub(crate) fn dispatch(&self) -> i32 {
        i32_at(self.record + DISPATCH).unwrap_or(-1)
    }

    pub(crate) fn behavior(&self) -> u32 {
        u32_at(self.record + BEHAVIOR).unwrap_or(0)
    }

    pub(crate) fn logic(&self) -> i32 {
        i32_at(self.record + LOGIC).unwrap_or(-1)
    }

    /// The NUL-terminated buffer of the `EscadraString` at `offset`, ready for `Sound_BuildName`.
    ///
    /// The game's own `Body_InitializeCannonShell` takes the buffer exactly this way
    /// (`CMP [field+0x18],0x10` / inline else heap pointer at `+0`), so no copy and no `CString` is
    /// made. `None` when the field is not a sane string: the shot still fires, it only has no
    /// launch sound.
    pub(crate) fn sound_name(&self, offset: usize) -> Option<usize> {
        let field = self.record.checked_add(offset)?;
        let length = usize_at(field + 0x10)?;
        let capacity = usize_at(field + 0x18)?;
        if length > STRING_MAX {
            return None;
        }
        let base = if capacity >= 16 { usize_at(field)? } else { field };
        if base < 0x1_0000 || !readable(base, length + 1) {
            return None;
        }
        // SAFETY: `readable` proved `length + 1` bytes; MSVC strings are NUL-terminated at `length`.
        // A record that is not is refused here instead of letting `Sound_BuildName` scan off the end.
        (unsafe { read::<u8>(base + length) } == 0).then_some(base)
    }
}

static LOGGED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Log what one ammo resolved to, once per `item_name`.
///
/// `mapped` is the configured type name when the ammo is in `shells`, `None` when it is not. The
/// ids are printed for diagnosis only — since 0.7.0 they do not decide anything (see the module
/// docs).
pub(crate) fn note_resolution(ammo: &Ammo, mapped: Option<&str>) {
    if !first_time(&ammo.item) {
        return;
    }
    let (dispatch, behavior, logic) = ids(ammo);
    match mapped {
        Some(mapped) => log::info!(
            "hf_hitscan: ammo {:?}: +0x154 {dispatch} ({dispatch:#x}), +0xcc {behavior:#x}, \
             +0x150 {logic} -> laser type {mapped:?}",
            ammo.item
        ),
        None => log::info!(
            "hf_hitscan: ammo {:?}: +0x154 {dispatch} ({dispatch:#x}), +0xcc {behavior:#x}, \
             +0x150 {logic} -> not in `shells`; its shots stay vanilla shells",
            ammo.item
        ),
    }
}

fn ids(ammo: &Ammo) -> (i32, u32, i32) {
    (ammo.dispatch(), ammo.behavior(), ammo.logic())
}

fn first_time(item: &str) -> bool {
    let mut logged = LOGGED.lock().unwrap_or_else(PoisonError::into_inner);
    logged.insert(item.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_addresses_are_the_ones_in_the_roadmap() {
        assert_eq!(TABLE, 0x143a13be0);
        assert_eq!(STRIDE, 0x188);
    }

    #[test]
    fn an_out_of_range_position_is_refused_before_any_read() {
        assert!(Ammo::at(-1).is_none());
        assert!(Ammo::at(4097).is_none());
    }

    /// The launch-sound fields are handed to `Sound_BuildName` the way the game takes them: inline
    /// while the capacity is below 16, through the heap pointer otherwise (`CMP [field+0x18],0x10`).
    #[test]
    fn a_sound_field_is_handed_over_as_its_own_nul_terminated_buffer() {
        let mut record = vec![0u8; STRIDE];
        let base = record.as_ptr() as usize;

        // Inline form: "shell\0", capacity 15.
        record[0xf0..0xf6].copy_from_slice(b"shell\0");
        record[0xf0 + 0x10..0xf0 + 0x18].copy_from_slice(&5usize.to_le_bytes());
        record[0xf0 + 0x18..0xf0 + 0x20].copy_from_slice(&15usize.to_le_bytes());
        let ammo = Ammo { record: base, item: String::new() };
        assert_eq!(ammo.sound_name(0xf0), Some(base + 0xf0));

        // Heap form: capacity 16 or more, the pointer at +0 decides.
        let heap = b"shell_out_med\0".to_vec();
        let heap_base = heap.as_ptr() as usize;
        record[0x110..0x118].copy_from_slice(&heap_base.to_le_bytes());
        record[0x110 + 0x10..0x110 + 0x18].copy_from_slice(&13usize.to_le_bytes());
        record[0x110 + 0x18..0x110 + 0x20].copy_from_slice(&31usize.to_le_bytes());
        assert_eq!(ammo.sound_name(0x110), Some(heap_base));

        // An empty field is still a valid (one-byte) buffer: vanilla builds "_01" from it.
        assert_eq!(ammo.sound_name(0x130), Some(base + 0x130));

        // A field that is not NUL-terminated at its length is refused, not scanned.
        record[0xf0..0xf4].copy_from_slice(b"abcd");
        record[0xf0 + 0x10..0xf0 + 0x18].copy_from_slice(&4usize.to_le_bytes());
        record[0xf0 + 4] = b'X';
        assert_eq!(ammo.sound_name(0xf0), None);

        // An absurd length is a bad record, not a name.
        record[0xf0 + 0x10..0xf0 + 0x18]
            .copy_from_slice(&(STRING_MAX as usize + 1).to_le_bytes());
        assert_eq!(ammo.sound_name(0xf0), None);
    }
}
