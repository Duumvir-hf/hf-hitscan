//! `Modloader/config/hitscan.json` — the whole configuration of `hf_hitscan`.
//!
//! The file has exactly two tables:
//! * `types` — the laser damage types (range, damage, decay, piercing, visuals);
//! * `shells` — ammo `item_name` → type name. **This is the only binding** (user decision D2):
//!   no gun key, no `default` type. An ammo that is not listed is not a laser.
//!
//! Rules that shape this module (roadmap §4, §6.1, §6.2, §6.4):
//! * **Unknown keys are a load error**, so a typo cannot silently do nothing.
//! * **No clamps.** A finite value is taken as written, however extreme. Values that are
//!   *meaningless* rather than extreme (`range <= 0`, a negative damage, …) reject the file with a
//!   key path, they are never rewritten.
//! * Every rejection is collected, not just the first, so one bad reload shows the whole list.
//! * The file is read **once**, in `init`, into a [`Snapshot`]; the fire path only does lookups.
//!
//! The shipped `release/Modloader/config/hitscan.json` is part of the crate
//! (`include_str!`) and is written out when the file is missing, so there is one source of truth
//! for the defaults.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where the modloader keeps this mod's settings (relative to the game folder).
pub(crate) const PATH: &str = "Modloader/config/hitscan.json";

/// The build's own copy of the config, used when the game folder has none.
pub(crate) const SHIPPED: &str = include_str!("../release/Modloader/config/hitscan.json");

/// The raw file. Every struct below rejects unknown fields on purpose.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    /// Master switch. `false` installs no hook at all.
    pub(crate) enabled: bool,
    /// Log the first shots and the resolved ammo bindings.
    pub(crate) verbose_logging: bool,
    /// Free-form type name → laser definition.
    pub(crate) types: BTreeMap<String, LaserType>,
    /// Ammo `item_name` (case-insensitive) → a key of `types`.
    pub(crate) shells: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            verbose_logging: true,
            types: BTreeMap::new(),
            shells: BTreeMap::new(),
        }
    }
}

/// One configured laser damage type (`types.<name>`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LaserType {
    /// World units; the beam is hard-clipped here (D4: always a number, never `null`).
    pub(crate) range: f32,
    /// Damage at the muzzle.
    pub(crate) damage: f32,
    /// Damage lost per world unit travelled (D1).
    pub(crate) damage_decay_per_unit: f32,
    /// Uniform ± scatter of the firing direction, in **radians** — vanilla's own unit (phase 5.5,
    /// §5.13). `0.0` (the default) is pinpoint and draws no random number at all.
    pub(crate) spread: f32,
    /// `false` / `true` shorthand is accepted for `{"enabled": ...}`.
    pub(crate) pierce: PierceSetting,
    /// `directHit = 0` in `Body_ApplyDamage` (blast-style, armour threshold bypassed).
    pub(crate) ignore_armor_threshold: bool,
    /// Dose written to `Body +0x2f0` for every damaged part.
    pub(crate) incendiary: f32,
    /// What happens where the beam stops (phase 2: only the blast).
    pub(crate) impact: Impact,
    pub(crate) visual: Visual,
}

impl Default for LaserType {
    fn default() -> Self {
        Self {
            range: 3000.0,
            damage: 8.0,
            damage_decay_per_unit: 0.0033,
            spread: 0.0,
            pierce: PierceSetting::Detailed(Pierce::default()),
            ignore_armor_threshold: true,
            incendiary: 0.0,
            impact: Impact::default(),
            visual: Visual::default(),
        }
    }
}

/// `types.<name>.impact` — the effect where the beam ends.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Impact {
    /// Spawn the game's own blast (`Body_Explode`) at the part in [`Impact::at`].
    pub(crate) explosion: bool,
    /// Which part of the beam carries the blast.
    pub(crate) at: BlastAt,
    /// The part's `explosivePower (+0x2dc)` while the blast runs; the vanilla 130 mm HE shell is
    /// `20`. Restored immediately, so the part does not become explosive afterwards.
    pub(crate) explosive_power: f32,
}

impl Default for Impact {
    fn default() -> Self {
        Self {
            explosion: false,
            at: BlastAt::default(),
            explosive_power: 20.0,
        }
    }
}

