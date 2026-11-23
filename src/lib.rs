//! HighFleet Steam 1.163 — **`hf_hitscan`**, phase 4 (damage + beam + launch sound).
//!
//! The finished mod turns configured cannon ammo into a hitscan laser: no shell body is spawned,
//! a ray is cast from the muzzle, the parts it crosses take damage, and a short-lived beam is
//! drawn. The specification is `..\hf_hitscan_roadmap.md`; phase 0 proved the hook, phase 1
//! measured the shot in play (muzzle, terrain, ammo ids, scan cost), phase 2 added the damage
//! chain (a shot whose loaded ammo is bound in `shells` never calls `FireShell`), phase 3 added the
//! beam (one line per shot, drawn for `visual.seconds` by a per-frame tick in the HUD's draw pass),
//! and phase 4 adds the launch sound vanilla's `Body_InitializeCannonShell` would have played.
//!
//! # The one site
//!
//! `FireGun` (`0x140032e60`) finishes by calling `FireShell` (`0x1400300d0`):
//!
//! ```text
//! 140032f88  MOV  dword ptr [RSP + 0x28], ESI      ; FireShell arg6 = FireGun's own `unused`
//! 140032f8c  MOV  byte ptr [RSP + 0x20], 1         ; FireShell arg5 = mirror_angle
//! 140032f91  MOVSS dword ptr [RDI + 0x86c], XMM7   ; the cooldown store (hf_gun_mechanics)
//! 140032f99  CALL 0x1400300d0                      ; <-- the five bytes this mod replaces
//! 140032f9e  JMP  0x140032ef2                      ; epilogue, AL = FireShell's return value
//! ```
//!
//! Phase 0 replaced those five bytes with a jump to a trampoline that calls [`fire_gate`]. The gate
//! asks [`damage::try_fire`] whether the loaded ammo is a configured laser; if it is not (or if
//! anything goes wrong) it forwards every argument to the real `FireShell` unchanged.
//!
//! # Why the trampoline jumps back instead of ending with `RET`
//!
//! Roadmap §5.2 says the trampoline "ends with `RET` because the patched instruction was a `CALL`".
//! That is not true, and this build deliberately does something else. patchy replaces the site
//! with a five-byte `JMP`, which does **not** push the return address the original `CALL` would
//! have pushed, so at trampoline entry `[RSP]` is whatever sits beneath `FireGun`'s frame — a
//! `RET` there would jump into garbage. The trampoline therefore does `absolute_call(fire_gate)`
//! and then an explicit `relative_jump` to `0x140032f9e`, exactly where the vanilla call would
//! have continued, with `AL` = the gate's return value. (The same `call … ; ret` shape is what
//! `hf_gun_mechanics` uses for its `FireGuns` gate, which is one reason not to copy it.)
//!
//! # Argument frame
//!
//! `absolute_call` is a plain `call m64`: `RCX`/`RDX`/`R8`/`R9` and the caller's stack slots are
//! untouched, so the gate sees exactly the frame `FireShell` would have seen — including the four
//! stack arguments at `[RSP+0x20 .. 0x38]`, which `patchy`'s `preserved_call` / `patch_call`
//! would *not* deliver. That is why the gate is a bare call with an eight-argument `extern "C"`
//! signature rather than a one-pointer callback.
//!
//! Nothing else is touched: loaded rounds, the cooldown, the reload timer and the AI burst latch
//! are all written by `FireGun` before this call.
//!
//! # The second site: the draw tick
//!
//! The beam needs a per-frame hook in the **draw pass**, so this mod also detours the entry of
//! `BattleHud_DrawCombatMarkers` (`0x14029c5c0`) — the battle HUD's marker renderer (it calls
//! `Hud_DrawAimRay` twice and draws the enemy's pre-fire laser itself) — and calls
//! [`beam::draw_tick`] before the original body runs:
//!
//! ```text
//! 14029c5c0  48 8b c4                 MOV  RAX, RSP        ; replayed by the trampoline
//! 14029c5c3  48 89 58 10              MOV  [RAX+0x10], RBX ; replayed by the trampoline
//! 14029c5c7                              <-- the trampoline continues here
//! ```
//!
//! Seven bytes, two complete position-independent instructions, and the function takes no
//! arguments. `patchy`'s preserved call assumes it is entered with `RSP` 16-byte aligned (a hook
//! after a prologue); a function entry has `RSP` at 8 mod 16, so the trampoline moves `RSP` by 8
//! around the call. Without that the hook — and `Gfx_DrawWorldLine` under it, which spills
//! `XMM6` with `MOVAPS` — would run 8 bytes off the ABI's alignment. `Gfx_DrawWorldLine` itself is
//! never hooked: that would recurse.

