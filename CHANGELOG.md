# Changelog

All notable changes to `hf_hitscan`. One entry per built DLL; every entry carries the sha256 of the
DLL it produced, so a bug report can be matched to a build.

Format: [Keep a Changelog](https://keepachangelog.com/), versions are `MAJOR.MINOR.PATCH`, dates are
local (`2026-11-23`, `2026-11-24`). The engine-level detail behind each entry is in
[docs/DESIGN.md](docs/DESIGN.md).

## [1.0.0] — 2026-11-24

**Packaging release. No gameplay change:** the damage chain, both patch sites (the 5-byte
`CALL FireShell` at `0x140032f99`, resumed with an explicit jump to `0x140032f9e`, and the 7-byte draw
tick at `0x140029c5c0`), the config schema and the logging switches are untouched from 0.9.0.

* Version `Cargo.toml` 0.9.0 → **1.0.0** (the log's first line is now `hf_hitscan v1.0.0: …`).
* **Shipped config replaced.** The two-type demo (`laser_pierce` / `laser_blunt` over real 130 mm
  ammo) is replaced by the four **example** types — `hitscan_impact`, `hitscan_explosive`,
  `hitscan_incendiary`, `hitscan_pierce` — bound to the placeholder ammo names `ITEM_AMMO_EXAMPLE1..4`.
  The file is a template: nothing changes in game until a player maps real ammo in `shells`.
* `verbose_logging` now ships **`false`**. The diagnostics are still in the code (every per-shot line
  is behind the switch); only the ~17 launch-time lines and one line per ammo's first shot remain.
* `hitscan_incendiary` gained `pierce.enabled: true`, so its `max_parts: 10` means what it says:
  damage and heat to up to 10 parts along the ray.
* `hitscan_pierce` lost its `stop_at` entry (`[]`), because `stop_at` matches `m_category` bits and a
  combridge shares bit 64 with blocks — "stop at the combridge only" is not expressible without a new
  config feature (deferred to a later slice).
* `README.md` split: this player-facing page; all engine findings and play-check procedures moved to
  `docs/DESIGN.md` (nothing deleted).
* Added `LICENSE` (MIT, © 2026 Duumvir-hf) and this changelog; added `hf_hitscan-v1.0.0.zip`.
* Tests: **55** (`shipped_config_is_valid_and_test_ready` and
  `ammo_binding_is_case_insensitive_and_ammo_only` now assert the release config).

DLL sha256 `2CE26026A58D3A2D2C9B781EE6B6DEFADDD2063B0CDF4B20E853E84DD7B0F087`
· config sha256 `B39992F6A6AD39E94F283DBBEB7C631B890D4D1BB57F02A2BB6342DF53F15CDA`
· 478 720 bytes.

## [0.9.0] — 2026-11-23 22:50

**Phase 5.5 — per-type spread.** New `types.<name>.spread`, in **radians like vanilla**: one uniform
`HGE.Random_Float(-spread, +spread)` roll per shot (the same call `sound.rs` already makes, now
reused) rotates the firing direction between the muzzle and `ray::cast`; the muzzle does not move,
`|d|` is preserved, and the rotated `d` is what the scan, the decay distances, the blast and the beam
use. `0` (the default and the shipped value) draws no number at all — bit-for-bit the 0.8.x path. A
negative value rejects the file; `spread_degrees` is **not** a key (typos must not be silent). The
summary line prints `(scatter +0.0312 rad, spread 0.0500)` when the type scatters, and the config dump
prints every type's `spread`. Shipped config gained `"spread": 0.0` on both types (sha256
`31F3A051CF419762D54E2699777F272BA97B50D7D40FC98F3FFEB535E2A0E0A6`); the user's live file was not
touched. 55 tests. **Verified in play the same day** — "works perfectly fine. beautiful."

DLL sha256 `35E4CD5BADE433C0FA196735E9ECDD0DB5EBE71E129AEFB8ECC141DE06EDF2D4`.

## [0.8.2] — 2026-11-23 22:27

**The blast is spawned once, at the configured power.** The 0.7.2 4× retry is gone (user request):
its trigger was the victim count read out of `0x1403ef050..0x1403ef058`, which reported `0 bodies` on
**every** blast in every log — even 23-part passes — so it acted as a permanent 4× power multiplier
(and a second area-damage pass), not a rare fallback. `body::explode` now returns `bool`
(`Body_Explode`'s own value), the summary prints `blast yes at N u (first)` with no victim count, and
`FIZZLED` still appears if the guard refuses. **`explosive_power` is the only lever**: radius
`2·√power` game units — 5 → 4.5 u, 20 → 8.9 u, 50 → 14.1 u, 100 → 20 u, 400 → 40 u. 53 tests.

DLL sha256 `311CD464FF24C707030C8AC2C67F841FFB238178A186EC11E8F6A6DEF0969851`.

## [0.8.1] — 2026-11-23 22:04

**Where the beam went, in the summary line.** The 21:55/21:58 logs show `laser_pierce_2` piercing
normally whenever the ray crosses a ship (17/17, 16/15, 14/14, 10/10 parts damaged) and all 54 of its
shots stopping at `the end of the hit list` — no `max_parts`, `terrain` or `spent damage` stop with
`max_parts 0`, `decay 0`, `through_terrain true`; the remaining shots are `1 hit` / `0 hit`, i.e. the
ray met one body or none. To settle "it sticks on the engine", the summary now names both ends:
`first "MDL_ENGINE_05"@965.7 (joint), last "MDL_ARMOR1X1_03"@1008.8 (mounted)` (`joint` = the body
owns its own b2 body, `mounted` = a part whose fixture lives on the Frame), the tally gained a
`terrain` bucket, and a **piercing** type with ≤3 hits prints every hit with oid and verdict.
53 tests.

DLL sha256 `6A9E89371DACE8395F7FB28E6DA12A2FA8F17083699535579ED9A6DBC7A2F14D`.

## [0.8.0] — 2026-11-23 21:48

**`pierce.through_terrain` + a blast on a body the hit kills.** New `pierce.through_terrain` (default
`false`): the level's ground takes no damage **and** no longer ends the beam, so a type can pierce
everything in `range`; the shipped config now carries the key (sha256
`4E18A1958F391312F7468773E0029813E1FF15D9A189C5C4B58A2F8BC9D9DB61`). The 21:31 logs show
`blast FIZZLED — Body_Explode refused` on exactly the shots that **killed** the target part (the death
path clears `reflected`), so a blast on a lethal hit (`damage >= health`) now runs **before**
`Body_ApplyDamage`, while the body is still live and reflected; a non-lethal blast stays right after
the damage call. Also confirmed there: every blast found **0 bodies** (2x2 truss/armour neighbours sit
farther than the 8.9 u radius of `power 20`) — `explosive_power` is the lever. 53 tests.

DLL sha256 `AFBE0379441C85D2562F3DD3F9B266042D23149E5BFC27BBCC25E7672A712814`.

## [0.7.2] — 2026-11-23 13:46

**An invisible explosion is a victimless blast, not a missing call.** The 809-shot 13:22 log reaches
`Body_Explode` on all 262 damaging `laser_blunt` shots, but that routine only **draws for the bodies
it damages** (it fills the scratch vector `0x1403ef050..0x1403ef058` and attaches `airblast_01.psi` to
each victim), and its radius is `2·√power` game units — `power 20` is **8.9 u**, less than the distance
between the centres of two large parts. 37 of 262 calls finished in <25 µs (no victims) against
~137 µs average (victims). `body::explode` now returns `Blast { ran, victims, retried }` read out of
that vector, the summary line prints the count (`blast yes at 524.7 u (first: 7 bodies)` /
`FIZZLED …`), and a blast that finds **0** bodies is retried once at 4× power (radius ×2; the first
call damaged nothing, so nothing is doubled). 52 tests. *(The retry was removed again in 0.8.2.)*

DLL sha256 `E5DC3409CEA375FD3E3931E19E0284FDAE971D13533818C521ABF46D1B936482`.

## [0.7.1] — 2026-11-23 12:38

**Land is not "anything ownerless".** The 574-shot 12:23 log showed the last two reports were one bug:
a stop at `owner 0, partIndex -1, class 0x0f, health 100.0, oid ""` — wreck debris or a missile, not
terrain. `Node_AddChild` sets a spawned body's *master*, not its owner, so debris and missiles are
ownerless; 0.7.0's `owner == 0 → terrain` rule therefore ended beams on them and gave them no damage
and no blast. Terrain now also requires `health >= 10000` (`body::TERRAIN_HEALTH_MIN`; land is 100000,
missiles 10, ruin fragments 100, debris particles 1000, the toughest ship part a few hundred) —
ownerless debris/missiles are free objects again: damaged, and the beam passes through. Every verdict
on a free-body hit is logged (not only before the first damaged part), and the terrain line now prints
`owner`. 51 tests. No config change.

DLL sha256 `15C14AE7FB2E79E1C82DFABED1671C75617EBD2C01DE4FA842B7CDA772E3662E`.

## [0.7.0] — 2026-11-23 12:06

**Play-check fixes.**

1. **The "plain cannon shell" test is gone** — `+0x154` is `ammo-extended`'s caliber, so a custom round
   (`ITEM_AMMO_THRANGXNON`, caliber `95`) was refused; the `shells` map is the only opt-in now, the ids
   are logged only.
2. **Free bodies take damage**: a hit with no part index, a live owner and `class != 0x1f` is a
   missile/shell/debris and gets `Body_ApplyDamage` (no category, so no `stop_at`) instead of being
   skipped — anti-ship missiles can be shot down.
3. **Only an ownerless non-part body is terrain**; the part index is read first, so a torn-off part
   (owner link gone) no longer ends the beam as a hill, and a ship **Frame** (`class 0x1f`) is skipped
   without stopping it. The terrain log line carries class/partIndex/health/oid.
4. **The blast runs right after its body is damaged** (`impact.at` first/last still picks it), not
   after the whole chain, because a later death could reap the body — the "explosion sometimes did not
   spawn" bug; a `last` target the loop cannot predict keeps the post-chain fallback and logs when the
   body is gone.
5. **A sound set with a missing variant falls back** to the other variants by number when
   `playSFXSound_` returns `-1` (`shell_out_small_far` has 3 files, the far branch asks for 4).

Summary tally is now `[part, object, frame, dead]`. 51 tests. No config change.

DLL sha256 `C009975D83C0E93570601D51B162563CD3A81A5752FF12804B44AACED2314BC7`.

## [0.6.0] — 2026-11-23 10:47

**Phase 4 — the launch sound.** New `sound` module: `damage::try_fire` plays the sound
`Body_InitializeCannonShell` would have played, before the chain, from the ammo record it already has
— `shell_out` (+0xf0, 3 variants, `kind 0xb`, `GlobalNode_`) for the local player zoomed in,
`shell_enemy` (+0x110) / `shell_far` (+0x130) (4 variants, `kind 2`, the gun's `Body*`) otherwise, with
`Sound_BuildName(name, 1, variants, "_")` and vanilla's `HGE.Random_Float(75, 100)` volume (halved on
the player-close branch when `FUN_140073340(owner)` is non-zero). No hook, no patch site, no config
key, no rate limit, no clamp; a shot forwarded to vanilla is not doubled; the play call is inside its
own `catch_unwind`, and the `playSFXSound_` entry bytes are checked once. `ammo::Ammo::sound_name`
hands the game the field's own NUL-terminated buffer. 52 tests. Config untouched (`987F990C…`).
**Verified in play the same day** — all three branches queued.

DLL sha256 `68E245CFE22BE8F4075450BF78AF923A3AF3311A60D95CB67503876F30D78EFF`.

## [0.5.0] — 2026-11-23 09:37

**Phase 3 — the beam.** A new `beam` module keeps a 32-entry ring of live beams; `damage::try_fire`
pushes one line per handled shot (muzzle → where the chain stopped: the terrain hit, else the last
part the beam reached, else `muzzle + d × range`), and a second hook at the entry of
`BattleHud_DrawCombatMarkers` (`0x14029c5c0`, 7-byte steal, `SUB RSP,8` alignment fix for a
function-entry site) draws them each frame with `Gfx_DrawWorldLine` (`0x140248ba0`) — two lines per
beam when `visual.glow`, alpha faded by `visual.seconds`/`fade` on the `HGE` application clock
(`+0xd68`; a negative age is a clock reset and is dropped, H11). A shot forwarded to vanilla never
pushes a beam. No config change (the `visual` keys have existed since 0.1.0). 46 tests.

DLL sha256 `33509C61C9E670EE673A160986133998F6B5AB8D03ED2FDA2BD673E34DFD5A86`.

## [0.4.2] — 2026-11-23 08:40

`impact.at`: `"first"` (default, the entry point) or `"last"` (the far end of a piercing beam / wherever
it stopped), per type. The summary line says which one ran: `blast yes at N u (first|last)`. Shipped
config gained `"at": "first"` on both types (sha256
`987F990C92575CA2F62DD9FACD88580E7044E0F10FA8FF992E084A02A6CAF0F1`); the user's live file got the key
with `last` on `laser_pierce` and `first` on `laser_blunt` so one session shows both.

DLL sha256 `67F9F0502D61BC1C4A3883BF837EF780E6AE31C561F8312672C91DA774D0FDAF`.

## [0.4.1] — 2026-11-23 08:29

Fourth play-check fix: the blast was spawned at the **last** part the beam damaged, i.e. the exit point
of a piercing beam; it is now spawned at the **first** part (the entry point), and the summary line
says `blast yes at N u`. No config change. Measured cost in the 0.4.0 log: a 23-part kill costs
**1.06 ms** of `Body_ApplyDamage` in total (~46 µs per part), the scan 0.15–0.70 ms — the mod is not
frame-scale, so the earlier hitch was the per-shot log volume, which 0.4.0 cut.

DLL sha256 `0532B7E54A94B90837DE2E4DDC8EF7C10001961E6DEAE58C9FCE3CE709B31861`.

## [0.4.0] — 2026-11-23 08:09

Third play-check build. **Impact blast:** new `types.<name>.impact { explosion, explosive_power }`;
when on, the part the beam damaged gets `explosivePower (+0x2dc)` stamped, `Body_Explode 0x140029b60`
(the game's own blast: radius `2·√power`, area damage, particles, ignition) runs, and the field is
restored. **Cost accounting:** the per-shot line now reports `scan`, `chain` and the time inside
`Body_ApplyDamage`/`Body_Explode`. **Less log I/O:** per-hit lines are no longer printed for the first
20 shots, only for shots that damage nothing. Config: the shipped file gained the `impact` block
(`laser_blunt` on, `laser_pierce` off), sha256
`70A56D3E3A98600D5DBAFAE07DF0BBE2A87261C7DB112498CD3D4365A0F22058`; the user's live file got the
same block with all of his values unchanged.

DLL sha256 `BA82D366E3148D083EE9D214702FEAB54FB8702827FB52FA2D3BBCDB9199F823`.

## [0.3.2] — 2026-11-23 07:17

**Second play-check fix — the damage model was hitting the wrong object.** A ship's rigidly mounted
parts are one **fixture each on the Frame body** (`Body_CreateFixture`), and
`b2Fixture.m_userData (+0x40)` is the part `Body*` (`b2Fixture::Create` copies it); only
gimballed/free parts own a body. `ray::cast` now resolves every fixture to its part, so
armour/hull/ammo/block parts take damage in nearest-first order instead of only engines/guns/legs.
`FIXTURES_PER_BODY_MAX` 128 → 8192 (a big frame has hundreds of part fixtures). The chain no longer
requires `IsActive` for a hit part (mounted parts have no body) but requires a vtable inside the module
image (`body::plausible`) plus the game's own health/lock guard; a part whose owner is not a live
object is skipped. Config untouched.

DLL sha256 `463AB1CDB1A5377B85C72AFB1928F1F665C1A5D69EA150F9B81C177102E4E823`.

## [0.3.1] — 2026-11-23 07:08

First play-check fixes. **`_PROXY` bug:** logic `160` (proximity-fuze cannon shell) was in the missile
veto list, so every `_PROXY` shot stayed a vanilla round; removed. `+0xcc` dropped from the plain-shell
rule (it read `0x5f8c` in vanilla and is not a dispatch id without `ammo-extended`); `+0x154` alone
decides. Diagnostics: every ammo logs its resolution once — including ammo that is **not** in `shells`;
every shot logs a tally (`part`/`non-part`/`dead`, damaged, blocked) and its stop reason; until
something has been damaged, every individual hit is printed with its verdict. Config untouched.

DLL sha256 `4B112F80F0832CD17F595167054AA28176624CAEAE6F1A3EE619C02C9C9A47F5`.

## [0.3.0] — 2026-11-23 05:57

**Phase 2 damage chain:** mapped ammo no longer calls `FireShell`; muzzle rebuilt as `FireShell` does,
HUD ray cast from it, `Body_ApplyDamage` nearest-first with range/decay, `pierce`/`stop_at`/
`max_parts`, `ignore_armor_threshold` as `directHit`, incendiary dose to `+0x2f0`; terrain
(`owner 0`) and non-parts (`partIndex < 0`) rules; owner-based own-body exclusion (drops the `+0x224`
flag and H9); one `VirtualQuery` per body. `probe.rs` replaced by `ammo.rs` (one-time ammo resolution
log) plus `ray.rs` / `damage.rs` / `body.rs` / `mem.rs`. Config untouched
(`513092EB0EAC9AA6A8EBFF30761FB8B74101369B79D295498AE175F13AE5CA81`).

DLL sha256 `94D3EDA17AB5B91C2EBAB80052271AC462BD18C1E4C5A01A36FF8AFB0E6ADB00`.

## [0.2.0] — 2026-11-23 05:39

**Phase 1 probe:** per-shot log of `m_oid`, `item_name`, `+0x154` / `+0xcc` / `+0x150`, `ammo_pos`,
`dir`, `FireShell`'s muzzle (`o + 10·d`, vtable `+0xe8`, not `+0x18`), the HUD's ray with every hit and
the scan cost. Vanilla `FireShell` still fired every shot. Config untouched. 50 shots, two guns;
"shots still look and sound vanilla".

DLL sha256 `8951956400A66D611BD32DF894BBDFE26CCBF26E392053E9ECD8D4AFFAD4B72B`.

## [0.1.0] — 2026-11-23

**Phase 0 scaffold:** one hook at `0x140032f99`, gate forwards every argument to vanilla `FireShell`;
full config schema + validation; no gameplay change. Config `hitscan.json` sha256
`513092EB0EAC9AA6A8EBFF30761FB8B74101369B79D295498AE175F13AE5CA81`.

DLL sha256 `A7104FAA7A8B84C6CEDCCB962615F8BA8963089BE7EF2F68E4C83C1310C2FE52`.
