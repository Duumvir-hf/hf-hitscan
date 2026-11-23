# hf_hitscan

**Hitscan lasers for HighFleet — Steam 1.163.**

Cannon ammo you bind in the config stops spawning a shell. Instead the shot casts a ray from the
muzzle, damages every part it crosses, draws a short-lived beam and plays the ammo's own launch sound.
No shell, no tracer, no travel time — the damage is where you point, on the frame you fire.

Version **1.0.0** · MIT · [changelog](CHANGELOG.md) · [design notes and engine reference](docs/DESIGN.md)

---

## What it does

* **A ray instead of a shell.** The muzzle is rebuilt exactly the way vanilla's `FireShell` builds it,
  then the game's own HUD aim ray (`Hud_DrawAimRay`) is cast from it. Every body the ray crosses is
  walked nearest-first and takes `Body_ApplyDamage` — armour, hull, blocks, guns, engines, radar,
  ammo, bridge, and free bodies too (shells and missiles in flight can be shot down).
* **Per-type damage model.** `range` (hard cutoff), `damage` at the muzzle and
  `damage_decay_per_unit` along the way; `pierce` with `stop_at` categories and a `max_parts` limit;
  `ignore_armor_threshold` for blast-style hits that bypass the armour floor; `incendiary` heat per
  damaged part; and an optional impact blast that is the game's own explosion.
* **A beam.** A line from the muzzle to the point where the chain stopped, fading over
  `visual.seconds`, on the game's application clock (pausing freezes it). Not dimmed by the battle
  HUD's darkness overlay.
* **The right sound.** The ammo's own `shell_out` / `shell_enemy` / `shell_far` set, with vanilla's
  random variant and volume — a laser sounds like the shell it replaced, at any zoom, for either side.
* **No options on the gun.** Binding is by ammo only: there is no gun key, no default type, and no
  side filter. Unlisted ammo behaves exactly like vanilla, forever.

## Requirements

* **HighFleet, Steam 1.163.** The mod checks the version string and refuses to start on anything else.
* **[logdot's Modloader](https://github.com/logdot/Highfleet-Modloader)** v4.0.0 (the mod is built
  against `highfleet-mod-api` v4.0.0 and `patchy-rs` v0.1.0).
* Optional: **`ammo-extended`** — fully compatible (see *Known limits* for the AI-aim detail).

## Install

