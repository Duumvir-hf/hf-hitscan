# hf_hitscan — design notes and engine reference

**This is the engineering document.** The player-facing page is `..\README.md`; the per-version
history (including every DLL sha256) is `..\CHANGELOG.md`; the specification the mod was built from is
`..\..\hf_hitscan_roadmap.md`.

Everything below was measured on **HighFleet Steam 1.163**, image base `0x140000000`, and is the
authority for how the mod works — the roadmap is the plan, this file is what is actually true. Where a
section says "verified in play", it means the user saw the behaviour in his own session on the date
named; the dates are the `Modloader\logs` sessions listed in the changelog.

Contents:

1. What the pre-1.0 README recorded, version by version (0.3.1 → 0.9.0)
2. Play checks and the per-shot log (how each phase was verified)
3. Engine notes — fixtures, patch sites, damage, fire, blast, beam, sound, ray, cost, limits
4. Phase 1 findings (the live probe, 2026-11-23 05:43)
5. Building, and the standing process decisions

---

## 1. What the pre-1.0 README recorded, version by version

`0.1.0` – `0.3.0` are described in `..\CHANGELOG.md` only. From `0.3.1` on, these are the notes as
they were written after each play check, with the measured facts intact.


**0.3.1 (2026-11-23, after the first play check):** `ITEM_AMMO_130MM_PROXY` stayed a vanilla shell
because the plain-shell rule vetoed projectile logic `160`, which is the *proximity-fuze cannon
shell*, not a missile. Fixed, and the per-shot log now explains every shot that damages nothing —
see *The log, per laser shot*.

**0.3.2 (2026-11-23, after the second play check): a hit is a fixture, and a fixture belongs to a
part.** Most parts of a ship (armour, hull, ammo, blocks, FCS, …) have **no `b2Body` of their own**:
they are one polygon **fixture on the ship's Frame body**, and the part they belong to is stored in
`b2Fixture.m_userData`. Damage 0.3.0/0.3.1 used the *body array entry* as "what was hit", i.e. the
Frame (`partIndex -1`, skipped) — so only the jointed parts that do own a body (engines, guns, legs)
ever took damage. The ray now resolves every fixture to its part. See *A hit is a fixture* below.

**0.4.0 (2026-11-23, third play check): impact blast + cost accounting.** `types.<name>.impact`
decides whether the beam spawns the game's own explosion where it stops (`Body_Explode`), with a
configurable power; the shipped `laser_blunt` demo turns it on and `laser_pierce` leaves it off. The
per-shot line now reports **scan, chain and the time spent inside the game's own damage/blast calls**,
which is what the 1–2 frame hitch question needs — see *Cost, and the stutter*. Per-hit detail lines
are no longer printed for the first 20 shots (only for shots that damage nothing), because writing
dozens of log lines per shot is itself a frame-time cost.

**0.4.1 / 0.4.2 (2026-11-23, fourth play check): where the blast goes is now a choice.**
`types.<name>.impact.at` is `"first"` (the entry point, default) or `"last"` (the far side of a
piercing beam, or wherever the beam stopped). 0.4.0 hard-coded `last`, which put the blast at the
exit side of a ship; 0.4.1 switched to `first`; 0.4.2 makes it configurable and the summary line
says which one it used (`blast yes at N u (first)`).