/// Where along the beam the impact blast is spawned.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BlastAt {
    /// The first part the beam reached — the surface it entered.
    #[default]
    First,
    /// The last part the beam damaged — for a piercing type, the far side (or wherever it stopped).
    Last,
}

/// `pierce` in the file: either a bare `true` / `false` or the expanded object.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub(crate) enum PierceSetting {
    Flag(bool),
    Detailed(Pierce),
}

impl PierceSetting {
    /// The expanded form; the shorthand only changes `enabled`.
    fn expanded(&self) -> Pierce {
        match self {
            Self::Flag(enabled) => Pierce {
                enabled: *enabled,
                ..Pierce::default()
            },
            Self::Detailed(pierce) => pierce.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Pierce {
    /// `false` = the first damaged part stops the beam.
    pub(crate) enabled: bool,
    /// Categories that take damage and then terminate the beam.
    pub(crate) stop_at: Vec<StopEntry>,
    /// `0` = unlimited; counted only when piercing is enabled.
    pub(crate) max_parts: i32,
    /// `true` = the level's ground takes no damage and **does not stop the beam** (the beam and its
    /// damage carry on behind a hill). Off by default: a hill blocks a shot, as it does in vanilla.
    pub(crate) through_terrain: bool,
}

impl Default for Pierce {
    fn default() -> Self {
        Self {
            enabled: true,
            stop_at: vec![StopEntry::Name("block".to_owned())],
            max_parts: 12,
            through_terrain: false,
        }
    }
}

/// A `stop_at` entry: a category name, or a raw `m_category` bit value.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub(crate) enum StopEntry {
    Name(String),
    Bits(u32),
}

/// `visual` of one type.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Visual {
    /// `"#RRGGBB"`, `"#AARRGGBB"` or an ARGB integer.
    pub(crate) color: Colour,
    /// World units, the bright line.
    pub(crate) width: f32,
    /// A second, wider, dimmer line underneath.
    pub(crate) glow: bool,
    /// Omitted = `color` with 20 % alpha.
    pub(crate) glow_color: Option<Colour>,
    /// How long the beam is drawn, in game-clock seconds.
    pub(crate) seconds: f32,
    pub(crate) fade: Fade,
}

impl Default for Visual {
    fn default() -> Self {
        Self {
            color: Colour::Text("#FF3322".to_owned()),
            width: 1.5,
            glow: true,
            glow_color: None,
            seconds: 0.08,
            fade: Fade::Linear,
        }
    }
}

/// A colour as written in the file: a string or an ARGB integer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub(crate) enum Colour {
    Text(String),
    Argb(u32),
}

/// How the beam's alpha fades over `visual.seconds`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Fade {
    #[default]
    Linear,
    None,
}

/// A validated type, ready for the fire and draw paths. All lookups are by index.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedType {
    /// The key as written in the file (for logs only).
    pub(crate) name: String,
    pub(crate) range: f32,
    pub(crate) damage: f32,
    pub(crate) damage_decay_per_unit: f32,
    /// `types.<name>.spread`: uniform ± scatter of the firing direction in radians (`0` = pinpoint).
    pub(crate) spread: f32,
    /// `pierce.enabled`.
    pub(crate) pierce: bool,
    /// The `stop_at` list folded into one `m_category` mask.
    pub(crate) stop_mask: u32,
    pub(crate) max_parts: i32,
    /// `pierce.through_terrain`: the ground takes no damage and does not stop the beam.
    pub(crate) through_terrain: bool,
    pub(crate) ignore_armor_threshold: bool,
    pub(crate) incendiary: f32,
    /// `impact.explosion`.
    pub(crate) explosion: bool,
    /// `impact.at`.
    pub(crate) blast_at: BlastAt,
    /// `impact.explosive_power`.
    pub(crate) explosive_power: f32,
    pub(crate) color: u32,
    pub(crate) width: f32,
    pub(crate) glow: bool,
    pub(crate) glow_color: u32,
    pub(crate) seconds: f32,
    pub(crate) fade: Fade,
}

/// The validated, immutable view installed in `init`.
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub(crate) enabled: bool,
    pub(crate) verbose_logging: bool,
    pub(crate) types: Vec<ResolvedType>,
    /// Lower-cased ammo `item_name` → index into `types`.
    pub(crate) shells: BTreeMap<String, usize>,
}