1. Copy `Modloader\` over your game folder's `Modloader\`:

   ```
   Modloader\mods\hf_hitscan.dll
   Modloader\config\hitscan.json
   ```

2. Start the game. The Modloader log must show:

   ```
   hf_hitscan v1.0.0: image base 0x140000000, version Steam 1.163
   hf_hitscan: 4 type(s), 4 shell binding(s)
   hf_hitscan: FireGun call site verified at 0x140032f99 (CALL FireShell 0x1400300d0)
   hf_hitscan: draw tick verified at 0x14029c5c0 (BattleHud_DrawCombatMarkers entry)
   Installed 2 permanent patch(es) using 1 shared trampoline page(s)
   hf_hitscan v1.0.0: 2 hook(s) installed; config 4 type(s) / 4 shell binding(s); …
   ```

   If the fire hook is refused, nothing is installed at all. If only the draw tick is refused
   (`draw tick refused: …`), damage still works and only the beam is missing.

## Configure it

> **The shipped `hitscan.json` is a template.** Its four types are bound to the placeholder ammo
> names `ITEM_AMMO_EXAMPLE1` … `ITEM_AMMO_EXAMPLE4`, which do not exist in vanilla — so **nothing
> changes in game until you map real ammo** in `shells`. Pick the type you want and put a real
> `item_name` next to it.

The file is read **once**, at startup: edit it, restart the game. If the file is missing, the shipped
copy is written out of the DLL and used.

```jsonc
{
  "enabled": true,          // false = no hook at all
  "verbose_logging": false, // true = per-shot chain and scan-cost lines in the Modloader log

  "types": {
    "my_laser": {
      "range": 3000.0,                  // world units, hard cutoff
      "damage": 8.0,                    // at the muzzle
      "damage_decay_per_unit": 0.0033,  // damage lost per world unit travelled
      "spread": 0.0,                    // uniform ± scatter of the aim, RADIANS (vanilla 0.05 / 0.02); 0 = pinpoint
      "pierce": {
        "enabled": true,                // false = the first part stopped takes the damage and ends the beam
        "stop_at": ["block"],           // categories that take damage and then stop the beam
        "max_parts": 12,                // 0 = unlimited; counted only while piercing
        "through_terrain": false        // true = the ground no longer stops the beam
      },
      "ignore_armor_threshold": true,   // true = blast-style hit, the part's armour floor is bypassed
      "incendiary": 0.0,                // heat added to every part the beam damages
      "impact": {
        "explosion": true,              // spawn the game's own explosion where the beam stops
        "at": "first",                  // "first" = entry point (default), "last" = far end
        "explosive_power": 20.0         // blast power; radius = 2·√power game units (a 130 mm HE shell is 20)
      },
      "visual": {
        "color": "#FF3322",             // beam colour: "#RRGGBB", "#AARRGGBB" or an ARGB integer
        "width": 1.5,                   // the game's own aim line is 1.0
        "glow": true,                   // a second, wider, dimmer line under the core
        "glow_color": "#33FF3322",      // optional; default = color at 20 % alpha
        "seconds": 0.08,                // lifetime on the game clock; 0 draws nothing
        "fade": "linear"                // "linear" = alpha × (1 − age/seconds), "none" = constant
      }
    }
  },

  "shells": {
    // ammo item_name (case-insensitive) -> a key of `types`. Ammo-only binding:
    // there is no gun key and no default type. Unlisted ammo stays a normal shell.
    "ITEM_AMMO_130MM": "my_laser"
  }
}
```

The four shipped example types, and what their numbers do:

| type | what it demonstrates |
|---|---|
| `hitscan_impact` | pinpoint white 5 000 u line, 20 damage decaying to 0 at range, no pierce, no blast |
| `hitscan_explosive` | short orange 2 000 u line with a little spread, 3 damage, armour respected, 20-power blast at the entry point |
| `hitscan_incendiary` | red 5 000 u line with wide spread, 10 damage, pierces up to 10 parts, ignores the armour floor, **100 heat per damaged part**, blast at the entry point |
| `hitscan_pierce` | blue 2 000 u line, 27.5 damage decaying 0.0125/u, unlimited pierce that stops at nothing, 2-power blast at the **exit** point |

`pierce` also accepts the shorthand `true` / `false` (expands to `{"enabled": …}` with the defaults
`stop_at: ["block"]`, `max_parts: 12`). `impact` is optional: without it there is no blast, `at` is
`"first"` and the power is `20`.

**`stop_at` takes `m_category` categories, not part names.** A part name such as `MDL_COMBRIDGE_01`
is rejected; every other entry must be a raw bit number:

| bit 1 `fuel` | bit 32 `gun` | bit 1024 `aircraft` | bit 16384 `missile` |
|---|---|---|---|
| bit 2 `armor` | bit 64 `block` / `bridge` | bit 4096 `cargo` | bit 65536 `radar` / `sensor` |
| bit 4 `fss` | bit 128 `hull` / `truss` / `hardpoint` | bit 8192 `leg` | |
| bit 8 `evac` | bit 256 `engine` | | |
| bit 16 `ammo` | | | |

A combridge and a block share bit 64, so "stop at the combridge but not at blocks" is not expressible
today. The beam stops **after** the damage is applied to the part that matched.

**`spread` is in radians, exactly like vanilla** — the number is what the game's own
`HGE.Random_Float` receives. One uniform roll per shot in `[-spread, +spread]` rotates the firing
direction after the muzzle; the muzzle does not move and the rotated direction is what the damage,
the decay and the beam use. Vanilla's own values: `0.05` rad (2.8648°) for 37/57/100 mm guns, `0.02`
rad (1.1459°) for 130 mm and up. `0` is pinpoint and draws no random number. The AI does not
compensate for spread (vanilla's does not either).

### What the loader rejects (and why there are no clamps)

Every number is taken as written, however extreme; a finite value is never rewritten. Values that are
*meaningless* rather than extreme reject the whole file, with a log line naming the key path, and
nothing is installed:

* `range <= 0`, `damage < 0`, `damage_decay_per_unit < 0`, `spread < 0`, `pierce.max_parts < 0`,
  `visual.width < 0`, `visual.seconds < 0`, `incendiary < 0`, `impact.explosive_power < 0`;
* any non-finite number (`1e39` reads as `inf`);
* an unknown key anywhere (a typo must not silently do nothing);
* a `shells` value that is not a key of `types`;
* two keys that differ only by case (`types` and `shells` alike);
* a `stop_at` name that is not in the table above and is not a number;
* a colour that is not `#RRGGBB`, `#AARRGGBB` or an ARGB integer;
* a `fade` value other than `linear` / `none`.