**0.5.0 (2026-11-23): the beam.** `damage::try_fire` hands `beam` one line per handled shot — muzzle
→ the point where the chain stopped — and a new per-frame tick draws the live lines for their
lifetime. The tick is a second hook, at the entry of `BattleHud_DrawCombatMarkers` (the battle HUD's
marker renderer, i.e. the HUD's draw pass). Lifetime is measured on the game's application clock, so a
faded beam does not age while the game is paused; an entry whose age is negative (a clock reset) is
dropped rather than drawn. No new config keys: `visual.color`, `width`, `glow`, `glow_color`,
`seconds` and `fade` are now used as written.

**0.6.0 (2026-11-23): the launch sound.** A mapped ammo no longer calls `FireShell`, so
`Body_InitializeCannonShell` never runs — and that is where a cannon's own firing sound is played,
which is why every laser shot was silent. `damage::try_fire` now plays the same sound from the same
ammo record, before the chain: `shell_out` + `kind 0xb` + the global node when the local player fires
zoomed in, `shell_enemy` (another ship, zoomed in) and `shell_far` (any ship, zoomed out) + `kind 2`
+ the gun's body otherwise, with vanilla's random variant (`Sound_BuildName(name, 1, 3|4, "_")`) and
its random volume (`HGE.Random_Float(75, 100)`, halved on the player-close branch when the owner test
says so). **No rate limit and no clamp** (user decision: "vanilla has them unlimited, so why should
this mod limit them"). A shot forwarded to vanilla is not doubled — `FireShell` plays its own sound.
No new config keys, and the damage model and the incendiary gate are untouched.

**0.7.0 (2026-11-23): four play-check fixes.**

* **The "is this really a cannon shell" test is gone.** `+0x154` is not a dispatch id in a modded
  install — it is `ammo-extended`'s **caliber**, and a custom round may carry any value (the user's
  `ITEM_AMMO_THRANGXNON` is a 20 mm "laser" with caliber `95`), so the rule refused exactly the
  custom rounds this mod exists for. The `shells` map is already the explicit opt-in: **any ammo the
  config names becomes a laser.** The three ids are still logged once per ammo.
* **Free bodies take damage.** A hit body with a live owner and **no part index** is a missile in
  flight, a shell or debris; it now takes `Body_ApplyDamage` like a part (vanilla's own blast area
  damage does the same) instead of being skipped. Anti-ship missiles can be shot down, and a beam
  no longer loses damage on whatever is flying through it.
* **Only an ownerless *non-part* body is terrain.** The old rule (`owner == 0`) also caught torn-off
  parts whose owner link was gone, so the beam stopped dead on debris — the "lasers sometimes do not
  pierce" report. A part keeps its part index when it comes off, so it is still treated as a part.
  A ship **Frame** (`class 0x1f`, `Frame_Constructor`/`CreateFrameFragments`) is skipped and never
  stops the beam.
* **The blast runs as soon as the chain has finished with its body**, not after the whole chain: a
  later part's death (or explosion) could reap the body that `impact.at` named, which is why the
  explosion sometimes did not appear. `first`/`last` are unchanged.
* **A missing sound variant falls back.** Some sets have fewer clips than the branch asks for
  (`shell_out_small_far` has three files while the far branch asks for four, and
  `ITEM_AMMO_130MM_INC` names it), so a random pick could name a file that does not exist and the
  shot was silent. `playSFXSound_` returning `-1` now retries the other variants of the same set by
  number; the log line says how many were tried.

Everything else from phase 4 — the three branches, the volume, no rate limit — is unchanged, and
0.7.0 adds no config key.

**0.7.1 (2026-11-23): land is not "anything with a null owner".** The 0.7.0 play log
(`Highfleetcracked_2026-11-23_12-23-22.log`, 574 shots) proved the last two remaining reports were the
same bug: a beam that ended on a body with `owner 0, partIndex -1, class 0x0f, health 100.0, oid ""`
— a wreck fragment or a missile, **not** a hill. `Node_AddChild` sets a spawned body's *master*, not
its owner, so debris and missiles are ownerless too, and 0.7.0's `owner == 0 → terrain` rule made the
beam stop on them and give them no damage. Terrain now additionally requires
**`health >= 10000`** (`body::TERRAIN_HEALTH_MIN`): the level's `lnd_*` bodies carry 100000, while
nothing damageable comes near — a missile is 10, a shell 1, the ruin fragments 100, debris particles
1000. Ownerless debris/missiles are free objects again: damaged, and the beam passes through. Every
verdict on a free-body hit is now logged (not only before the first damaged part), so a later log can
answer "did the laser touch the missile?". The per-shot terrain line also prints `owner`.

**0.7.2 (2026-11-23): why an explosion can be invisible.** The 13:22 log (809 shots, `v0.7.1`) shows
`Body_Explode` being reached on **all 262** damaging `laser_blunt` shots — so the call was never the
problem. `Body_Explode` **only draws for the bodies it damages**: it walks the body array, collects
its victims in a scratch vector, and then attaches one `airblast` particle set per victim. Its radius
is `2·√power` **game units**, and the vanilla 130 mm value `power 20` is only **8.9 u** — less than
the distance between the centres of two large parts (an armour plate is ~20 u long). A blast centred
on a part therefore often reaches **no other body** and draws nothing, while the log still said
`blast yes`. 37 of those 262 calls finished in <25 µs (the array walk with no victims) against
~137 µs average for the ones that did work.

The summary line now carries the victim count, and a blast that finds none is **retried once at 4×
power** (radius ×2) — the first call damaged nothing, so there is no double damage — and says so:

```
blast yes at 524.7 u (first: 7 bodies)          normal
blast yes at 296.1 u (first: 0 bodies, retried at 4x power, 3 bodies)
blast FIZZLED at 296.1 u (first: Body_Explode refused)   has never been seen
```

`explosive_power` is still the lever for the normal case: a bigger power is a bigger radius *and*
more blast damage. *(The 4× retry described above was removed again in 0.8.2 — its victim counter
read `0 bodies` on every blast in every log, so it was not a rare fallback but a permanent 4×
multiplier. See 0.8.2 below.)*

**0.8.0 (2026-11-23): `pierce.through_terrain`, and a blast on a body the beam kills.**

* **`pierce.through_terrain`** (default `false`) — when `true`, the level's ground takes no damage
  **and does not stop the beam**: the shot and its damage carry on behind the hill. The beam is drawn
  through it (it ends where the damage ended, or at `range`). Off by default, because a hill blocks a
  shot in vanilla. This is the only way to make a type "pierce anything": with it off, terrain is the
  one thing that always ends a beam.
* **A blast on a body the hit kills now runs *before* the damage.** The 21:31 logs show
  `blast FIZZLED — Body_Explode refused` on exactly the shots where the beam killed the target part:
  the death path clears the body's `reflected` byte and `Body_Explode` refuses a body that is not
  reflected. When the hit is predicted to be lethal (`damage >= health`), the blast is spawned first,
  while the body is still live and reflected; otherwise it stays right after the damage call.
* **What the 21:31 logs also showed:** every blast that did run found **`0 bodies`** in range, even
  after the retry that existed then (`blast yes at 508.1 u (first: 0 bodies, retried at 4x power, 0
  bodies)`; that retry is **gone in 0.8.2** — see below). The blast's radius at `power 20` is 8.9 u and
  these hits were on 2x2 truss / armour parts, whose neighbours' centres are farther than that.
  **`explosive_power` is the lever** (radius = 2·√power: 5 → 4.5 u, 20 → 8.9 u, 100 → 20 u, 400 → 40 u);
  it raises the blast damage too.
* **`laser_pierce_2` "refusing to pierce" is its own config**, not the chain: in the 21:31 log the
  only stop reasons for it were `max_parts` (`max_parts: 10` — ten damaged parts end the beam) and
  `spent damage` (`damage_decay_per_unit: 0.005` with `damage 10` reaches zero at **2000 u**, inside
  its 15 000 u range). For "pierce everything in range": `max_parts: 0`, `damage_decay_per_unit: 0`,
  `stop_at: []`, `through_terrain: true`.

**0.8.1 (2026-11-23): the summary line now says where the beam went.** The 21:55 / 21:58 logs show
`laser_pierce_2` piercing normally whenever the ray crosses a ship — **17 hits / 17 damaged**,
16/15, 14/14, 10/10 — and every one of its 54 shots stopped with `the end of the hit list`, i.e. no
`max_parts`, no `terrain`, no `spent damage` stop at all (the type is `max_parts 0`, `decay 0`,
`through_terrain true`). What the same log also shows is a large share of shots with **1 hit or 0
hits**: the ray met one body or nothing. To stop guessing at "it sticks on the engine", both ends of
the chain are now in the summary:

```
… 17 hit(s) [17 part, 0 object, 0 frame, 0 terrain, 0 dead], 17 damaged, … first "MDL_ENGINE_05"@965.7 (joint), last "MDL_ARMOR1X1_03"@1008.8 (mounted); stopped: the end of the hit list; …
```

* `(joint)` = the body owns its **own** b2 body (an engine, a gun, a leg, a free body); `(mounted)` =
  a part whose fixture lives on the ship's Frame (armour, truss, bridge, …). If a shot ever ends on
  `(joint)` with one hit, the log says so outright.
* The tally gained a `terrain` bucket, so a `through_terrain` beam that crosses ground is counted.
* A **piercing** type whose ray found three hits or fewer prints **every** hit with its oid and
  verdict (the "it stopped on the engine" case), not only shots that damaged nothing.

**0.8.2 (2026-11-23): the blast is spawned exactly once, at the configured power.** The 0.7.2 4×
retry is gone. It was added for a real problem — `Body_Explode`'s radius is `2·√power` game units
(`power 20` = 8.9 u) and it attaches its particles to the bodies it damages, so a blast that reaches
nobody draws nothing — but its trigger was a victim count read out of the game's scratch vector
(`0x1403ef050..0x1403ef058`) that reported **`0 bodies` on every single blast** in every log, including
shots that damaged 23 parts of one ship. Either the game clears that vector before returning or it is
not the vector the particles use; either way the retry was not a rare fallback but a permanent 4× power
multiplier (and a second area-damage pass). Now:

* one `Body_Explode` call per `impact`, with `explosive_power` exactly as written;
* the summary says `blast yes at N u (first)` — no victim count, because the count was never
  trustworthy;
* `FIZZLED` still appears if `Body_Explode`'s own guard refuses (it has not since 0.8.0's lethal-first
  ordering).

**The lever is `explosive_power`, and the radius is small:** `2·√power` game units, so **5 → 4.5 u**,
20 → 8.9 u (a vanilla 130 mm HE shell), 50 → 14.1 u, 100 → 20 u, 400 → 40 u, 900 → 60 u. A power that
small can legitimately draw nothing on a large part.


---

## 2. Play checks and the per-shot log

### Phase 2 play check

Fire `MDL_CANNON_130` with **plain** ammo:

* at **close** range at an enemy — the shot must damage **several parts in a row** (the log lists
  part after part, including internals behind the hit armour), and (phase 2 had no sound yet) **no
  shell or tracer** must appear;
* at **long range** — the same shot must do visibly less per part (decay `0.0033`/unit: 8 → 4.7 at
  1 000 u);
* same gun with **`_PROXY`** (`laser_blunt`, pierce off) — exactly **one** part is damaged and the log
  says the beam stopped there;
* a shot into the **ground** — nothing is damaged and the log ends with `terrain (class …); the beam
  ends`;