highfleet_mod_api::export_logger!();

mod ammo;
mod beam;
mod body;
mod config;
mod damage;
mod mem;
mod ray;
mod sound;

use std::{
    ffi::{c_char, CStr},
    mem::{size_of, MaybeUninit},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

use patchy::{PatchSession, ReturnType, Trampoline};
use windows_sys::Win32::System::{
    LibraryLoader::GetModuleHandleA,
    Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS},
};

const IMAGE_BASE: usize = 0x1_4000_0000;
const _: () = assert!(IMAGE_BASE == 0x140000000);

/// `FireGun`'s `CALL FireShell`: five bytes, `E8 rel32`.
const SITE: usize = 0x1_4003_2f99;
const _: () = assert!(SITE == 0x140032f99);
const SITE_SIZE: usize = 5;
/// The vanilla `FireShell` the gate forwards to.
const FIRE_SHELL: usize = 0x1_4003_00d0;
const _: () = assert!(FIRE_SHELL == 0x1400300d0);
/// The instruction after the replaced call: `JMP 0x140032ef2`, `FireGun`'s epilogue.
const RESUME: usize = 0x1_4003_2f9e;
const _: () = assert!(RESUME == 0x140032f9e);
/// `CALL rel32`.
const CALL_OPCODE: u8 = 0xE8;

/// `BattleHud_DrawCombatMarkers` entry — once per frame, in the battle HUD's draw pass.
const DRAW_TICK: usize = 0x1_4029_c5c0;
const _: () = assert!(DRAW_TICK == 0x14029c5c0);

/// The two instructions the draw-tick trampoline replays:
/// `MOV RAX, RSP` + `MOV [RAX + 0x10], RBX`. Both are position-independent, so no relocation is
/// needed — and they must stay exactly these bytes for the 7-byte steal to be instruction-aligned.
const DRAW_TICK_PROLOGUE: [u8; 7] = [0x48, 0x8B, 0xC4, 0x48, 0x89, 0x58, 0x10];
const DRAW_TICK_SIZE: usize = DRAW_TICK_PROLOGUE.len();

/// `SUB RSP, 8` / `ADD RSP, 8` — the alignment fix for a hook at a **function entry** (see the
/// module docs): they cancel the 8-byte difference between a function entry and a mid-function
/// patch site for the duration of the preserved call.
const STACK_DOWN: [u8; 4] = [0x48, 0x83, 0xEC, 0x08];
const STACK_UP: [u8; 4] = [0x48, 0x83, 0xC4, 0x08];

static SUPPORTED_VERSION: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);
static CONFIG: OnceLock<config::Snapshot> = OnceLock::new();

fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// A two-component vector, matching the game's own `Vec2` at the call sites.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Vec2 {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

/// `FireShell`'s real prototype (roadmap §5.3):
///
/// ```c
/// bool FireShell(Body *firing_body, Vec2 *fire_dir, int ammo_index, int side,
///                bool mirror_angle, undefined4 unused, float alignment_override, int suppress_sfx)
/// ```
///
/// The first four arguments travel in `RCX`/`RDX`/`R8`/`R9`: the gun part (`RDI` at the call), a
/// pointer to the local fire direction (`{0.0, 1.0}` for cannon shells), the loaded ammo's
/// position in the ammo table (`gun + 0x850`), and the side (`+1` / `-1`). The last four sit in
/// the caller's shadow space and stack slots, which the bare `call` in the trampoline leaves in
/// place: `mirror_angle` (always `true` from `FireGun`), `FireGun`'s own fourth argument, the
/// alignment override (always `0.0`), and the sound-suppression flag (always `0`).
type FireShellFn = unsafe extern "C" fn(
    gun: *mut u8,
    dir: *const Vec2,
    ammo_position: i32,
    side: i32,
    mirror_angle: bool,
    unused: i32,
    alignment_override: f32,
    suppress_sfx: i32,
) -> bool;