`damage: 0` is legal (a harmless marker laser).

### Why a shot can do nothing — check the type first

Most "the laser does not damage / pierce / explode" reports are one of these, and the per-shot log
(with `verbose_logging: true`) names the cause in its summary line:

* **`pierce.enabled: false`** → `stopped: pierce off at N u` on every shot. The type is configured
  not to pierce.
* **`damage_decay_per_unit`** → the beam is spent where `damage − decay·distance` reaches zero. A type
  with `damage 0.3` and `decay 0.001/u` is dead beyond 300 u, **inside** its own `range`.
* **`pierce.max_parts`** → `stopped: max_parts at N u` (10 is easy to hit on a capital ship).
* **`damage` below the part's armour floor** with `ignore_armor_threshold: false` → every hit is
  `blocked`. The floor is `m_mdl_armor` from `OL.seria`, and the whole game uses only **1**
  (fuel/ammo racks, missiles, aircraft, jammers), **3** (guns, FSS, protector, evac, RSZO), **5**
  (armour, bridge, hull, blocks, engines, legs, radar, generator, quarters, cargo, deck) and **10**
  (bombs). An incendiary type whose `damage` is below the floor never heats anything either, because
  the dose is only written for parts the game actually accepted.
* **`impact.explosion: false`** → `blast no` by design.
* **`spread`** → a large value misses by geometry: the log prints the rolled angle and the beam
  follows it.
* **`range`** shorter than the target → `stopped: range`.
* **A piercing type with no stop rule ends on the last body it damages** — that is expected, not a
  bug (see *Known limits*).

## Known limits

* **No muzzle flash, no impact particles, no recoil.** The impact *blast* is the game's own explosion;
  the beam is a line, not a tracer.
* **The launch sound is the ammo's own vanilla sound**, not a per-type one (there is no sound key).
  Some ammo rows point at a small-calibre set — e.g. `ITEM_AMMO_130MM_INC` names
  `shell_out_small_far` (and `SHELL_57mm_INC` as its magazine sprite) because that is what its row in
  the ammo data says. If the chosen set has fewer clips than the branch asks for, the mod retries the
  other variants, so the shot is audible.
* **A mapped ammo is a laser whatever it is.** The `shells` map is the only test, so mapping a rocket
  or a bomb ammo really does turn it into a hitscan weapon.
* **Free bodies take damage**, so a laser can destroy shells and missiles in flight — including the
  enemy's. Debris is damageable too.
* **Aircraft guns are not intercepted** (they call `FireShell` directly), so they neither damage along
  a ray nor draw a beam.
* **`spread` rotates the ray, not the muzzle**, and nothing re-aims afterwards: at `0.05` rad a shot
  at 1 000 u lands ±50 u off the aim point and can miss a small ship entirely.
* **AI aim needs the ammo's `speed` raised.** The AI solves a lead for a travelling projectile; with
  `ammo-extended`, setting `speed` to `1000000.0` on the round the AI loads makes its guns aim
  straight at you (verified in play). **Rule:** every ammo whose speed you raise must stay mapped in
  `shells`, because the AI rewrites the loaded ammo every tick — an unmapped 1e6 round is a
  teleporting shell.
* **A beam that crosses a whole ship kills every part on the line in one frame.** That is the config's
  `damage` / `max_parts`, and it is the one thing worth watching for a frame hitch on very large
  volleys.
* **A piercing type with no stop rule (`max_parts: 0`, `damage_decay_per_unit: 0`, `stop_at: []`)** has
  nothing left to stop it, so the chain simply runs out of bodies: a shot that crosses a ship is drawn
  through it, a shot that clipped one block is drawn to that block. **That is expected — do not
  report it as "the laser does not pierce".** To make the line longer, raise `range` (miss case), set
  `pierce.through_terrain: true` (cross a hill), or use spread.