* nothing past **`range`** is touched (`laser_pierce` 3 000 u, `laser_blunt` 1 200 u);
* a **non-piercing** type with `impact.explosion: true` — the shot must make the game's own explosion
  where it hit (`blast yes` in the log), and the parts are not just quietly detached.

### Phase 3 play check (the beam)

Fire `MDL_CANNON_130` with a type that has `visual.glow: true` and `seconds >= 0.08`:

* the beam **starts at the muzzle** and **ends at the last part it reached** (damaged or merely blocked
  there) — for the user's `laser_blunt` (`#FFAA00`, `width 2.0`, `seconds 1.0`) that is a thick orange
  line that lasts a full second, so it is easy to see; the log's `beam to N u` must match where the
  line stops;
* a shot that hits **terrain** ends there, a shot that hits **nothing** runs the full `range`;
* the line is **brightest on the frame it is fired** and fades out (the glow is a wider, dimmer line behind
  the thinner core; the width itself does not change with age);
* a shot that is **forwarded to vanilla** (an ammo not mapped in `shells`) must show **no beam** —
  and the **enemy's** laser shots must show a beam too (they use the same gun path);
* **two guns firing at once** show **two** beams;
* **pausing** freezes a live beam instead of letting it fade (the lifetime is on the game clock);
* the game must not stutter with no laser firing at all — the draw tick returns immediately while
  no beam is live and does no body walk.

`visual.color`, `width`, `glow`, `glow_color`, `seconds` and `fade` are the only controls; phase 3
adds no config key and does not need a config change to test.

### Phase 4 play check (the launch sound)

Fire `MDL_CANNON_130` with a mapped ammo:

* **zoomed in**, player's own ship — the gun's own close bang (`shell_out_*`), and the same shot
  **zoomed out** — the distant variant (`shell_far_*`); zoom `>= 4` is "far" (vanilla's `zoom > 3`);
* an **enemy** ship firing a mapped laser — audible with its own `shell_enemy_*` close set at any
  zoom while close, `shell_far_*` when zoomed out (the enemy uses the same `FireGun` path);
* a shot **forwarded to vanilla** (an ammo not in `shells`) is **not** doubled: `FireShell` plays its
  own sound and this mod must stay silent for it;
* the sound never plays without a shot, and a shot never plays twice;
* the log names the same variant that is heard:
  `hf_hitscan: ammo "ITEM_AMMO_130MM": launch sound "shell_out_med_02" (kind 0xb, player close, zoom 2, volume 44.3, 1 = queued)`.

No rate limit and no clamp: a 30 mm laser at full rate sounds exactly like vanilla's 30 mm. `0.6.0`
adds no config key.

An ammo whose chosen sound set has a hole (fewer clips than the branch asks for) is retried on the
other variants, and the log line then ends with `, N variants tried`; without that fallback such a
shot was simply silent (`ITEM_AMMO_130MM_INC`'s `shell_out_small_far` has three files, the far branch
asks for four).

The log, per laser shot (verbose):

```
hf_hitscan: ammo "ITEM_AMMO_130MM": +0x154 20 (0x14), +0xcc 0x14, +0x150 100 -> laser type "laser_pierce"
hf_hitscan: ammo "ITEM_AMMO_130MM": launch sound "shell_out_med_02" (kind 0xb, player close, zoom 2, volume 44.3, 1 = queued)
hf_hitscan: shot 1: hit 1: dist 159.0 part 172 oid "MDL_ENGINE_04" cat 0x00000100 damage 7.47 applied (health 92.5)
hf_hitscan: shot 1: hit 2: dist 171.1 part 127 oid "MDL_LEG_02" cat 0x00002000 damage 7.44 applied (health 72.6)
hf_hitscan: shot 1: "MDL_CANNON_130" + "ITEM_AMMO_130MM" -> "laser_pierce": 6 hit(s) [4 part, 0 object, 1 frame, 0 terrain, 1 dead], 4 damaged, 0 blocked for 29.7; first "MDL_FERMA2X2_02"@159.0 (mounted), last "MDL_LEG_02"@171.1 (joint); stopped: stop_at at 214.0 u; beam to 214.0 u; blast no; 137 slot(s) / 76 candidate(s) / 113 cast(s) in 412 us (chain 88 us, of which 61 us in the game's damage/blast calls)
```

The **launch sound line is printed once per (ammo, branch)** — not per shot — so the play check is
verifiable from the log without writing dozens of lines a second. The last number is
`playSFXSound_`'s return (`1` = queued, `-1` = dropped, e.g. the name is not in `sound.res`).

Every ammo that fires gets **one** resolution line (mapped to a type, or "not in `shells`"), so an
ammo that stays vanilla always says so once. Since 0.7.0 there is no "not a plain cannon shell"
refusal — the `shells` map is the only opt-in — so a mapped ammo always resolves to its type.

Per shot, the summary line carries a tally of what the beam met (`part` / `object` / `frame` /
`terrain` / `dead`), **the first and last body it touched** (`"oid"@N u (joint|mounted)`, 0.8.1), how
much was damaged or blocked, the **stop reason** - `terrain`, `stop_at`, `pierce off`, `max_parts`,
`range`, `spent damage` or `the end of the hit list` - where the beam is drawn to (`beam to N u`),
whether a blast was spawned, and three timings (see *Cost, and the stutter*). **Until something in that
shot has been damaged, every individual hit is also printed with its verdict**, and so is **every hit of
a piercing shot that found three bodies or fewer**, so a shot that seems to do nothing always explains
itself; a shot that damages prints only its summary line (writing dozens of per-hit lines per shot is
itself a frame-time cost). All of it is
behind `verbose_logging`.

**With `verbose_logging: false` nothing is written per shot at all** — only the launch-time lines
(version, patch-site verification, the config dump: one line per type and one per shell binding — 17
lines for the current 5 types / 6 shells) and one resolution line the first time each ammo fires. The
per-shot set is the switch, not a build difference: **phase 6 is not going to delete the diagnostics**,
it ships the release config with the flag off.

**`beam to N u` is the beam's end point**, and it is decided by the chain, never by the draw pass:
the terrain hit when a hill stopped the beam, otherwise the last body the beam reached (for
`pierce: false` that is the one part it hit; for a piercing type the far side, or the body it stopped
on for `stop_at`/`max_parts`), and the full `range` when it hit nothing at all.

**How to read a "refused" shot:** `frame` is a ship's Frame (a structural body vanilla never damages
either; its mounted parts are the fixtures that take hits), `object` is a free body that *was*
damaged (a missile, a shell, debris), `dead` is a body of an already-destroyed part (the game's own
`health <= 0` guard — vanilla shells would not damage it either), `blocked` is the armour threshold
(`ignore_armor_threshold: false`), and `terrain` means an ownerless, non-part body with the land's
`100000` hp was in the way. None of those is a bug, and none of them is visible from the beam, which
is why the log names them. The terrain line carries the body's `class`, `partIndex`, `owner`,
`health` and `oid`, so an ownerless body that is *not* a hill can be recognised from the log. A hit on
a **free object** (a missile, a shell, debris) is always printed with its health and verdict, even on
a shot that has already damaged a part — it is the one thing the log must never hide.

---

## Engine notes (Steam 1.163, image base `0x140000000`)

### A hit is a fixture, and a fixture belongs to a part (`b2Fixture.m_userData`)