impl Snapshot {
    /// The laser a loaded ammo would use, or `None` when this ammo is not a laser.
    ///
    /// Phase 1 uses it for the probe's ray length; phase 2 uses it for the shot itself.
    pub(crate) fn laser_for_item(&self, item_name: &str) -> Option<&ResolvedType> {
        let key = item_name.trim().to_ascii_lowercase();
        self.shells
            .get(&key)
            .and_then(|index| self.types.get(*index))
    }

    pub(crate) fn type_count(&self) -> usize {
        self.types.len()
    }

    pub(crate) fn shell_count(&self) -> usize {
        self.shells.len()
    }
}

/// Read and validate the config. `Err` carries one message per problem; the caller refuses to
/// install anything in that case (roadmap §6.4).
pub(crate) fn load() -> Result<Snapshot, Vec<String>> {
    let text = match std::fs::read_to_string(PATH) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = std::path::Path::new(PATH).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(PATH, SHIPPED) {
                Ok(()) => log::info!("hf_hitscan: no {PATH}; wrote the shipped config"),
                Err(error) => log::warn!("hf_hitscan: no {PATH} and cannot write it: {error}"),
            }
            SHIPPED.to_owned()
        }
        Err(error) => return Err(vec![format!("{PATH}: cannot be read: {error}")]),
    };
    // A UTF-8 BOM would otherwise be a parse error and silently disable the mod.
    let text = text.trim_start_matches('\u{feff}');
    let config: Config = serde_json::from_str(text)
        .map_err(|error| vec![format!("{PATH}: cannot be parsed: {error}")])?;
    config.validate()
}

impl Config {
    /// Collect every rejection (roadmap §6.4) and otherwise build the [`Snapshot`].
    pub(crate) fn validate(&self) -> Result<Snapshot, Vec<String>> {
        let mut errors = Vec::new();
        let mut types: Vec<ResolvedType> = Vec::new();
        // Lower-cased type name -> index, and -> the spelling seen first (for the message).
        let mut index_of: BTreeMap<String, usize> = BTreeMap::new();
        let mut spelling: BTreeMap<String, String> = BTreeMap::new();

        for (name, laser) in &self.types {
            let key = name.trim().to_ascii_lowercase();
            if key.is_empty() {
                errors.push("types: a type name is empty".to_owned());
                continue;
            }
            if let Some(first) = spelling.get(&key) {
                errors.push(format!(
                    "types: {name:?} and {first:?} differ only by case; type names are case-insensitive"
                ));
                continue;
            }
            spelling.insert(key.clone(), name.clone());
            match resolve_type(name, laser) {
                Ok(resolved) => {
                    index_of.insert(key, types.len());
                    types.push(resolved);
                }
                Err(mut problems) => errors.append(&mut problems),
            }
        }

        let mut shells: BTreeMap<String, usize> = BTreeMap::new();
        let mut shell_spelling: BTreeMap<String, String> = BTreeMap::new();
        for (item_name, type_name) in &self.shells {
            let key = item_name.trim().to_ascii_lowercase();
            if key.is_empty() {
                errors.push("shells: an ammo item_name is empty".to_owned());
                continue;
            }
            if let Some(first) = shell_spelling.get(&key) {
                errors.push(format!(
                    "shells: {item_name:?} and {first:?} differ only by case; ammo names are case-insensitive"
                ));
                continue;
            }
            shell_spelling.insert(key.clone(), item_name.clone());
            match index_of.get(&type_name.trim().to_ascii_lowercase()) {
                Some(index) => {
                    shells.insert(key, *index);
                }
                None => errors.push(format!(
                    "shells.{item_name}: type {type_name:?} does not exist in `types`"
                )),
            }
        }

        if errors.is_empty() {
            Ok(Snapshot {
                enabled: self.enabled,
                verbose_logging: self.verbose_logging,
                types,
                shells,
            })
        } else {
            Err(errors)
        }
    }
}