* **`incendiary` ignition is the game's own fire model**: fuel and engines burn and feed themselves,
  trusses/blocks/bridge/guns/ammo catch fire but cannot feed it, and armour never burns (it has no
  `m_burn_hp`). The dose is applied only to parts the damage call accepted.
* **Coexistence:** this mod patches the five bytes of `CALL FireShell` at `0x140032f99` and resumes
  with an explicit jump to `0x140032f9e`. `hf_gun_mechanics` patches the cooldown store at
  `0x140032f91` and resumes *into* those bytes, so both can be installed together as long as this
  patch stays exactly five bytes.

## Verification status

Every feature below was seen in play by the author on a Steam 1.163 install, not just unit-tested.
The per-shot log is the evidence; the sessions are listed in the changelog.

| feature | since | status |
|---|---|---|
| version gate (`Steam 1.163` only) and vanilla pass-through | 0.1.0 | **verified in play** 2026-11-23 |
| config load, validation and reject list | 0.1.0 | unit-tested; the load log is verified in play |
| a fixture hit damages its **part**, not the ship frame | 0.3.2 | **verified in play** (armour, hull, radar, missiles…) |
| damage chain: range, decay, pierce, `stop_at`, `max_parts`, armour floor | 0.3.x | **verified in play** 2026-11-23 |
| incendiary dose → fire | 0.3.0 | **seen in play**: fuel and engines ignite, armour never burns |
| impact blast (`impact.explosion`, `Body_Explode`) | 0.4.x | **seen in play** 2026-11-23 |
| per-shot cost: scan 0.14–0.70 ms, ~46 µs per damaged part | 0.3.0 / 0.4.0 | **measured in play** |
| beam (`Gfx_DrawWorldLine` + draw tick), not dimmed by the HUD overlay | 0.5.0 | **seen in play** 2026-11-23 |
| launch sound: player close, enemy close, far fallback | 0.6.0 | **verified in play** 2026-11-23 |
| ammo binding without a "plain cannon shell" test (custom rounds work) | 0.7.0 | **verified in play** (`ITEM_AMMO_THRANGXNON`, caliber 95) |
| free bodies (missiles, shells, debris) take laser damage | 0.7.0 | **verified in play** ("missile interception works wonderfully") |
| land recognised by its 100 000 hp, beams pass wrecks | 0.7.1 | **verified in play** 2026-11-23 |
| `pierce.through_terrain` | 0.8.0 | **verified in play** (beam carries on behind a hill) |
| blast spawned once at `explosive_power` | 0.8.2 | exercised in the **verified** 0.9.0 session (12 `blast yes`, 0 `FIZZLED`); `explosive_power` is the only lever |
| per-type `spread` (radians) | 0.9.0 | **verified in play** — "works perfectly fine. beautiful." |
| AI lead via `speed = 1e6` (config only, no DLL change) | 0.9.0 | **seen in play** — "AI aims straight at me, very well" |
| **overall** | 1.0.0 | **feature-complete**: phases 0–5.5 verified, 1.0.0 changes no gameplay |

## Layout, credits, licence

```
hf_hitscan\
  Cargo.toml, Cargo.lock, src\        the crate (9 modules, 55 unit tests)
  README.md                           this page
  CHANGELOG.md                        one entry per build, with every DLL sha256
  docs\DESIGN.md                      engine reference: fixtures, patch sites, blast, beam, sound, cost
  release\Modloader\mods\hf_hitscan.dll
  release\Modloader\config\hitscan.json
  hf_hitscan-v1.0.0.zip               the two files above, ready to drop in
```

Built on **logdot's** [Highfleet-Modloader](https://github.com/logdot/Highfleet-Modloader) and
`patchy-rs`, with the mod API and the loader's patch discipline from `hf_gun_mechanics`. The engine
facts come from the author's own Ghidra work on Steam 1.163 and are recorded in `docs/DESIGN.md`.

**MIT licence**, © 2026 Duumvir-hf. See [LICENSE](LICENSE).

Not affiliated with Konstantin Koshutin or MicroProse. HighFleet and its assets belong to their
owners; this mod ships no game data.