A ship is **one Frame body plus a few free bodies**. `Frame::Reflect 0x14011cbc0` creates the frame's
b2 body; `Body::Reflect 0x14004b010` then gives every child part either

* a **fixture on the Frame's body** — when the part's master is the Frame (`Body_CreateFixture
  0x14004b530`), or
* its **own b2 body + fixture** — only when the part's master is `GlobalNode_` (a gimballed or torn-off
  part: engines, guns, legs, debris).

`Body_CreateFixture` builds a `b2FixtureDef` whose **`userData` is the part's own `Body*`**, and
`b2Fixture::Create 0x140320b30` copies it (`this->m_userData = def->userData;`). The 1.163 fixture
layout, read in Ghidra on 2026-11-23:

| offset | field | note |
|---|---|---|
| `+0x00` | `m_density` (f32) | |
| `+0x08` | `m_next` | the list is `*(b2Body+0x70)` |
| `+0x10` | `m_body` | this is the HUD's transform argument: `*(fixture+0x10) + 0xc` |
| `+0x18` | `m_shape` | `RayCast` at shape vtable `+0x20` |
| `+0x20` / `+0x24` | friction / restitution | |
| `+0x28` / `+0x30` | proxies / proxyCount | |
| `+0x34` / `+0x3a` | filter / isSensor | |
| **`+0x40`** | **`m_userData` = the part `Body*`** | 0x48-byte allocation |

That is why a ray cast against the body array alone only ever meets the Frame (`partIndex -1`) and
the jointed parts. `ray::cast` therefore **returns the part from `m_userData`** (and the array body
itself when the field is 0, which is the case for free bodies, terrain, shells and effects). This
also means one ship presents ~100–200 fixtures on one frame — the `FIXTURES_PER_BODY_MAX` guard is
8192 for that reason, and the phase 1 "108–225 casts per shot" were mostly those part fixtures.

Two consequences for the damage chain:

* `IsActive` is no longer required for a hit part — a mounted part has no `b2Body`, and the chain only
  needs "this memory is still a live `Body`" (`body::plausible`: the first word is a vtable inside the
  module image, which rejects freed heap memory) plus the game's own `health > 0` / `healthLock == 0`
  guard.
* The Frame itself is skipped (`class 0x1f`) and does not stop the beam; a free body with an owner —
  a missile, a shell, debris — takes damage and does not stop it either. Only an ownerless body with
  no part index (the ground) ends a beam.

### The two patched sites

```
140032f99  E8 ?? ?? ?? ??   CALL 0x1400300d0        <- replaced, 5 bytes, target verified at install
140032f9e                  JMP  0x140032ef2        ; epilogue, AL = FireShell's bool
```

```
14029c5c0  48 8b c4 / 48 89 58 10                   <- replaced, 7 bytes, prologue verified at install
14029c5c7                                          ; continuation (draw pass, once per frame)
```

Verified at install: the fire site's byte is `E8` and its `rel32` resolves to `0x1400300d0`; the draw
tick starts with exactly `MOV RAX,RSP` + `MOV [RAX+0x10],RBX`. Anything else — a different opcode, a
different target, a different prologue, unreadable memory for 2 s — and **no patch is applied at that
site**. A refused fire site leaves the mod uninstalled (the game runs vanilla); a refused draw tick
leaves it installed with one hook and no beam. Both sites are retried 20 × 100 ms because the Steam
build unpacks itself while the modloader starts.

### The trampoline is `call gate; jmp 0x140032f9e`, not `call gate; ret`

Roadmap §5.2 (and hazard H1) say the trampoline "ends with `RET` because the patched instruction was
a `CALL`". **That is wrong and this build does not do it.** `patchy` replaces the site with a
five-byte `JMP`; the `JMP` pushes nothing, so at trampoline entry `[RSP]` is whatever lies beneath
`FireGun`'s frame. A `RET` there jumps into that value, not to `0x140032f9e`. The trampoline
therefore names its continuation explicitly:

```
call  [rip+2]              ; absolute_call(fire_gate)
jmp   +8                   ; (skip the inline pointer)
<8-byte gate address>
E9 <rel32>                 ; relative_jump(0x140032f9e)  -> FireGun's epilogue with AL = bool
```

This is also why the gate is a **bare eight-argument call**, not a `patchy` `preserved_call` /
`patch_call`: those save and spill volatile state, so a callback would receive `RCX`/`RDX`/`R8`/`R9`
but *not* `FireShell`'s four stack arguments. A bare `call` leaves the caller's shadow space and
stack slots exactly where `FireShell` expects them.

> Side note for `hf_gun_mechanics`: its per-gun fire delay uses the `absolute_call(gate); ret` shape
> at the two `CALL FireGuns` sites (`0x14007f7be`, `0x14007f7fe`). By the argument above that `RET`
> cannot land on the loop continuation. It was never verified in play (v0.28.0), and it is worth
> checking before reusing that pattern.

### The muzzle transform is vtable `+0xe8`, not `+0x18` (roadmap correction)

Read in Ghidra on 2026-11-23, the call inside `FireShell` is:

```
1400301a6  CALL qword ptr [RAX + 0xe8]     ; RAX = *(Body*) = vtable; RCX = gun; RDX = &{0,0}
1400301f2  CALL qword ptr [RAX + 0xe8]     ; same slot with *fire_dir* = {0,1}
1400301fe  SUBSS XMM11, XMM8               ; d = p - o, not normalised
140030224  MOVSS XMM7, [0x140352818]       ; 10.0
14003022c  MULSS XMM0, XMM7                ; muzzle = o + 10*d
```

Measured in play (phase 1): `o` equals `gun+0x128` to the printed precision and `|d| = 1.0000`.
`body::muzzle` therefore reproduces `FireShell`'s numbers instead of re-deriving a rotation, and the
direction is **not** normalised.

### The damage chain (phase 2)

`Body_ApplyDamage` = **`0x1400519d0`** (vtable `+0x210`). Re-read in the 1.163 program before this
build; the prototype is

```c
undefined8 Body_ApplyDamage(Body *this, float damage, char directHit, undefined8 param_4,
                            char hullContact, char defer)
```

and it is called as `extern "C" fn(*mut u8, f32, u8, u64, u8, u8) -> u8` with `param_4 = 0`,
`hullContact = 0`, `defer = 0` (on x86_64/MSVC the two trailing bytes land in the caller's stack
slots after the shadow space, which is what the C prototype does). It returns `0` when nothing was
applied — `damage == 0`, `healthLock (+0x2d0) != 0`, `health (+0x2c8) <= 0`, or, on a direct hit,
`damage < GetStats(partIndex)->m_mdl_armor (+0xc4)`. `directHit = 0` is the blast-style path with the
threshold bypassed: that is `ignore_armor_threshold`. Its **player-ship ×0.6** applies to every
caller and is correct; do not "fix" it.

`GetStats(int)` = `0x140281e00`, `Stats.m_category` at `+0xb0`. The chain:

| step | rule |
|---|---|
| distance | `fraction × range` (the scan's fraction is unitless and already clipped) |
| damage | `damage − damage_decay_per_unit × distance`; `≤ 0` **ends the beam** |
| range | `distance > range` ends the beam (belt and braces) |
| terrain | **ownerless, not a part, and `health >= 10000`**: `+0xb8 == 0` **and** `partIndex` outside `0..4096` **and** the level's indestructible health → no damage, beam ends (**unless `pierce.through_terrain`**, which lets the beam carry on behind the hill) |
| a ship Frame | `class (+0x28) == 0x1f` (`Frame_Constructor`, `CreateFrameFragments`) → no damage, **does not** stop the beam |
| a ship part | `partIndex` inside `0..4096`; the fixture resolves to its part through `b2Fixture.m_userData (+0x40)` — see *A hit is a fixture* |
| a free object | no part index, a live owner and not a frame: a missile in flight, a shell, debris → **takes damage** like a part (vanilla's blast area damage does the same) and never stops the beam |
| dead/locked | re-checked immediately before every call (H10/H16) → skipped |
| apply | `Body_ApplyDamage(body, damage, direct = !ignore_armor_threshold, 0, 0, 0)` |
| incendiary | `+(float*) (body+0x2f0) += incendiary` for every body the call actually damaged |
| stop | `pierce` off → after the first body; else `category & stop_mask` → stop; else `max_parts` (0 = unlimited). A free object has no category, so no `stop_at` can match it |

The order matters and changed in 0.7.0/0.7.1: `partIndex` is read **first**, so a torn-off part
(which keeps its part index when it comes off) is never mistaken for terrain, and **only** a body that
is not a part, has no owner **and** carries the land's `100000` hp is a hill. The owner alone is not a
terrain test — `Node_AddChild` gives spawned bodies a *master*, not an owner, so debris and missiles
are ownerless as well.

Own parts are excluded by **owner**, not by the `+0x224` flag: [`ray::cast`](src/ray.rs) skips every
body whose `+0xb8` is the firing gun's creature. That is the same set the HUD's flag covers for
**parts**, and it removes hazard H9 entirely, because this build never writes the aim-ray exclusion
flag, so it cannot leak. Shells, missiles and debris are attached to `GlobalNode_` instead of a
creature (`(*GlobalNode_+0x50)(GlobalNode_, body, 0, 0)` in `FireShell` and in `Body_DieExplode`), so
they are not excluded and — since 0.7.0 — take damage, which is exactly what makes a missile killable.

### Armour is a part stat, and it is what `ignore_armor_threshold` bypasses

`m_mdl_armor` (**`Stats +0xc4`**, set per part in `Libraries\OL.seria`) is a **floor for a direct
hit**, not a plate: `Body_ApplyDamage` returns 0 for the whole *part* when `damage < m_mdl_armor`.
The whole game is four values (measured from `OL.seria`, 2026-11-23):

| floor | parts |
|---|---|
| **1** | fuel tanks, ammo racks, missiles/nukes, aircraft, jammer/IRST |
| **3** | guns, FSS, protector, evac, RSZO |
| **5** | armour plates, bridge, hull/truss/hardpoints (`MDL_FERMA*`, `MDL_HARD*`), blocks, engines, legs, radar/SPO/FCR, generator, quarters, cargo, deck, flares |
| **10** | bombs |

So a type below `1` can never damage anything (a 0.5-damage config logs every hit as `blocked`), and
`ignore_armor_threshold: true` (`directHit = 0`) is the only way to skip the floor. The beam is **not**
stopped by a blocked part — `pierce`/`stop_at`/`max_parts` are the only stop rules, and `stop_at`
still matches a blocked part's category — so a low-damage laser draws straight through a ship it
cannot scratch.

### Fire — the dose, and what can actually burn

`incendiary` is written to **`Body +0x2f0`**, once per shot and only for the parts
`Body_ApplyDamage` accepted (the "incendiary gate" — a blocked part gets no dose). The rest is the
game's, per frame:

* `World_ThermalPass` (`0x140127810`, from `SomethingGameTask`) runs `Body_ThermalStep`
  (`0x140055870`) on every active body with `dt_step = (dt/n)·4`, `heat = 1/n`, `ambient = 0`,
  `k = 200` (`n = ceil(dt/0.2)`), so **`T(+0x2ec) += dose(+0x2f0)/n`** minus the cooling term
  `0.0202·T·dt_step` (≈8 %/s at 50 fps) — and then **zeroes `+0x2f0`**. One dose is converted once;
  it does not accumulate in the world.
* `Body_BurnUpdate` (`0x140054850`, from `Part_Tick`) **returns on its first check when `burnHp` is
  0**, and otherwise ignites the part once `T >= m_burn_ignite`, burns `min(burnHp, 0.25·dt)` HP per
  tick and adds `dt × m_burn_heat` to `T` (which is what "feeding the fire" is).

| part | `m_burn_hp` (`parts.seria`) | `m_burn_ignite` | `m_burn_heat` |
|---|---|---|---|
| `MDL_FUEL_01/02/03` | 30 / 400 / 30 (max 100 / 800 / 100) | 200 | 200 (runaway) |
| `MDL_ENGINE_0x` | 10–40 (`01`/`02` 40, `03`–`06` 10) | 400 | 10 |
| bulk trusses/hardpoints (`MDL_FERMA2X2/4X4`, `MDL_HARD2X2`), blocks, bridge, guns, ammo racks | 5–10 | absent | absent |
| **armour** (`MDL_ARMOR1X1_*`, `MDL_ARMOR2X1_01`) | **absent** | — | — |

A part with `m_burn_hp` but no `m_burn_heat` **catches fire and burns, but does not feed the flames**
(the class default for `m_burn_ignite` decides how much heat it needs, and its own temperature only
cools afterwards) — that is what a truss does in play. Armour is the block-family part whose records
carry **no `m_burn_hp`**, and it is the one thing that never catches fire whatever the dose
(confirmed in play 2026-11-23 with `laser_incen` at `incendiary 10000`). Some thin 1x1
truss/hardpoint records carry no `m_burn_hp` either; the bulk ones do.

### The impact blast — `Body_Explode` (`0x140029b60`)

A vanilla shell looks like an explosion because the **shell body** explodes: `Projectile_DetonateOnImpulse`
calls `Body_DieExplode` (`0x140023ac0`), which calls `Body_Explode` (`0x140029b60`) when the body's
`explosivePower (+0x2dc)` is non-zero. `Body_Explode` computes the radius from that power
(`radius = 2·√power`, ×0.66 when the source is not the player's ship), gathers the live bodies inside
it, calls `Body_AreaDamageFromBlast` with falloff and armour shadowing, applies direct damage and
knock-back to the hit body, queues secondary ignition and spawns the fire particles. It **does not
kill** the source body — `Body_DieExplode` does that separately.

A part body has `explosivePower == 0`, so a part that our beam kills just dies: the health path sets
hp to 0, the game tears the part off the frame, and there is no blast. That is exactly the "it simply
disconnects the hit part" behaviour of 0.3.x. `types.<name>.impact.explosion` fixes it by calling
`Body_Explode` on the part selected by `impact.at` with a temporarily stamped power:

```rust
let previous = *(f32*)(part + 0x2dc);
*(f32*)(part + 0x2dc) = explosive_power;   // the config value, 20 = a 130 mm HE shell
Body_Explode(part);                        // 0x140029b60
*(f32*)(part + 0x2dc) = previous;          // never leave a part explosive
```

The blast is the game's own — same particles, same area damage, same player ×0.6 rule — and it is
centred on that body, i.e. within its own size of the intercept point. **`at`:** 0.4.0 hard-coded the
*last* part the beam damaged, which put the blast at the far side of a ship for a piercing beam
(reported in play 2026-11-23); 0.4.1 switched to the *first* part; 0.4.2 makes it `"first"` (default,
entry point) or `"last"` (far end) per type.

**When it runs (0.7.0):** as soon as the chain has finished with the body the config names, i.e.
immediately after that body's `Body_ApplyDamage` call — not after the whole chain. Up to 0.6.x the
blast was spawned at the end, and a later part's death or explosion could reap the body in between,
which is why the explosion sometimes did not appear at all (reported in play). A body that died from
the very hit that selected it is fine: the allocation is still there and vanilla explodes bodies it
has just killed. A `last` target the loop cannot predict (the beam ended at the end of the hit list,
at `range` or on spent damage) still falls back to the post-chain call, plus a log line if the body
is gone by then.

**What it actually draws (0.7.2):** the radius is `2·√power` **game units** (`power 20` > 8.9 u,
`power 50` > 14.1 u), and the visuals are attached to the **victims**: `Body_Explode` walks the body
array into the scratch vector `begin 0x1403ef050` / `end 0x1403ef058`, then loads
`Media/Prt/airblast_01.psi` **once per victim**. So a blast that reaches no other body (a lone part,
a missile in empty space, an outer plate 20 u from its neighbours) applies its area damage to nobody
and draws nothing — vanilla behaves the same; a shell just explodes at the hull surface, where a part
is within a few units. **The mod spawns the blast once, at the configured `explosive_power`.** The
0.7.2 4× retry that tried to compensate was removed in 0.8.2, because the victim count it read out of
that vector was `0` on every blast in every log (including 23-part passes), so it acted as a permanent
4× multiplier rather than a fallback. The count is no longer printed; `blast yes at N u (first)` is.

### The beam — `Gfx_DrawWorldLine`, the draw tick and its ABI (phase 3)

`Gfx_DrawWorldLine` = **`0x140248ba0`**, the game's own line primitive (`Hud_DrawAimRay` uses it for
the aim line and the X marker):

```c
undefined Gfx_DrawWorldLine(Vec2 from, Vec2 to, float width, uint argb, undefined4 param_5,
                            char alreadyScreen, float cornerField8, uint argb2, float param_9);
```

It converts both points with the camera globals (unless `alreadyScreen != 0`), drops a line that is
fully off-screen, builds a `width`-wide quad and draws it through HGE's quad renderer (`HGE` vtable
`+0x2e0`, `Gfx_RenderQuad` `0x14016cc80`) — so it must be called while the frame is being rendered,
which is exactly where the game's own calls are. The colour is **ARGB** — the alpha lives in the top
byte, so a fade is a different colour value, not another argument.

The five trailing arguments are **not** the same at every call site: `Hud_DrawAimRay`'s three calls
use `(1, 0, 0.0, 1, 1.0)` (`alreadyScreen = 0` = world coordinates), while the enemy pre-fire laser
in `BattleHud_DrawCombatMarkers` passes `param_5 = 3`, `argb2 = 0` and a per-call end colour/float —
that is a gradient quad. `param_5`/`cornerField8`/`param_9` are still unresolved (roadmap §5.9), so
this mod copies the **aim-line** form verbatim: solid, both ends the same colour, world coordinates.

The ABI was read out of the 1.163 program (2026-11-23), not assumed — hazard H15 is a crash, not a
cosmetic problem:

| argument | register / slot (at the callee) |
|---|---|
| `from`, `to` | `RCX`, `RDX` (the body stores `RDX` then `RCX` into `[RSP+0x20]`/`[RSP+0x28]`) |
| `width` | `XMM2` (`MOVAPS XMM14, XMM2`) |
| `argb` | `R9D` (`MOV EBX, R9D`) |
| `param_5`, `alreadyScreen`, `cornerField8`, `argb2`, `param_9` | `[RSP+0x28]`, `[RSP+0x30]`, `[RSP+0x38]`, `[RSP+0x40]`, `[RSP+0x48]` |

Both `Vec2`s are passed **by value** in integer registers, which is exactly what an `extern "C"`
declaration produces; a `*const Vec2` would be a different function.

The draw tick is the **entry of `BattleHud_DrawCombatMarkers` `0x14029c5c0`** — per frame, in the
battle HUD's draw pass. It is the battle marker renderer: it calls `Hud_DrawAimRay` (the player's aim
line and X marker) twice, and it draws the enemy's pre-fire laser itself (red `0xFFDD2727`, cyan
`0xFF00E4FF`, both present in its body and handed to `Gfx_DrawWorldLine`). `Gfx_DrawWorldLine` is
never hooked (recursion) and nothing is drawn from the fire path (wrong phase). The tick's first act
is "is the beam list empty?", so a frame with no live beam returns after that check — no clock read,
no body walk, no draw call.