/// Call the vanilla `FireShell` with the gate's own arguments.
///
/// # Safety
///
/// Only valid from the gate, i.e. with the frame `FireGun` prepared for `FireShell`.
unsafe fn vanilla_fireshell(
    gun: *mut u8,
    dir: *const Vec2,
    ammo_position: i32,
    side: i32,
    mirror_angle: bool,
    unused: i32,
    alignment_override: f32,
    suppress_sfx: i32,
) -> bool {
    // SAFETY: the install step proved this address is the Steel 1.163 `FireShell`, and the caller
    // supplies the argument frame `FireGun` built for it.
    let fire_shell: FireShellFn = unsafe { std::mem::transmute(FIRE_SHELL) };
    unsafe {
        fire_shell(
            gun,
            dir,
            ammo_position,
            side,
            mirror_angle,
            unused,
            alignment_override,
            suppress_sfx,
        )
    }
}

/// The gate `FireGun` enters instead of `FireShell`.
///
/// [`damage::try_fire`] answers `true` when the loaded ammo is a configured laser and the shot has
/// been handled without a shell; `false` means "not a laser" and the gate forwards everything.
///
/// # Safety
///
/// Entered from the trampoline with `FireGun`'s frame, so the argument layout is `FireShell`'s.
unsafe extern "C" fn fire_gate(
    gun: *mut u8,
    dir: *const Vec2,
    ammo_position: i32,
    side: i32,
    mirror_angle: bool,
    unused: i32,
    alignment_override: f32,
    suppress_sfx: i32,
) -> bool {
    let forward = || unsafe {
        vanilla_fireshell(
            gun,
            dir,
            ammo_position,
            side,
            mirror_angle,
            unused,
            alignment_override,
            suppress_sfx,
        )
    };
    // A panic must never cross the FFI boundary (hazard H1).
    match catch_unwind(AssertUnwindSafe(|| unsafe {
        damage::try_fire(gun, dir, ammo_position)
    })) {
        Ok(true) => true,
        Ok(false) => forward(),
        Err(_) => {
            log::error!("hf_hitscan: gate panicked; firing the vanilla shell");
            forward()
        }
    }
}

/// The trampoline behind the replaced `CALL`: call the gate, return to `FireGun`.
fn gate_trampoline() -> Trampoline {
    let mut trampoline = Trampoline::new();
    // A plain eight-argument call: argument registers and the caller's stack slots stay in place,
    // so the gate sees the same frame FireShell would have.
    trampoline.absolute_call(fire_gate as *const ());
    // The replaced instruction was a `CALL`; the `JMP` that replaced it pushed nothing, so the
    // continuation has to be named explicitly (see the module docs).
    trampoline.relative_jump(RESUME);
    trampoline
}

/// The per-frame draw tick. Entered from the draw tick's trampoline with the game's frame, before
/// `BattleHud_DrawCombatMarkers` has executed a single instruction.
extern "C" fn draw_tick_hook() {
    // A panic must never cross the FFI boundary (hazard H1); drawing nothing this frame is the only
    // safe outcome.
    let _ = catch_unwind(AssertUnwindSafe(beam::draw_tick));
}

