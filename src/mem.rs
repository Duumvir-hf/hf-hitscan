//! Guarded reads. Every pointer the mod follows is range-checked with
//! [`crate::readable`] before it is dereferenced, unless a caller has already proved a wider
//! extent readable in this same call (see [`crate::readable_extent`]) and reads the field with
//! [`crate::read`] instead.
//!
//! The phase 1 probe measured 3.0–5.6 ms per shot, almost all of it these checks (4–5
//! `VirtualQuery` per body). Phase 2 keeps exactly one query per body (the body is a single
//! `0xa40` allocation, so proving `crate::body::SCAN_EXTENT` bytes readable proves every field the
//! scan reads) and trusts what Box2D owns — see `ray.rs`.

use std::mem::size_of;

use crate::{read, readable, Vec2};

/// An `EscadraString` longer than this is a bad pointer, not a name.
pub(crate) const STRING_MAX: usize = 256;

pub(crate) fn u32_at(address: usize) -> Option<u32> {
    readable(address, size_of::<u32>()).then(|| unsafe { read::<u32>(address) })
}

pub(crate) fn i32_at(address: usize) -> Option<i32> {
    readable(address, size_of::<i32>()).then(|| unsafe { read::<i32>(address) })
}

pub(crate) fn usize_at(address: usize) -> Option<usize> {
    readable(address, size_of::<usize>()).then(|| unsafe { read::<usize>(address) })
}

pub(crate) fn vec2_at(address: usize) -> Option<Vec2> {
    readable(address, size_of::<Vec2>()).then(|| unsafe { read::<Vec2>(address) })
}

pub(crate) fn f32_at(address: usize) -> Option<f32> {
    readable(address, size_of::<f32>()).then(|| unsafe { read::<f32>(address) })
}

/// MSVC `std::string` (`EscadraString`): size at `+0x10`, capacity at `+0x18`, inline buffer at
/// `+0` while the capacity is below 16, heap pointer at `+0` otherwise.
pub(crate) fn string_at(field: usize) -> Option<String> {
    let length = usize_at(field + 0x10)?;
    let capacity = usize_at(field + 0x18)?;
    if length == 0 {
        return Some(String::new());
    }
    if length > STRING_MAX {
        return None;
    }
    let base = if capacity >= 16 {
        usize_at(field)?
    } else {
        field
    };
    if base == 0 || !readable(base, length) {
        return None;
    }
    // SAFETY: the range was just proved readable.
    let bytes = unsafe { std::slice::from_raw_parts(base as *const u8, length) };
    Some(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_string_reader_handles_both_msvc_forms() {
        // Inline ("short") form: capacity < 16, the bytes live in the object itself.
        let mut inline = [0u8; 0x20];
        inline[0..4].copy_from_slice(b"ABCD");
        inline[0x10..0x18].copy_from_slice(&4usize.to_le_bytes());
        inline[0x18..0x20].copy_from_slice(&15usize.to_le_bytes());
        assert_eq!(string_at(inline.as_ptr() as usize).as_deref(), Some("ABCD"));

        // Empty: length 0 is an empty name, not an error.
        let empty = [0u8; 0x20];
        assert_eq!(string_at(empty.as_ptr() as usize).as_deref(), Some(""));

        // Heap form: capacity >= 16, pointer at +0.
        let heap = b"ITEM_AMMO_130MM_AP".to_vec();
        let mut long = [0u8; 0x20];
        long[0..8].copy_from_slice(&(heap.as_ptr() as usize).to_le_bytes());
        long[0x10..0x18].copy_from_slice(&heap.len().to_le_bytes());
        long[0x18..0x20].copy_from_slice(&heap.len().to_le_bytes());
        assert_eq!(
            string_at(long.as_ptr() as usize).as_deref(),
            Some("ITEM_AMMO_130MM_AP")
        );
    }

    #[test]
    fn an_absurd_length_is_a_bad_pointer_not_a_name() {
        let mut field = [0u8; 0x20];
        field[0x10..0x18].copy_from_slice(&(STRING_MAX as usize + 1).to_le_bytes());
        field[0x18..0x20].copy_from_slice(&15usize.to_le_bytes());
        assert_eq!(string_at(field.as_ptr() as usize), None);
    }
}