/// Validate one `types.<name>` entry.
fn resolve_type(name: &str, laser: &LaserType) -> Result<ResolvedType, Vec<String>> {
    let path = format!("types.{name}");
    let mut errors = Vec::new();

    finite(&mut errors, &path, "range", laser.range);
    if laser.range.is_finite() && !(laser.range > 0.0) {
        errors.push(format!("{path}.range: {} is not greater than 0", laser.range));
    }
    finite(&mut errors, &path, "damage", laser.damage);
    if laser.damage.is_finite() && laser.damage < 0.0 {
        errors.push(format!("{path}.damage: {} is negative", laser.damage));
    }
    finite(
        &mut errors,
        &path,
        "damage_decay_per_unit",
        laser.damage_decay_per_unit,
    );
    if laser.damage_decay_per_unit.is_finite() && laser.damage_decay_per_unit < 0.0 {
        errors.push(format!(
            "{path}.damage_decay_per_unit: {} is negative",
            laser.damage_decay_per_unit
        ));
    }
    finite(&mut errors, &path, "incendiary", laser.incendiary);
    if laser.incendiary.is_finite() && laser.incendiary < 0.0 {
        errors.push(format!(
            "{path}.incendiary: {} is negative",
            laser.incendiary
        ));
    }
    finite(&mut errors, &path, "spread", laser.spread);
    if laser.spread.is_finite() && laser.spread < 0.0 {
        errors.push(format!("{path}.spread: {} is negative", laser.spread));
    }
    finite(
        &mut errors,
        &path,
        "impact.explosive_power",
        laser.impact.explosive_power,
    );
    if laser.impact.explosive_power.is_finite() && laser.impact.explosive_power < 0.0 {
        errors.push(format!(
            "{path}.impact.explosive_power: {} is negative",
            laser.impact.explosive_power
        ));
    }
    finite(&mut errors, &path, "visual.width", laser.visual.width);
    if laser.visual.width.is_finite() && laser.visual.width < 0.0 {
        errors.push(format!(
            "{path}.visual.width: {} is negative",
            laser.visual.width
        ));
    }
    finite(&mut errors, &path, "visual.seconds", laser.visual.seconds);
    if laser.visual.seconds.is_finite() && laser.visual.seconds < 0.0 {
        errors.push(format!(
            "{path}.visual.seconds: {} is negative",
            laser.visual.seconds
        ));
    }

    let pierce = laser.pierce.expanded();
    if pierce.max_parts < 0 {
        errors.push(format!(
            "{path}.pierce.max_parts: {} is negative",
            pierce.max_parts
        ));
    }
    let mut stop_mask = 0u32;
    for entry in &pierce.stop_at {
        match entry {
            StopEntry::Name(stop) => match category_bit(stop) {
                Some(bit) => stop_mask |= bit,
                None => errors.push(format!(
                    "{path}.pierce.stop_at: unknown category {stop:?} (use a name from the README or a raw bit value)"
                )),
            },
            StopEntry::Bits(bits) => stop_mask |= bits,
        }
    }

    let color = match colour_of(&laser.visual.color) {
        Some(color) => color,
        None => {
            errors.push(format!(
                "{path}.visual.color: {:?} is not \"#RRGGBB\", \"#AARRGGBB\" or an ARGB integer",
                laser.visual.color
            ));
            0
        }
    };
    let glow_color = match &laser.visual.glow_color {
        None => (color & 0x00FF_FFFF) | 0x3300_0000,
        Some(glow) => match colour_of(glow) {
            Some(value) => value,
            None => {
                errors.push(format!(
                    "{path}.visual.glow_color: {:?} is not \"#RRGGBB\", \"#AARRGGBB\" or an ARGB integer",
                    laser.visual.glow_color
                ));
                0
            }
        },
    };

    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(ResolvedType {
        name: name.to_owned(),
        range: laser.range,
        damage: laser.damage,
        damage_decay_per_unit: laser.damage_decay_per_unit,
        pierce: pierce.enabled,
        stop_mask,
        max_parts: pierce.max_parts,
        through_terrain: pierce.through_terrain,
        ignore_armor_threshold: laser.ignore_armor_threshold,
        incendiary: laser.incendiary,
        spread: laser.spread,
        explosion: laser.impact.explosion,
        blast_at: laser.impact.at,
        explosive_power: laser.impact.explosive_power,
        color,
        width: laser.visual.width,
        glow: laser.visual.glow,
        glow_color,
        seconds: laser.visual.seconds,
        fade: laser.visual.fade,
    })
}