/// The trampoline behind the draw tick: replay the two stolen instructions, call
/// [`draw_tick_hook`], continue at `0x14029c5c7`.
fn draw_tick_trampoline() -> Trampoline {
    let mut trampoline = Trampoline::new();
    // `MOV RAX, RSP` / `MOV [RAX+0x10], RBX` — `RAX` is restored by the preserved call, so the
    // function's own prologue still sees `RSP`.
    trampoline.bytes(&DRAW_TICK_PROLOGUE);
    // See `STACK_DOWN`: `preserved_call` keeps `RSP` unchanged mod 16, so a function-entry site
    // needs this one adjustment to enter the hook and `Gfx_DrawWorldLine` properly aligned.
    trampoline.bytes(&STACK_DOWN);
    trampoline.preserved_call(draw_tick_hook as *const (), &[], &[], ReturnType::None);
    trampoline.bytes(&STACK_UP);
    trampoline.relative_jump(DRAW_TICK + DRAW_TICK_SIZE);
    trampoline
}

pub(crate) unsafe fn read<T: Copy>(address: usize) -> T {
    // SAFETY: every caller checks `readable` for the range first.
    unsafe { (address as *const T).read_unaligned() }
}

/// True when `size` bytes at `address` sit in a committed, readable, guard-free page.
pub(crate) fn readable(address: usize, size: usize) -> bool {
    readable_extent(address, size) >= size
}