```text
14029c5c0  48 8b c4        MOV  RAX, RSP          ; replayed by the trampoline
14029c5c3  48 89 58 10     MOV  [RAX+0x10], RBX   ; replayed by the trampoline
14029c5c7                   <-- the trampoline continues here
```

Seven bytes, two complete position-independent instructions, and the function takes no arguments. One
detail that is easy to miss: `patchy`'s preserved call pushes seven registers (56 bytes) and
allocates a `0x88` frame — 192 bytes, i.e. it leaves `RSP` unchanged mod 16 — so it is written for a
hook **after** a prologue, where `RSP` is 16-byte aligned. This site is a *function entry*, where
`RSP` is 8 mod 16, so the trampoline does `SUB RSP, 8` around the call (`ADD RSP, 8` afterwards).
Without it the hook — and `Gfx_DrawWorldLine` under it, which spills `XMM6` with `MOVAPS` — would
run 8 bytes off the ABI's alignment.

Lifetime is the `HGE` application clock (`*(f32 *)(HGE + 0xd68)`, `Timer_GetTime`), never
`Instant::now()`: pausing freezes it, and a battle restart resets it. An entry whose age is negative
is from before a reset and is **dropped** (hazard H11), so a restart cannot leave a ghost beam.

**Observed in play (2026-11-23):** the line is **not** dimmed by the screen-darkness/blackout overlay
— it stays at its configured brightness while the screen goes dark. That is the same primitive the
game's own aim line uses, and the user likes the effect; do not "fix" it unless asked.

### The launch sound — `Body_InitializeCannonShell`'s three branches (phase 4)

A cannon's firing sound is played at the end of `Body_InitializeCannonShell` (`0x140052a80`), the
helper `FireShell` uses to build a shell — so a mapped ammo, which never calls `FireShell`, has to
play it itself. The disassembly (1.163, re-read 2026-11-23) is two nested tests and three calls:

| branch | test | ammo field | variants | `kind` | node | call |
|---|---|---|---|---|---|---|
| player, close | `player_ship` **and** `zoom <= 3` | `+0xf0` `shell_out` | 3 | `0x0b` | `*(usize*)0x147fc6f88` (`GlobalNode_`) | `0x140052e71` |
| enemy, close | **not** `player_ship` **and** `zoom <= 3` | `+0x110` `shell_enemy` | 4 | `2` | the shot's `Body` | `0x140052f11` |
| far | `zoom > 3`, either side | `+0x130` `shell_far` | 4 | `2` | the shot's `Body` | `0x140052fa4` |

```text
name    = Sound_BuildName(<the field's own buffer>, 1 /* random variant */, variants, "_")
volume  = HGE.Random_Float(75.0, 100.0)      ; player-close only: x 0.5 when FUN_140073340(owner)
playSFXSound_(kind, name, node, volume, 1.0, NULL, NULL, 0, 0, 0, 0, 0.0, 0)
```