/// `types.<name>.<field>` must be a finite number (roadmap §6.4: "any non-finite number").
fn finite(errors: &mut Vec<String>, path: &str, field: &str, value: f32) {
    if !value.is_finite() {
        errors.push(format!("{path}.{field}: {value} is not a finite number"));
    }
}

fn colour_of(colour: &Colour) -> Option<u32> {
    match colour {
        Colour::Argb(value) => Some(*value),
        Colour::Text(text) => parse_colour(text),
    }
}

/// Parse `"#RRGGBB"` (alpha `FF`) or `"#AARRGGBB"`. The `#` is optional.
pub(crate) fn parse_colour(text: &str) -> Option<u32> {
    let digits = text.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match digits.len() {
        6 => u32::from_str_radix(digits, 16)
            .ok()
            .map(|value| 0xFF00_0000 | value),
        8 => u32::from_str_radix(digits, 16).ok(),
        _ => None,
    }
}

/// The `m_category` bit a `stop_at` name refers to (roadmap §6.1).
fn category_bit(name: &str) -> Option<u32> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "fuel" => 1,
        "armor" => 2,
        "fss" => 4,
        "evac" => 8,
        "ammo" => 16,
        "gun" => 32,
        "block" | "bridge" => 64,
        "hull" | "truss" | "hardpoint" => 128,
        "engine" => 256,
        "aircraft" => 1024,
        "cargo" => 4096,
        "leg" => 8192,
        "missile" => 16384,
        "radar" | "sensor" => 65536,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Snapshot, Vec<String>> {
        let config: Config = serde_json::from_str(text).map_err(|error| vec![error.to_string()])?;
        config.validate()
    }

    /// The file that ships in `release/` is the build's only default; it must always be valid.
    ///
    /// 1.0.0 ships the four **example** types as a template (bound to the placeholder ammo names
    /// `ITEM_AMMO_EXAMPLE1..4`, so nothing changes in game until a player maps real ammo) with
    /// `verbose_logging` off.
    #[test]
    fn shipped_config_is_valid_and_test_ready() {
        let snapshot = parse(SHIPPED).expect("the shipped config validates");
        assert!(snapshot.enabled);
        assert!(!snapshot.verbose_logging, "the release config ships quiet");
        assert_eq!(snapshot.type_count(), 4);
        assert_eq!(snapshot.shell_count(), 4);

        let impact = snapshot
            .laser_for_item("ITEM_AMMO_EXAMPLE1")
            .expect("example 1 is a laser type");
        assert_eq!(impact.name, "hitscan_impact");
        assert!(!impact.pierce);
        assert!(!impact.explosion, "the impact example does not explode");
        assert!((impact.range - 5000.0).abs() < f32::EPSILON);
        assert_eq!(impact.spread, 0.0, "the impact example is pinpoint");

        let explosive = snapshot
            .laser_for_item("ITEM_AMMO_EXAMPLE2")
            .expect("example 2 is a laser type");
        assert_eq!(explosive.name, "hitscan_explosive");
        assert!(explosive.explosion);
        assert_eq!(explosive.explosive_power, 20.0);
        assert_eq!(explosive.blast_at, BlastAt::First);
        assert!(explosive.spread > 0.0, "the explosive example scatters a little");

        let incendiary = snapshot
            .laser_for_item("ITEM_AMMO_EXAMPLE3")
            .expect("example 3 is a laser type");
        assert_eq!(incendiary.name, "hitscan_incendiary");
        assert!(incendiary.pierce, "the incendiary example pierces up to 10 parts");
        assert_eq!(incendiary.max_parts, 10);
        assert!(incendiary.ignore_armor_threshold);
        assert!(incendiary.incendiary > 0.0);

        let pierce = snapshot
            .laser_for_item("ITEM_AMMO_EXAMPLE4")
            .expect("example 4 is a laser type");
        assert_eq!(pierce.name, "hitscan_pierce");
        assert!(pierce.pierce);
        assert_eq!(pierce.stop_mask, 0, "the shipped pierce example stops at nothing");
        assert_eq!(
            pierce.blast_at,
            BlastAt::Last,
            "the pierce example explodes at the exit point"
        );
        assert!((pierce.explosive_power - 2.0).abs() < f32::EPSILON);
    }

    /// `impact.at` picks between the two ends of the beam and rejects anything else.
    #[test]
    fn the_blast_position_is_configurable() {
        let parse_at = |at: &str| {
            parse(&format!(
                r#"{{ "types": {{ "a": {{ "impact": {{ "at": "{at}" }} }} }}, "shells": {{}} }}"#
            ))
        };
        assert_eq!(
            parse_at("first").expect("valid").types[0].blast_at,
            BlastAt::First
        );
        assert_eq!(
            parse_at("last").expect("valid").types[0].blast_at,
            BlastAt::Last
        );
        let errors = parse_at("middle").expect_err("only first/last exist");
        assert!(errors.iter().any(|error| error.contains("first") || error.contains("last")));
    }

    /// `impact` is optional (older files keep working) and its numbers are validated like the rest.
    #[test]
    fn the_impact_block_is_optional_and_validated() {
        let snapshot = parse(r#"{ "types": { "a": { "range": 10.0 } }, "shells": {} }"#)
            .expect("no impact block is fine");
        assert!(!snapshot.types[0].explosion, "no blast unless asked for");
        assert_eq!(snapshot.types[0].blast_at, BlastAt::First, "default is the entry");
        assert_eq!(snapshot.types[0].explosive_power, 20.0);

        let snapshot = parse(
            r#"{ "types": { "a": { "impact": { "explosion": true, "explosive_power": 5.0 } } }, "shells": {} }"#,
        )
        .expect("valid");
        assert!(snapshot.types[0].explosion);
        assert_eq!(snapshot.types[0].explosive_power, 5.0);

        let errors = parse(
            r#"{ "types": { "a": { "impact": { "explosive_power": -1.0 } } }, "shells": {} }"#,
        )
        .expect_err("a negative power is meaningless");
        assert!(errors.iter().any(|error| error.contains("impact.explosive_power")));

        let errors = parse(
            r#"{ "types": { "a": { "impact": { "explosoin": true } } }, "shells": {} }"#,
        )
        .expect_err("a typo in the new block must not be silent");
        assert!(errors.iter().any(|error| error.contains("explosoin")));
    }

    #[test]
    fn ammo_binding_is_case_insensitive_and_ammo_only() {
        let snapshot = parse(SHIPPED).expect("valid");
        assert!(snapshot.laser_for_item("item_ammo_example4").is_some());
        assert!(snapshot.laser_for_item("  ITEM_AMMO_EXAMPLE1  ").is_some());
        assert!(
            snapshot.laser_for_item("ITEM_AMMO_130MM").is_none(),
            "unlisted ammo stays a normal shell"
        );
        assert!(snapshot.laser_for_item("").is_none());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let text = r#"{
            "enabled": true,
            "verbose_logging": true,
            "typo": 1,
            "types": {},
            "shells": {}
        }"#;
        assert!(parse(text).is_err(), "top-level typo");

        let text = r#"{
            "types": { "a": { "range": 10.0, "damge": 1.0 } },
            "shells": {}
        }"#;
        assert!(parse(text).is_err(), "type-level typo");

        let text = r##"{
            "types": { "a": { "range": 10.0, "visual": { "colur": "#FF0000" } } },
            "shells": {}
        }"##;
        assert!(parse(text).is_err(), "visual-level typo");
    }

    #[test]
    fn shells_must_name_an_existing_type() {
        let text = r#"{
            "types": { "laser": { "range": 10.0 } },
            "shells": { "ITEM_AMMO_130MM": "laser", "ITEM_AMMO_37MM": "missing" }
        }"#;
        let errors = parse(text).expect_err("an unknown type is a load error");
        assert!(
            errors.iter().any(|error| error.contains("ITEM_AMMO_37MM")),
            "the message names the key: {errors:?}"
        );
    }

    #[test]
    fn keys_that_differ_only_by_case_are_rejected() {
        let text = r#"{
            "types": { "laser": { "range": 10.0 }, "LASER": { "range": 20.0 } },
            "shells": {}
        }"#;
        assert!(parse(text).is_err(), "duplicate type names");

        let text = r#"{
            "types": { "laser": { "range": 10.0 } },
            "shells": { "ITEM_AMMO_130MM": "laser", "item_ammo_130mm": "laser" }
        }"#;
        assert!(parse(text).is_err(), "duplicate ammo names");
    }

    #[test]
    fn meaningless_numbers_are_rejected_not_clamped() {
        for (key, value) in [
            ("range", "0.0"),
            ("range", "-1.0"),
            ("damage", "-1.0"),
            ("damage_decay_per_unit", "-0.1"),
            ("incendiary", "-1.0"),
            ("spread", "-0.05"),
        ] {
            let text = format!(
                r#"{{ "types": {{ "a": {{ "range": 10.0, "{key}": {value} }} }}, "shells": {{}} }}"#
            );
            assert!(parse(&text).is_err(), "{key} = {value} must be rejected");
        }
        for (key, value) in [("width", "-1.0"), ("seconds", "-1.0")] {
            let text = format!(
                r#"{{ "types": {{ "a": {{ "visual": {{ "{key}": {value} }} }} }}, "shells": {{}} }}"#
            );
            assert!(parse(&text).is_err(), "visual.{key} = {value} must be rejected");
        }
        let text = r#"{
            "types": { "a": { "pierce": { "max_parts": -1 } } },
            "shells": {}
        }"#;
        assert!(parse(text).is_err(), "negative max_parts");
    }

    #[test]
    fn non_finite_numbers_are_rejected() {
        // 1e39 overflows f32 to +inf.
        let text = r#"{ "types": { "a": { "range": 1e39 } }, "shells": {} }"#;
        let errors = parse(text).expect_err("inf is not a number the mod accepts");
        assert!(
            errors.iter().any(|error| error.contains("finite")),
            "{errors:?}"
        );
    }

    #[test]
    fn zero_damage_is_allowed() {
        let text = r#"{ "types": { "marker": { "damage": 0.0 } }, "shells": {} }"#;
        let snapshot = parse(text).expect("a 0-damage marker laser is legal");
        assert_eq!(snapshot.types[0].damage, 0.0);
    }

    /// Phase 5.5: `spread` is **radians**, vanilla's own unit (§5.13), so the number in the file is
    /// what `HGE.Random_Float` receives — no degrees→radians conversion anywhere. Absent = `0.0` =
    /// pinpoint; every finite value from 0 up is taken as written (no clamp).
    #[test]
    fn spread_is_radians_optional_and_unclamped() {
        let snapshot = parse(r#"{ "types": { "a": { "range": 10.0 } }, "shells": {} }"#)
            .expect("valid");
        assert_eq!(snapshot.types[0].spread, 0.0, "absent spread is pinpoint");

        let snapshot = parse(
            r#"{ "types": { "a": { "range": 10.0, "spread": 0.05 } }, "shells": {} }"#,
        )
        .expect("valid");
        assert_eq!(snapshot.types[0].spread, 0.05, "stored as written, in radians");

        // Vanilla's two values, and a value no sane config would use, all pass through untouched.
        for value in ["0.02", "3.14159", "6.28318"] {
            let text = format!(
                r#"{{ "types": {{ "a": {{ "range": 10.0, "spread": {value} }} }}, "shells": {{}} }}"#
            );
            let snapshot = parse(&text).expect("a finite spread is accepted as written");
            assert_eq!(snapshot.types[0].spread, value.parse::<f32>().unwrap());
        }

        // The old degrees key is not silently accepted: a typo must reject the file.
        let errors = parse(
            r#"{ "types": { "a": { "range": 10.0, "spread_degrees": 2.8648 } }, "shells": {} }"#,
        )
        .expect_err("`spread_degrees` is not a key");
        assert!(
            errors.iter().any(|error| error.contains("spread_degrees")),
            "{errors:?}"
        );
    }

    #[test]
    fn stop_at_names_fold_into_one_mask_and_raw_bits_work() {
        let text = r#"{
            "types": { "a": { "pierce": { "enabled": true, "stop_at": ["block", "Hull", "ammo", 8192] } } },
            "shells": {}
        }"#;
        let snapshot = parse(text).expect("valid");
        assert_eq!(snapshot.types[0].stop_mask, 64 | 128 | 16 | 8192);

        let text = r#"{
            "types": { "a": { "pierce": { "stop_at": ["turret"] } } },
            "shells": {}
        }"#;
        let errors = parse(text).expect_err("an unknown name is not a number");
        assert!(errors.iter().any(|error| error.contains("turret")), "{errors:?}");
    }

    #[test]
    fn pierce_shorthand_expands_to_the_documented_defaults() {
        let text = r#"{
            "types": { "on": { "pierce": true }, "off": { "pierce": false } },
            "shells": {}
        }"#;
        let snapshot = parse(text).expect("valid");
        let on = snapshot.types.iter().find(|t| t.name == "on").unwrap();
        let off = snapshot.types.iter().find(|t| t.name == "off").unwrap();
        assert!(on.pierce);
        assert_eq!(on.stop_mask, 64);
        assert_eq!(on.max_parts, 12);
        assert!(!on.through_terrain, "the ground still stops a shot by default");
        assert!(!off.pierce);
    }

    /// `pierce.through_terrain` is a plain boolean: off unless the file asks for it, and an unknown
    /// key next to it still rejects the file.
    #[test]
    fn through_terrain_is_optional_and_off_by_default() {
        let text = r#"{
            "types": {
                "plain": { "pierce": { "enabled": true } },
                "ground": { "pierce": { "enabled": true, "max_parts": 0, "through_terrain": true } }
            },
            "shells": {}
        }"#;
        let snapshot = parse(text).expect("valid");
        let plain = snapshot.types.iter().find(|t| t.name == "plain").unwrap();
        let ground = snapshot.types.iter().find(|t| t.name == "ground").unwrap();
        assert!(!plain.through_terrain);
        assert!(ground.through_terrain);
        assert_eq!(ground.max_parts, 0);

        let bad = r#"{
            "types": { "a": { "pierce": { "throughTerrain": true } } },
            "shells": {}
        }"#;
        assert!(parse(bad).is_err(), "an unknown key is still a load error");
    }

    #[test]
    fn colours_accept_three_forms_and_reject_the_rest() {
        assert_eq!(parse_colour("#FF3322"), Some(0xFFFF_3322));
        assert_eq!(parse_colour("ff3322"), Some(0xFFFF_3322));
        assert_eq!(parse_colour("#80FF3322"), Some(0x80FF_3322));
        assert_eq!(parse_colour("red"), None);
        assert_eq!(parse_colour("#FF33"), None);
        assert_eq!(parse_colour("#GG3322"), None);

        let text = r#"{
            "types": { "int": { "visual": { "color": 4294901760 } } },
            "shells": {}
        }"#;
        assert_eq!(parse(text).expect("valid").types[0].color, 0xFFFF_0000);

        let text = r#"{
            "types": { "bad": { "visual": { "color": "red" } } },
            "shells": {}
        }"#;
        assert!(parse(text).is_err());
    }

    #[test]
    fn glow_colour_is_derived_at_twenty_percent_alpha() {
        let text = r##"{
            "types": { "a": { "visual": { "color": "#FF3322", "glow": true } } },
            "shells": {}
        }"##;
        let snapshot = parse(text).expect("valid");
        assert_eq!(snapshot.types[0].color, 0xFFFF_3322);
        assert_eq!(snapshot.types[0].glow_color, 0x33FF_3322);

        let text = r##"{
            "types": { "a": { "visual": { "glow_color": "#40FFAA00" } } },
            "shells": {}
        }"##;
        assert_eq!(
            parse(text).expect("valid").types[0].glow_color,
            0x40FF_AA00
        );
    }

    #[test]
    fn a_rejected_file_collects_every_problem() {
        let text = r#"{
            "types": { "a": { "range": -1.0, "damage": -2.0 } },
            "shells": { "ITEM_X": "missing" }
        }"#;
        let errors = parse(text).expect_err("invalid");
        assert!(errors.len() >= 3, "one message per problem: {errors:?}");
    }

    #[test]
    fn an_empty_config_is_valid_but_inert() {
        let snapshot = parse("{}").expect("empty config");
        assert!(snapshot.enabled);
        assert_eq!(snapshot.type_count(), 0);
        assert_eq!(snapshot.shell_count(), 0);
    }
}