/// How many bytes (up to `max`) are committed, readable and guard-free from `address` **inside its
/// current region**. Lets a whole-array walk do one query instead of one per element.
pub(crate) fn readable_extent(address: usize, max: usize) -> usize {
    if address < 0x1_0000 {
        return 0;
    }
    let mut information = MaybeUninit::<MEMORY_BASIC_INFORMATION>::uninit();
    // SAFETY: VirtualQuery only inspects an address; the buffer is sized and aligned for the struct.
    let queried = unsafe {
        VirtualQuery(
            address as *const _,
            information.as_mut_ptr(),
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if queried != size_of::<MEMORY_BASIC_INFORMATION>() {
        return 0;
    }
    // SAFETY: VirtualQuery returned a complete structure.
    let information = unsafe { information.assume_init() };
    if information.State != MEM_COMMIT
        || information.Protect == PAGE_NOACCESS
        || information.Protect & PAGE_GUARD != 0
    {
        return 0;
    }
    let region_offset = address.saturating_sub(information.BaseAddress as usize);
    information
        .RegionSize
        .saturating_sub(region_offset)
        .min(max)
}

/// Target of the five-byte `CALL rel32` at `site`, or `None` when the bytes are not such a call.
fn call_target(site: usize) -> Option<usize> {
    if !readable(site, SITE_SIZE) {
        return None;
    }
    // SAFETY: the five bytes were just proved readable.
    let bytes = unsafe { std::slice::from_raw_parts(site as *const u8, SITE_SIZE) };
    if bytes[0] != CALL_OPCODE {
        return None;
    }
    let displacement = i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    Some(((site + SITE_SIZE) as i64 + displacement as i64) as usize)
}

/// Wait for `0x14032f99` to become the `CALL FireShell` the hook assumes.
///
/// The Steam build unpacks itself while the modloader starts, so the first reads can land on
/// still-encrypted memory. Any other outcome — a different opcode or a call to a different target
/// — is refused rather than patched (roadmap §5.2).
fn wait_for_call_site() -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..=20 {
        match call_target(SITE) {
            Some(target) if target == FIRE_SHELL => {
                if attempt > 0 {
                    log::info!("hf_hitscan: {SITE:#x} matched after {attempt} retr(ies)");
                }
                return Ok(());
            }
            Some(target) => {
                last = format!("the CALL at {SITE:#x} reaches {target:#x}, not FireShell {FIRE_SHELL:#x}");
            }
            None => last = format!("no readable `E8 rel32` at {SITE:#x}"),
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(format!("{last}; not patched"))
}

/// Wait for `BattleHud_DrawCombatMarkers` to start with the two instructions the draw-tick
/// trampoline replays. A different prologue is refused rather than patched: stealing 7 bytes in the
/// middle of an instruction would crash the game on the first frame of a battle.
fn wait_for_draw_tick() -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..=20 {
        if readable(DRAW_TICK, DRAW_TICK_SIZE) {
            // SAFETY: the range was just proved readable.
            let bytes = unsafe { std::slice::from_raw_parts(DRAW_TICK as *const u8, DRAW_TICK_SIZE) };
            if bytes == DRAW_TICK_PROLOGUE {
                if attempt > 0 {
                    log::info!("hf_hitscan: {DRAW_TICK:#x} matched after {attempt} retr(ies)");
                }
                return Ok(());
            }
            last = format!(
                "{DRAW_TICK:#x} starts {:02X?}, not the expected {DRAW_TICK_PROLOGUE:02X?}",
                bytes
            );
        } else {
            last = format!("{DRAW_TICK:#x} is not readable");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(format!("{last}; the beam is not drawn (damage is unaffected)"))
}

/// Verify both sites and install the two hooks. Returns the number of hooks installed.
///
/// The fire hook is required; the draw tick is not — without it the damage model works exactly as
/// in phase 2 and only the beam is missing, so a bad prologue is reported and the mod stays
/// installed.
unsafe fn install() -> Result<usize, String> {
    wait_for_call_site()?;
    log::info!("hf_hitscan: FireGun call site verified at {SITE:#x} (CALL FireShell {FIRE_SHELL:#x})");

    let draw_tick = wait_for_draw_tick();
    match &draw_tick {
        Ok(()) => log::info!(
            "hf_hitscan: draw tick verified at {DRAW_TICK:#x} (BattleHud_DrawCombatMarkers entry)"
        ),
        Err(error) => log::error!("hf_hitscan: draw tick refused: {error}"),
    }

    let mut session = PatchSession::new();
    // SAFETY: the fire site was verified above.
    unsafe {
        session
            .detour_trampoline(SITE, SITE_SIZE, gate_trampoline())
            .map_err(|error| error.to_string())?;
    }
    // The draw tick is optional: a patch error must not cost the damage model.
    let mut draw_hook = 0usize;
    if draw_tick.is_ok() {
        // SAFETY: the prologue was verified above.
        match unsafe {
            session.detour_trampoline(DRAW_TICK, DRAW_TICK_SIZE, draw_tick_trampoline())
        } {
            Ok(_) => draw_hook = 1,
            Err(error) => log::error!("hf_hitscan: the draw tick could not be patched: {error}"),
        }
    }
    // SAFETY: both sources were verified above.
    unsafe {
        session
            .install_permanently()
            .map_err(|error| error.to_string())?;
    }
    Ok(1 + draw_hook)
}

/// One line per configured type, so a play session records what was armed.
fn log_config(config: &config::Snapshot) {
    log::info!(
        "hf_hitscan: {} type(s), {} shell binding(s)",
        config.type_count(),
        config.shell_count()
    );
    for laser in &config.types {
        log::info!(
            "hf_hitscan: type {:?}: range {:.0}, damage {:.2}, decay {:.4}/u, spread {:.4} rad, pierce {} (stop mask {:#x}, max parts {}, through terrain {}), ignore armour threshold {}, incendiary {:.1}, impact explosion {} at {:?} (power {:.1}), colour {:#010x}/{:#010x}, width {:.2}, {:.3}s {:?}",
            laser.name,
            laser.range,
            laser.damage,
            laser.damage_decay_per_unit,
            laser.spread,
            if laser.pierce { "on" } else { "off" },
            laser.stop_mask,
            laser.max_parts,
            laser.through_terrain,
            laser.ignore_armor_threshold,
            laser.incendiary,
            laser.explosion,
            laser.blast_at,
            laser.explosive_power,
            laser.color,
            laser.glow_color,
            laser.width,
            laser.seconds,
            laser.fade
        );
    }
    for (item_name, index) in &config.shells {
        if let Some(laser) = config.types.get(*index) {
            log::info!("hf_hitscan: shell {item_name} -> {:?}", laser.name);
        }
    }
}

#[no_mangle]
unsafe extern "C" fn version(version: *const c_char) -> bool {
    // SAFETY: the loader passes a live NUL-terminated version string.
    let supported =
        !version.is_null() && unsafe { CStr::from_ptr(version) }.to_bytes() == b"Steam 1.163";
    SUPPORTED_VERSION.store(supported, Ordering::Relaxed);
    supported
}

#[no_mangle]
unsafe extern "C" fn init() -> bool {
    // SAFETY: a null module name asks for the main module.
    let image_base = unsafe { GetModuleHandleA(std::ptr::null()) } as usize;
    let supported = SUPPORTED_VERSION.load(Ordering::Relaxed);
    log::info!(
        "hf_hitscan v{}: image base {image_base:#x}, version {}",
        env!("CARGO_PKG_VERSION"),
        if supported { "Steam 1.163" } else { "unsupported" }
    );
    if !supported {
        log::error!("hf_hitscan: needs Steam 1.163; not started");
        return false;
    }
    if image_base != IMAGE_BASE {
        log::error!(
            "hf_hitscan: unexpected image base {image_base:#x} (want {IMAGE_BASE:#x}); not started"
        );
        return false;
    }
    if INSTALLED.load(Ordering::Relaxed) {
        return true;
    }

    let snapshot = match config::load() {
        Ok(snapshot) => snapshot,
        Err(errors) => {
            for error in &errors {
                log::error!("hf_hitscan: config rejected: {error}");
            }
            log::error!(
                "hf_hitscan: {} problem(s) in {}; nothing is installed and no shot becomes a laser",
                errors.len(),
                config::PATH
            );
            return false;
        }
    };
    VERBOSE.store(snapshot.verbose_logging, Ordering::Relaxed);
    log_config(&snapshot);
    if !snapshot.enabled {
        log::info!("hf_hitscan: disabled in {}", config::PATH);
        return true;
    }
    let _ = CONFIG.set(snapshot);

    match unsafe { install() } {
        Ok(hooks) => {
            INSTALLED.store(true, Ordering::Relaxed);
            let (types, shells) = CONFIG
                .get()
                .map(|config| (config.type_count(), config.shell_count()))
                .unwrap_or_default();
            log::info!(
                "hf_hitscan v{}: {hooks} hook(s) installed; config {types} type(s) / {shells} shell \
                 binding(s); mapped ammo damages along the ray instead of spawning a shell, plays \
                 the ammo's own launch sound and draws a beam (phase 4)",
                env!("CARGO_PKG_VERSION")
            );
            true
        }
        Err(error) => {
            log::error!("hf_hitscan: not installed: {error}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_site_is_the_call_and_the_continuation_is_the_next_instruction() {
        assert_eq!(SITE, 0x140032f99);
        assert_eq!(FIRE_SHELL, 0x1400300d0);
        assert_eq!(RESUME, SITE + SITE_SIZE, "the epilogue jump follows the call");
    }

    /// The trampoline must call the gate and then land on `0x140032f9e` — not `RET` into the
    /// caller's frame.
    #[test]
    fn the_trampoline_calls_the_gate_and_resumes_at_the_epilogue() {
        const ADDRESS: usize = 0x1_7000_0000;
        let code = gate_trampoline().build(ADDRESS).expect("trampoline builds");
        assert_eq!(code.len(), 21, "16-byte absolute call + 5-byte jump");

        assert_eq!(
            &code[..8],
            &[0xFF, 0x15, 0x02, 0x00, 0x00, 0x00, 0xEB, 0x08],
            "call qword ptr [rip+2]; jmp +8"
        );
        assert_eq!(
            usize::from_le_bytes(code[8..16].try_into().unwrap()),
            fire_gate as *const () as usize,
            "the inline pointer is the gate"
        );
        assert_eq!(code[16], 0xE9, "explicit jump back into FireGun");
        let displacement = i32::from_le_bytes(code[17..21].try_into().unwrap());
        assert_eq!(
            (ADDRESS + 21) as i64 + displacement as i64,
            RESUME as i64,
            "resumes on JMP 0x140032ef2"
        );
        assert_ne!(code[20], 0xC3, "never a RET");
    }

    /// `call_target` resolves `E8 rel32` and rejects anything else.
    #[test]
    fn the_install_check_resolves_a_relative_call() {
        let mut bytes = vec![0x90u8; 16];
        let site = bytes.as_ptr() as usize;
        let target = site + 0x128;
        bytes[0] = CALL_OPCODE;
        bytes[1..5].copy_from_slice(&((target as i64 - (site + 5) as i64) as i32).to_le_bytes());
        assert_eq!(call_target(site), Some(target));

        bytes[0] = 0xE9;
        assert_eq!(call_target(site), None, "a jump is not a call");
    }

    /// The draw tick is at a function entry, so the steal must be two whole instructions and the
    /// continuation must be the third.
    #[test]
    fn the_draw_tick_site_starts_with_the_two_replayed_instructions() {
        assert_eq!(DRAW_TICK, 0x14029c5c0);
        assert_eq!(DRAW_TICK_SIZE, 7, "MOV RAX,RSP is 3 bytes; MOV [RAX+0x10],RBX is 4");
        assert_eq!(DRAW_TICK_PROLOGUE, [0x48, 0x8B, 0xC4, 0x48, 0x89, 0x58, 0x10]);
        assert_eq!(DRAW_TICK + DRAW_TICK_SIZE, 0x14029c5c7, "continuation");
        // Both are position-independent, which is what lets the trampoline replay them verbatim.
        assert!(!DRAW_TICK_PROLOGUE.contains(&0xE8), "no relative call");
        assert!(!DRAW_TICK_PROLOGUE.contains(&0xE9), "no relative jump");
    }

    /// The trampoline: stolen bytes, the alignment fix, the hook, then the HUD's next instruction.
    #[test]
    fn the_draw_tick_trampoline_replays_the_prologue_and_realigns_the_stack() {
        const ADDRESS: usize = 0x1_7000_0000;
        let code = draw_tick_trampoline().build(ADDRESS).expect("trampoline builds");

        assert_eq!(&code[..DRAW_TICK_SIZE], &DRAW_TICK_PROLOGUE, "stolen bytes first");
        assert_eq!(
            &code[DRAW_TICK_SIZE..DRAW_TICK_SIZE + STACK_DOWN.len()],
            &STACK_DOWN,
            "RSP -= 8 before the preserved call"
        );
        let tail = code.len() - STACK_UP.len() - 5;
        assert_eq!(&code[tail..tail + STACK_UP.len()], &STACK_UP, "RSP += 8 after it");
        assert_eq!(code[tail + STACK_UP.len()], 0xE9, "explicit jump back into the HUD");
        let displacement = i32::from_le_bytes(code[tail + STACK_UP.len() + 1..].try_into().unwrap());
        assert_eq!(
            (ADDRESS + code.len()) as i64 + displacement as i64,
            (DRAW_TICK + DRAW_TICK_SIZE) as i64,
            "continues on the instruction after the stolen two"
        );

        let hook = draw_tick_hook as *const () as usize;
        assert!(
            code.windows(std::mem::size_of::<usize>())
                .any(|window| window == hook.to_le_bytes().as_slice()),
            "the hook's address is inline (an absolute call)"
        );
    }

    /// Why `STACK_DOWN` exists: a function entry has `RSP` at 8 mod 16, while `patchy`'s preserved
    /// call (seven pushes plus a `0x88` frame, 192 bytes) leaves `RSP` unchanged mod 16. Without
    /// the 8-byte adjustment the hook — and `Gfx_DrawWorldLine` under it, which spills `XMM6` with
    /// `MOVAPS` — would run 8 bytes off the ABI's alignment.
    #[test]
    fn the_draw_tick_call_is_aligned_for_a_function_entry() {
        let entry_rsp: i32 = 8;
        let preserved_call: i32 = 7 * 8 + 0x88;
        assert_eq!(preserved_call % 16, 0, "patchy assumes an aligned hook");
        assert_eq!((entry_rsp - 8) % 16, 0, "STACK_DOWN puts RSP on a 16-byte boundary");
        assert_eq!((entry_rsp - 8 + preserved_call) % 16, 0, "so the hook's CALL is aligned");
    }
}