All of it was read out of the program, not assumed: `Sound_BuildName` (`0x1401b1fb0`) formats into
its global string and returns that `c_str` (used immediately, never cached) and replaces `number`
with an HGE random when its third argument is non-zero; the volume bounds are the globals
`0x140354850` = `75.0` and `0x1403524e0` = `100.0` (read from the binary); the halving factor is the
literal `0.5` at `0x1403527a4` versus `1.0` at `0x1403524d8`, chosen by `FUN_140073340(owner)` — a
25-byte leaf: `[owner+0x48] != 0` **and** `40.0 <= f32[owner+0x15f8]` (`0x140354834` = `40.0`).
`player_ship` is `Profile_IsPlayerShip(Get_Profile_(), *(usize*)(gun+0xb8))`.

Two deliberate choices: the `kind 2` branches pass the **gun's** `Body*` as the node instead of the
freshly created shell (this mod creates none; the gun is at the same place at t = 0 and is a valid
`Node`), and the player test is the game's own function, so a co-op ally is not mistaken for the
local player. The four sound fields are `EscadraString`s 0x20 apart: `shell_in +0xd0` is the
magazine-load sound and is not part of firing. The mod's entry check is
`playSFXSound_`'s first eight bytes (`48 8b c4 48 89 58 08 48`); a different build reports once and
skips the **sound only**. The call is wrapped in its own `catch_unwind` inside the fire path, so a
sound failure can never cost the shot.

### What the ray does, byte for byte (`Hud_DrawAimRay`, 0x140248490)

| step | code |
|---|---|
| body array | `0x143a33c90` is the `Body*` array **itself** (`LEA R14,[0x143a33c90]`, then `[R14]`), count at `0x143a51150` |
| filters | `IsActive` = `[[body]+0x78]`, `body+0x28 & 0xf == 0xf`, `body+0x178` (b2Body) non-null, `body+0x1f0` (`m_layer`) non-zero |
| fixture walk | first fixture `*(b2Body+0x70)`, next at `fixture+8`, shape at `fixture+0x18`, transform = `*(fixture+0x10) + 0xc` |
| cast | shape vtable `+0x20` = `RayCast(out, in, transform, 0)`; `in = {p1, p2, 1.0}` in b2 units (`× 0.05`), `out = {normal, fraction}` (fraction at `out+8`) |

The scan keeps every hit, sorts by fraction and drops duplicate bodies (a part can own several
fixtures).

### Cost — one `VirtualQuery` per body, not per field

Phase 1 measured **3.0–5.6 ms per shot** on 136–205 live bodies, and the raycasts were not the cost:
the probe's per-pointer `VirtualQuery` validation was (4–5 queries per body plus one per hit, plus a
query per part for the `+0x224` flag). Phase 2:

* scans **only** when the loaded ammo is bound in `shells`;
* proves **one** extent per body (`0x1f4` bytes, which covers every field the scan reads) and then
  reads the fields directly;
* resolves `IsActive` and the shape `RayCast` entry **once per distinct vtable** instead of once per
  body;
* no longer writes the `+0x224` flags (owner-based exclusion), which alone was ~30–100 queries;
* keeps one `readable_extent` for the whole body array.

Everything Box2D owns — the `b2Body`, its fixture chain, the shapes' vtables — is trusted, exactly as
the game's own aim ray trusts it for all 15 000 slots every frame; the world cannot change under the
scan because it runs synchronously inside `FireGun` and this mod destroys nothing until the chain has
collected its hits. The one thing that *can* be stale is an array slot, and that is the one check
kept.

The per-shot summary line reports the measured `scan` and `chain` microseconds. The first play check
put the scan at **138–517 µs on 131–285 live bodies** (against 3.0–5.6 ms for the phase 1 probe), so
the walk is no longer the cost of a shot. Nothing binds a laser to 37 mm / 30 mm in the shipped
config; at ~100 shots/s the scan would still be 1–5 % of the frame.

### Cost, and the stutter (0.4.0)

The third play check reported a **1–2 frame hitch** on the 12 000-unit blunt laser. The order of
magnitude rules out the ray: at 50 fps one frame is 20 ms, and the whole scan is 0.2–0.5 ms. The
per-shot line now separates the pieces so the next log answers it directly:

| field | what it times |
|---|---|
| `in N us` | the ray (`ray::cast`) — the walk, the casts, the sort |
| `chain N us` | the whole chain after the scan, **including** our log lines |
| `of which N us in the game's damage/blast calls` | the `Body_ApplyDamage` and `Body_Explode` calls only |

Where the time really goes, in the order the evidence points:

* **Per-part deaths.** A 12 000-unit beam with `damage 1200` and `max_parts 1200` crosses a whole ship,
  so one trigger pull can take **10–30 parts to 0 hp in the same call**. Every one of those runs the
  game's own `Body_DieExplode` (`0x140023ac0`, a 5 KB routine: UI messages, sound, debris). Vanilla
  hands that cost out one shell at a time; a laser hands it 30 deaths in one frame. This is the prime
  suspect, and `max_parts` is the lever.
* **Log lines.** Each `log::info!` goes through the modloader's logger (file + log window). Twenty to
  thirty lines inside one frame is real time, which is why 0.4.0 stopped printing per-hit lines for a
  shot that damages something.
* **The blast**, now: `Body_Explode` walks the bodies in its radius and applies area damage, so it adds
  one more pass per shot rather than per part.

Not done, and why (the "more efficient method" question):

* **Box2D's own broadphase ray query** (`b2World::RayCast` with a callback object) is the textbook
  answer: it visits only the fixtures near the segment. It needs a C++-ABI callback object with a
  vtable built in Rust, and it would save ~0.2–0.5 ms — invisible next to the death work it cannot
  fix. Not worth the crash surface.
* **A fixture-proxy AABB pre-filter** (`b2FixtureProxy` is 0x20 bytes and starts with a world-space
  `b2AABB`; `fixture+0x28`/`+0x30` are the proxy array and count) would cut the cast cost roughly in
  half, but a body that is not yet in the broadphase has zeroed proxies, so a wrong pre-filter would
  silently lose hits. The scan is not the bottleneck; leave it.

### Ammo binding — the `shells` map is the only test (0.7.0)

The three ids are read at fire time and logged once per ammo: `+0x150` is the projectile **logic**
(`100` a plain shell, `160` the proximity-fuze cannon shell, `130`/`132`/… rockets), `+0x154` is
`ammo-extended`'s **caliber** (`5/10/15/20` for 37/57/100/130 mm), `+0xcc` is its `behavior`
(`20` for the 130 mm family). None of them decides anything any more.

Until 0.7.0 a mapped ammo also had to look like an unmodified cannon shell — `+0x154` in
`{5, 10, 0xf, 0x14, 0x19, 0x1e, 0x28}` and `+0x150` outside
`{1, 3, 4, 130, 132, 135, 137, 140, 150, 400}` — so that a typo could not turn a missile into a
laser. That test was wrong for a modded install: `+0x154` is the caliber, and a custom round may put
any number there (`ITEM_AMMO_THRANGXNON` is a 20 mm "laser" with caliber `95` and was refused). The
`shells` map is already the explicit opt-in — there is no gun key and no `default` type — so an ammo
the config names becomes a laser, whatever its ids say. The ids are still printed once per ammo:

```
hf_hitscan: ammo "ITEM_AMMO_THRANGXNON": +0x154 95 (0x5f), +0xcc 0x14, +0x150 100 -> laser type "laser_blunt"
```

An ammo that is **not** in `shells` says so once and its shots stay vanilla shells.

### What the mod does **not** touch

Loaded rounds (`+0x860`), the cooldown (`+0x86c`), the reload timer (`+0x864`), the AI burst latch
(`+0x870`) and the ammo table position (`+0x850`) are all written by `FireGun` *before* the patched
call, and this mod changes none of them. There is no side test: friendly parts take damage like a
shell would.

### Coexistence hazard (H12)

`hf_gun_mechanics` patches the cooldown store at `0x140032f91` (8 bytes, resuming at `0x140032f99`)
— i.e. it jumps **into** this mod's five bytes. That is fine as long as this patch stays exactly five
bytes and never touches `0x140032f8e..0x140032f98`. Do not grow the site.

### The beam ends on the last body it damages (expected, not a bug)

A piercing type with `max_parts: 0` and `damage_decay_per_unit: 0` has **no stop rule left**, so the
chain simply runs out of bodies: once the ray leaves the ship (or only ever clipped one block of it)
there is nothing more to damage and the shot ends there. That is why a beam that crosses a ship from
one end to the other is drawn through it (17 parts, far side), while a shot that merely clipped an
engine is drawn only to that engine — the line's end is **the last body the chain reached**, not
`range`. When the ray hits nothing at all the line goes to `range`.

This was investigated in the 21:55 / 21:58 logs and confirmed in play on 2026-11-23: not a bug, a
minor inconvenience of a line-of-sight beam. **Do not "fix" it.** The only ways to see a longer line
are a longer `range` (the miss case), `pierce.through_terrain` (so a hill is crossed and the ship
behind it is reached), or spread (phase 5.5) making the ray cross more of a ship.

---

## Phase 1 findings — what phase 2 relies on (live log, 2026-11-23 05:43)

50 shots, two guns, `163 Highfleet`, every shot still vanilla. Log:
`Highfleetcracked_2026-11-23_05-43-54.log`.

**Muzzle — settled.** `o` is `gun+0x128` to the printed precision (`(1242.68, -1085.55)` both),
`|d| = 1.0000 ± 0.00004`, and `muzzle − gun+0x128 = 10·d` exactly (`delta (-9.96, 0.83)`). The
transform is the vtable slot `+0xe8`; the direction is **not** normalised.

**H20 — settled for the vanilla path.** `ITEM_AMMO_130MM` → `+0x154 = 20 (0x14)`, `+0xcc = 0`,
`+0x150 = 100`; `ITEM_AMMO_37MM` → `+0x154 = 5`, `+0xcc = 0`, `+0x150 = 100`. See the dispatch rule
above.

**Terrain — settled.** `lnd_close_01 … lnd_close_06`: class byte **`0x0f`** (same as a ship part, so
the class filter does not separate them), owner **`0x0`**, partIndex `-1`, category `0`, health
`100000`. So **`+0xb8 == 0` is the terrain marker**.

**H14 — measured, and too slow as written.** 136–205 live bodies, 76–111 candidates, 113–162 fixture
casts, **3.0–5.6 ms per probed shot** (of which ~1.2 ms was report overhead). The raycasts were cheap;
the cost was the probe's own per-pointer `VirtualQuery` validation. Phase 2's answer is described
under *Cost* above.

**Two kinds of body the ray hits that are not ship parts.** Both appear in front of the firing gun,
both have partIndex `-1` and no `oid`:

| body | class | health | where | reading |
|---|---|---|---|---|
| `oid ""` | `0x0f` | `1.0` | 40–175 u from the muzzle | a **shell in flight** (`Body_InitializeCannonShell` gives shells health 1.0) |
| `oid ""` | `0x1f` | `100.0` | right behind a hit part | the ship's **Frame** (`0x1f` = Frame class, which passes the `& 0xf == 0xf` filter). Phase 1 read it as an "effect body"; 0.3.2 established that it is the frame that carries every mounted part's fixture |

Phase 2 skipped both (`partIndex < 0`) and let the beam continue through them, while terrain
(`owner == 0`) stopped it. Since 0.3.2 the parts behind the frame's fixtures are damaged
individually, so the frame no longer hides a ship from the ray. Since 0.7.0 the two are separated by
class: `0x1f` is still skipped (a frame), while a `0x0f` body with a live owner is a free object and
**takes damage** — that is what lets a laser kill a missile in flight. A free body spawned by
`CreateShell_` (debris, fracture pieces) has owner `GlobalNode_`, not 0, so it is damageable too.

The class word is `Body+0x28` (a qword): `Frame_Constructor`, `Frame_CopyConstructor` and
`CreateFrameFragments` are the only writers of `0x1f`; every other body carries `0x0f`, which is also
why the aim ray's `& 0xf == 0xf` filter lets frames through.

---

## 5. Building, and the standing process decisions

Build: `cargo test --offline` (**55 tests** in v1.0.0) + `cargo build --release --offline` in this
folder; the DLL is copied to `release\Modloader\mods\` and into the throwaway `163 Highfleet` install.
The folder is a git repo since v1.0.0 (`main`, tag `v1.0.0`); `.gitignore` excludes `target/`, and
`.gitattributes` is `* -text` so the shipped config bytes and the release zip are never EOL-converted
(the config is `include_str!`'d into the DLL, so its bytes are part of the binary).

Historically the shipped config gained `impact` in 0.4.0, `impact.at` in 0.4.2 and
`pierce.through_terrain` in 0.8.0 (sha256 `4E18A1958F391312F7468773E0029813E1FF15D9A189C5C4B58A2F8BC9D9DB61`),
`spread` in 0.9.0 (sha256 `31F3A051CF419762D54E2699777F272BA97B50D7D40FC98F3FFEB535E2A0E0A6`), and was
replaced outright in 1.0.0 by the four-type example template (sha256
`B39992F6A6AD39E94F283DBBEB7C631B890D4D1BB57F02A2BB6342DF53F15CDA`, `verbose_logging: false`, bound to
`ITEM_AMMO_EXAMPLE1..4`); 0.5.0, 0.6.0 and 0.7.x change no config at all.

Standing decision (2026-11-23): **the user's live `hitscan.json` is not requested for every build and
is not rewritten**; a new key would be *added* to it, never substituted. On 2026-11-24 his working set
was renamed to `163 Highfleet\Modloader\config\hitscan_custom.json` (5 types / 6 shells: `laser_blunt`,
`laser_incen`, `laser_pierce`, `laser_pierce_2`, `laser_small` with `spread 0.5`, over
`ITEM_AMMO_130MM`, `_AP`, `_PROXY`, `_INC`, `ITEM_AMMO_37MM_INC` and `ITEM_AMMO_THRANGXNON`) and the
live `hitscan.json` was replaced by him with a commented example file that this loader **rejects**
(`comment` keys are unknown fields, and `stop_at: ["MDL_COMBRIDGE_01"]` is not an `m_category`), so a
session started from that file installs nothing. It is his file and is deliberately left alone.

`ammo-extended` **is** installed in the test install (`ammo_extended.dll` + its 60-row
`ammo_extended.json`, which is where `ITEM_AMMO_THRANGXNON`, caliber `95`, comes from). The mod never
writes ammo data, so the two coexist; its `behavior` id is what `+0xcc` reads, and `+0x154` is its
`caliber`.
