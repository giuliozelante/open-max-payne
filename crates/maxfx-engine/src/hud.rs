//! The player's HUD, laid out by `data/hud/hud.txt` in 640x480 space.
//! Layout and evidence: docs/rea/2026-11-04-hud.md.

use std::collections::BTreeMap;

/// A corner or edge of a sprite, as `hud.txt` defines `ReferencePoint`.
pub type ReferencePoint = u8;

/// A bitmap placed in 640x480 space.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HudSprite {
    pub reference: ReferencePoint,
    pub position: [f32; 2],
    pub width: f32,
    pub height: f32,
    pub file: String,
    pub alpha_file: String,
    pub alpha: u8,
}

/// Where a line of text is anchored in 640x480 space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HudText {
    pub reference: ReferencePoint,
    pub position: [f32; 2],
}

/// `[Print]` or `[PrintTip]` of `hud.txt`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HudPrintStyle {
    pub anchor: HudText,
    pub spacing: f32,
    pub max_lines: usize,
    pub color: [u8; 4],
    /// Seconds on screen, then seconds of fade.
    pub duration: f32,
    pub fade_out: f32,
}

/// Lines of text on the HUD, oldest first, each with its age.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HudPrintLog {
    pub lines: std::collections::VecDeque<(String, f32)>,
}

impl HudPrintLog {
    pub fn push(&mut self, style: &HudPrintStyle, text: String) {
        self.lines.push_back((text, 0.0));
        while self.lines.len() > style.max_lines {
            self.lines.pop_front();
        }
    }

    pub fn step(&mut self, style: &HudPrintStyle, seconds: f32) {
        for line in &mut self.lines {
            line.1 += seconds;
        }
        self.lines
            .retain(|line| line.1 < style.duration + style.fade_out);
    }

    /// `(text, position, alpha)` of each line. Tips grow down from their anchor; text anchored
    /// to the right grows up from it, newest lowest.
    pub fn placed(&self, style: &HudPrintStyle) -> Vec<(&str, [f32; 2], u8)> {
        let [x, y] = style.anchor.position;
        let grows_up = style.anchor.reference == 5;
        let last = self.lines.len().saturating_sub(1);
        self.lines
            .iter()
            .enumerate()
            .map(|(row, (text, age))| {
                let step = if grows_up {
                    (row as f32 - last as f32) * style.spacing
                } else {
                    row as f32 * style.spacing
                };
                let past = (age - style.duration).max(0.0) / style.fade_out.max(1e-3);
                let alpha = (f32::from(style.color[3]) * (1.0 - past).max(0.0)) as u8;
                (text.as_str(), [x, y + step], alpha)
            })
            .collect()
    }
}

/// `database/stringtable.txt`: `[ID] String = "text";`.
pub fn parse_string_table(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (id, rest) = line.trim().strip_prefix('[')?.split_once(']')?;
            let value = rest.split_once('"')?.1.rsplit_once('"')?.0;
            Some((id.to_string(), value.to_string()))
        })
        .collect()
}

/// `#USE#`-style placeholders in `text` replaced by the engine's keys.
pub fn expand_keys(text: &str) -> String {
    const KEYS: [(&str, &str); 4] = [
        ("#USE#", "E"),
        ("#SHOOT#", "Mouse 1"),
        ("#BULLETTIME#", "Mouse 2"),
        ("#JUMP#", "Space"),
    ];
    KEYS.iter().fold(text.to_string(), |text, (token, key)| {
        text.replace(token, key)
    })
}

/// `text` broken at spaces so no line is wider than `width`.
pub fn wrap_text(font: &HudFont, text: &str, width: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(line) if font.size(&format!("{line} {word}"))[0] <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// The parts of `hud.txt` the engine draws.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HudLayout {
    pub health_background: HudSprite,
    pub health_pain: HudSprite,
    pub hourglass_background: HudSprite,
    pub hourglass: HudSprite,
    pub painkiller_icon: HudSprite,
    pub painkiller_count: HudText,
    pub ammo_in_clip: HudText,
    pub ammo_in_pocket: HudText,
    /// `[ActiveWeapon]` sprites by `id`, such as `WEAPONID_BERETTA`.
    pub weapons: BTreeMap<String, HudSprite>,
    /// `UnlimitedAmmoText` by `id`.
    pub unlimited_text: BTreeMap<String, String>,
    pub hourglass_colors: HourglassColors,
    pub print: HudPrintStyle,
    pub tip: HudPrintStyle,
}

/// `[SlowMotionSpecific]`, as RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HourglassColors {
    pub active: [u8; 4],
    pub normal: [u8; 4],
    pub unavailable: [u8; 4],
}

impl HourglassColors {
    /// `0x4aef24`: the state of the slow motion picks the hourglass colour.
    pub fn pick(&self, gauge: &SlowMotion) -> [u8; 4] {
        if gauge.active {
            self.active
        } else if gauge.level > gauge.threshold {
            self.normal
        } else {
            self.unavailable
        }
    }
}

/// The bullet-time gauge of `globalconfig.txt` `[slowmotion]`.
#[derive(Debug, Clone, PartialEq)]
pub struct SlowMotion {
    /// 0..1.
    pub level: f32,
    pub active: bool,
    /// `SlowMotionTime`: seconds a full gauge lasts.
    pub seconds: f32,
    /// `Threshold`: the level it takes to start.
    pub threshold: f32,
    /// `HeartbeatSoundMessage`, sent every `HeartbeatSoundInterval` seconds while active.
    pub heartbeat_message: String,
    pub heartbeat_interval: f32,
    /// Seconds until the next heartbeat.
    pub heartbeat_in: f32,
    /// `RegenerationTime`: seconds to refill from empty while inactive.
    pub regeneration_seconds: f32,
    /// `characters.h` `BONUS_SLOWMOTION`: the share of the gauge a kill gives.
    pub kill_bonus: f32,
    /// `[StartMessages]`, `[EndMessages]` and `[CantSwitchOnMessages]`.
    pub start_messages: Vec<String>,
    pub end_messages: Vec<String>,
    pub refused_messages: Vec<String>,
}

impl SlowMotion {
    pub fn parse(globalconfig: &str, characters_h: &str) -> Option<Self> {
        let sections = parse_sections(globalconfig);
        let bonus = characters_h
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("#define")?
                    .trim()
                    .strip_prefix("BONUS_SLOWMOTION")
            })
            .and_then(|value| value.trim().parse().ok());
        let general = sections.iter().find(|s| s.path == "slowmotion/general")?;
        let threshold = general.text("threshold")?.strip_suffix('%')?;
        Some(Self {
            level: 1.0,
            active: false,
            seconds: general.number("slowmotiontime")?,
            threshold: threshold.trim().parse::<f32>().ok()? / 100.0,
            heartbeat_message: general.file("heartbeatsoundmessage")?,
            heartbeat_interval: general.number("heartbeatsoundinterval")?,
            heartbeat_in: 0.0,
            regeneration_seconds: general.number("regenerationtime")?,
            kill_bonus: bonus?,
            start_messages: config_messages(globalconfig, "[StartMessages]"),
            end_messages: config_messages(globalconfig, "[EndMessages]"),
            refused_messages: config_messages(globalconfig, "[CantSwitchOnMessages]"),
        })
    }

    /// The messages for a step from `was_active` with the player `asking` for the first time.
    pub fn transition(&self, was_active: bool, asking: bool) -> &[String] {
        match (was_active, self.active) {
            (false, true) => &self.start_messages,
            (true, false) => &self.end_messages,
            (false, false) if asking => &self.refused_messages,
            _ => &[],
        }
    }

    /// A kill fills the gauge by `kill_bonus`.
    pub fn reward_kill(&mut self) {
        self.level = (self.level + self.kill_bonus).min(1.0);
    }

    /// The heartbeat message when one is due. The first comes as slow motion starts.
    pub fn heartbeat(&mut self, seconds: f32) -> Option<&str> {
        if !self.active {
            self.heartbeat_in = 0.0;
            return None;
        }
        self.heartbeat_in -= seconds;
        if self.heartbeat_in > 0.0 {
            return None;
        }
        self.heartbeat_in = self.heartbeat_interval;
        Some(&self.heartbeat_message)
    }

    /// Advances `seconds` of real time while the player does or does not ask for slow motion.
    pub fn step(&mut self, wanted: bool, seconds: f32) {
        self.active = wanted && (self.active || self.level > self.threshold);
        if self.active {
            self.level -= seconds / self.seconds;
            if self.level <= 0.0 {
                self.level = 0.0;
                self.active = false;
            }
        } else {
            self.level = (self.level + seconds / self.regeneration_seconds).min(1.0);
        }
    }
}

/// The `[Message] String`s of the brace block after `header`, comments left out.
fn config_messages(text: &str, header: &str) -> Vec<String> {
    text.lines()
        .skip_while(|line| !line.trim().eq_ignore_ascii_case(header))
        .skip(1)
        .take_while(|line| !line.trim().starts_with('}'))
        .filter(|line| !line.trim().starts_with("//"))
        .filter_map(|line| {
            let (_, value) = line.split_once("String")?.1.split_once('=')?;
            let value = value.trim().trim_end_matches(';').trim();
            Some(value.trim_matches('"').to_string())
        })
        .collect()
}

/// One `[Section]` of `hud.txt`: its lower-case path, such as `health/sprite`, and its keys.
pub(crate) struct Section {
    pub(crate) path: String,
    keys: BTreeMap<String, String>,
}

impl Section {
    pub(crate) fn text(&self, key: &str) -> Option<&str> {
        self.keys.get(key).map(String::as_str)
    }

    pub(crate) fn number(&self, key: &str) -> Option<f32> {
        self.text(key)?.parse().ok()
    }

    fn reference(&self) -> Option<ReferencePoint> {
        self.text("referencepoint")?.parse().ok()
    }

    /// `( x, y, 0 )`.
    fn position(&self) -> Option<[f32; 2]> {
        let body = self.text("position")?.trim_matches(['(', ')', ' ']);
        let mut parts = body.split(',').map(|part| part.trim().parse::<f32>());
        Some([parts.next()?.ok()?, parts.next()?.ok()?])
    }

    pub(crate) fn file(&self, key: &str) -> Option<String> {
        Some(self.text(key)?.trim_matches('"').to_string())
    }

    fn sprite(&self) -> Option<HudSprite> {
        Some(HudSprite {
            reference: self.reference()?,
            position: self.position()?,
            width: self.number("width")?,
            height: self.number("height")?,
            file: self.file("filename")?,
            alpha_file: self.file("alphafilename")?,
            alpha: self.number("defaultvertexalpha")? as u8,
        })
    }

    /// `rrggbbaa`.
    fn color(&self, key: &str) -> [u8; 4] {
        let value = self
            .text(key)
            .and_then(|hex| u32::from_str_radix(hex, 16).ok());
        value.unwrap_or(0).to_be_bytes()
    }

    fn hourglass_colors(&self) -> HourglassColors {
        HourglassColors {
            active: self.color("coloractive"),
            normal: self.color("colornormal"),
            unavailable: self.color("colorunavailable"),
        }
    }

    fn anchor(&self) -> Option<HudText> {
        Some(HudText {
            reference: self.reference()?,
            position: self.position()?,
        })
    }
}

/// `text` without `/* */` blocks.
fn strip_block_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start..]
            .find("*/")
            .map_or("", |end| &rest[start + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// `value` with each `#define`d word replaced.
fn expand(value: &str, defines: &BTreeMap<String, String>) -> String {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut word = String::new();
    for c in value.chars().chain(std::iter::once(' ')) {
        if is_word(c) {
            word.push(c);
            continue;
        }
        out.push_str(defines.get(&word).unwrap_or(&word));
        word.clear();
        out.push(c);
    }
    out.pop();
    out
}

/// Sections in file order. A `[Name]` before `{` opens a block; the others end at the next one.
pub(crate) fn parse_sections(text: &str) -> Vec<Section> {
    let mut defines = BTreeMap::new();
    let mut sections: Vec<Section> = Vec::new();
    let mut blocks: Vec<String> = Vec::new();
    let mut last_name = String::new();
    for line in strip_block_comments(text).lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if let Some(define) = line.strip_prefix("#define") {
            if let Some((name, value)) = define.trim().split_once(char::is_whitespace) {
                defines.insert(name.to_string(), value.trim().to_string());
            }
        } else if line == "{" {
            blocks.push(last_name.clone());
        } else if line == "}" {
            blocks.pop();
        } else if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            last_name = name.to_ascii_lowercase();
            let mut path = blocks.join("/");
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(&last_name);
            sections.push(Section {
                path,
                keys: BTreeMap::new(),
            });
        } else if let (Some((key, value)), Some(section)) =
            (line.split_once('='), sections.last_mut())
        {
            let value = value.trim().trim_end_matches(';').trim();
            section
                .keys
                .insert(key.trim().to_ascii_lowercase(), expand(value, &defines));
        }
    }
    sections
}

fn print_style(sections: &[Section], block: &str) -> Option<HudPrintStyle> {
    let find = |name: &str| {
        let path = format!("{block}/{name}");
        sections.iter().find(|s| s.path == path)
    };
    let specific = find("printspecific")?;
    let visibility = find("visibility")?;
    Some(HudPrintStyle {
        anchor: find("text")?.anchor()?,
        spacing: specific.number("yspacing")?,
        max_lines: specific.number("maximumlines")? as usize,
        color: specific.color("color"),
        duration: visibility.number("duration")?,
        fade_out: visibility.number("fadeouttime")?,
    })
}

impl HudLayout {
    pub fn parse(text: &str) -> Option<Self> {
        let sections = parse_sections(text);
        let find = |path: &str| sections.iter().find(|s| s.path == path);
        let sprite = |path: &str| find(path)?.sprite();
        let anchor = |path: &str| find(path)?.anchor();
        let mut weapons = BTreeMap::new();
        let mut unlimited_text = BTreeMap::new();
        let mut id = None;
        for section in &sections {
            match section.path.as_str() {
                "weapons/activeweapon/weaponspecific" => {
                    id = section.text("id");
                    if let (Some(id), Some(text)) = (id, section.file("unlimitedammotext")) {
                        unlimited_text.insert(id.to_string(), text);
                    }
                }
                "weapons/activeweapon/sprite" => {
                    weapons.insert(id?.to_string(), section.sprite()?);
                }
                _ => {}
            }
        }
        Some(Self {
            health_background: sprite("health/backgroundsprite")?,
            health_pain: sprite("health/sprite")?,
            hourglass_background: sprite("slowmotion/backgroundsprite")?,
            hourglass: sprite("slowmotion/sprite")?,
            painkiller_icon: sprite("weapons/painkiller/sprite")?,
            painkiller_count: anchor("weapons/painkiller/amounttext")?,
            ammo_in_clip: anchor("weapons/ammoinclipstext")?,
            ammo_in_pocket: anchor("weapons/ammoinpockettext")?,
            weapons,
            unlimited_text,
            hourglass_colors: find("slowmotion/slowmotionspecific")
                .map(Section::hourglass_colors)
                .unwrap_or_default(),
            print: print_style(&sections, "print").unwrap_or_default(),
            tip: print_style(&sections, "printtip").unwrap_or_default(),
        })
    }
}

/// `{base}.tga` and `{base}_alpha.tga` from the game data.
pub fn load_font(game_dir: &std::path::Path, base: &str) -> Option<HudFont> {
    let read =
        |path: String| crate::scene::decode_texture_blob(&crate::read_game_bytes(game_dir, &path)?);
    let (width, height, colour) = read(format!("{base}.tga"))?;
    let (_, _, alpha) = read(format!("{base}_alpha.tga"))?;
    let pixels = colour
        .iter()
        .zip(&alpha)
        .map(|(rgb, grey)| ((grey & 0x00ff_0000) << 8) | (rgb & 0x00ff_ffff))
        .collect();
    HudFont::new(crate::scene::DecodedTexture {
        path: base.to_string(),
        flag: 0,
        width,
        height,
        pixels,
    })
}

/// What one `P_CreateProjectile` of a shot does, from `database/projectiles/<name>.txt`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProjectileDef {
    /// The message's second argument.
    pub count: u32,
    pub damage: i32,
    /// `Accuracy`: the spread, up to `Accuracy / 2500` off the aim (`0x7068cd`).
    pub accuracy: f32,
    /// `D_CreateDecal` block of `decals.txt`.
    pub decal: Option<String>,
    /// Its `[CharacterHit]` projectile, with that file's values.
    pub character_hit: Option<SplatterDef>,
}

/// A projectile that only flies and marks where it lands, such as `blood_bullet_small`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SplatterDef {
    pub count: u32,
    /// `Speed`, metres per second.
    pub speed: f32,
    pub accuracy: f32,
    /// `GravityMultiplier`.
    pub gravity: f32,
    pub decal: Option<String>,
}

impl SplatterDef {
    pub fn parse(text: &str, count: u32) -> Option<Self> {
        let sections = parse_sections(text);
        let number = |name: &str| {
            let value = sections.iter().find_map(|s| s.text(name))?;
            value.parse::<f32>().ok()
        };
        Some(Self {
            count,
            speed: number("speed")?,
            accuracy: number("accuracy").unwrap_or(0.0),
            gravity: number("gravitymultiplier").unwrap_or(0.0),
            // Blood files spell the verb in lower case.
            decal: weapon_cue_line(&text.to_ascii_lowercase(), "d_createdecal", 0),
        })
    }
}

/// `(projectile, count)` of a projectile file's `[CharacterHit]`.
pub fn parse_character_hit(text: &str) -> Option<(String, u32)> {
    let (_, rest) = text.split_once("[CharacterHit]")?;
    let block = rest.split("\n[").next()?;
    let name = weapon_cue_line(block, "P_CreateProjectile", 0)?;
    let count = weapon_cue_line(block, "P_CreateProjectile", 1)?;
    Some((name, count.parse().ok()?))
}

impl ProjectileDef {
    /// `shooting_h` names the numbers. `None` for what does no damage, such as a spent case.
    pub fn parse(shooting_h: &str, text: &str, count: u32) -> Option<Self> {
        let sections = parse_sections(&format!("{shooting_h}\n{text}"));
        let key = |name: &str| sections.iter().find_map(|s| s.text(name));
        let damage = key("damage")?.parse().ok().filter(|damage| *damage > 0)?;
        Some(Self {
            count,
            damage,
            accuracy: key("accuracy")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0.0),
            decal: weapon_cue_line(text, "D_CreateDecal", 0),
            character_hit: None,
        })
    }
}

/// One `WS_AnimateShooting` of a shoot clip.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ShotRound {
    /// Frame, then seconds once the clip's rate is known.
    pub at: f32,
    /// `WEAPONANIM_SHOOT_NOEFFECTS`.
    pub quiet: bool,
    /// `LeftHandWeapon`.
    pub left: bool,
}

/// The rounds of a `CHARANIM_SHOOT<WEAPON>` clip in order, `at` in frames.
pub fn parse_rounds(skeleton: &str, index: &str) -> Vec<ShotRound> {
    let header = format!("Index = {index};");
    skeleton
        .lines()
        .skip_while(|line| !line.contains(&header))
        .skip(1)
        .take_while(|line| !line.contains("[Animation]"))
        .filter(|line| line.contains("WS_AnimateShooting"))
        .filter_map(|line| {
            let frame = line.split_once("Frame =")?.1.split(';').next()?;
            Some(ShotRound {
                at: frame.trim().parse().ok()?,
                quiet: line.contains("WEAPONANIM_SHOOT_NOEFFECTS"),
                left: line.contains("LeftHandWeapon"),
            })
        })
        .collect()
}

/// `(projectile, count)` of each `P_CreateProjectile` in the weapon's `[Animation]` of `index`.
pub fn parse_shot_projectiles(weapon_txt: &str, index: &str) -> Vec<(String, u32)> {
    let header = format!("Index = {index};");
    weapon_txt
        .lines()
        .skip_while(|line| !line.contains(&header))
        .skip(1)
        .take_while(|line| !line.contains("[Animation]"))
        .filter_map(|line| {
            let name = weapon_cue_line(line, "P_CreateProjectile", 0)?;
            let count = weapon_cue_line(line, "P_CreateProjectile", 1)?;
            Some((name, count.parse().ok()?))
        })
        .collect()
}

/// Third `P_Explosion` argument of a melee projectile. HYPOTHESIS: its damage (9 for the pipe).
pub fn parse_melee_damage(projectile_txt: &str) -> Option<i32> {
    let damage = weapon_cue_line(projectile_txt, "P_Explosion", 2)?;
    damage.parse::<f32>().ok().map(|damage| damage as i32)
}

/// What `database/weapons/<name>.txt` says about a weapon's ammunition.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WeaponDef {
    /// `WeaponID`, the key of the weapon's `[ActiveWeapon]` sprite.
    pub id: String,
    /// `InventoryID`. Weapons that share it share ammunition, like single and dual Berettas.
    pub inventory_id: String,
    pub clip_size: u32,
    pub pocket_size: u32,
    /// Time from the start of the reload animation to its `C_Reload` frame.
    pub reload_seconds: f32,
    /// `ReloadAmount`: rounds per `C_Reload`, or -1 to fill the clip.
    pub reload_amount: i32,
    /// `[Crosshair]` names `dot.kf2` and not `invisible.kf2`.
    pub shows_dot: bool,
    /// First `[LOD]` `ExportData`, relative to `database\\weapons\\`.
    pub model: Option<String>,
    /// What a shot throws instead of firing rays, with its `P_CreateProjectile` count.
    pub flying: Vec<(crate::projectiles::FlyingDef, u32)>,
    /// `DefaultShootingFrequency`, shots per second.
    pub shots_per_second: f32,
    /// `ShootingFrequencyMultiplier`: 0 slows firing with time, 1 keeps it constant.
    pub slow_motion_rate: f32,
    /// `WEAPONANIM_SHOOT`'s `A_Play3DSound(weapons, ...)` cue.
    pub fire_sound: Option<String>,
    /// `WEAPONANIM_SHOOT`'s `PS_StartEffect` effect.
    pub muzzle: Option<String>,
    /// `WEAPONANIM_RELOAD`'s `A_Play3DSound(weapons, ...)` cue.
    pub reload_sound: Option<String>,
    /// Seconds into the reload clip of its `WS_Animate(WEAPONANIM_RELOAD)`.
    pub reload_sound_delay: f32,
    /// `CHARANIM_SHOOT<WEAPON>`.
    pub fire_clip: Option<crate::kf2::Kf2Anim>,
    /// `CHARANIM_RELOAD<WEAPON>`.
    pub reload_clip: Option<crate::kf2::Kf2Anim>,
    /// `TwoHandedAnimation`: Max carries it with the `CHARANIM_2_*` clips.
    pub two_handed: bool,
    /// What one shot or melee blow creates.
    pub projectiles: Vec<ProjectileDef>,
    /// What a `WEAPONANIM_SHOOT_NOEFFECTS` round creates.
    pub quiet_projectiles: Vec<ProjectileDef>,
    /// `parse_rounds` of the weapon's shoot clip, `at` in seconds.
    pub rounds: Vec<ShotRound>,
    /// `UseMultipleGunBones`: a gun in each hand.
    pub dual: bool,
    /// `TwoHandedBones`: the upper-body bone set.
    pub two_handed_bones: bool,
    /// Metres a melee weapon reaches; `None` for a gun.
    pub melee_reach: Option<f32>,
    /// Seconds into the shoot clip of each melee blow.
    pub melee_blows: Vec<f32>,
}

impl WeaponDef {
    /// Reads a weapon file, with `shooting.h` for the sizes it names.
    pub fn parse(shooting_h: &str, weapon_txt: &str) -> Option<Self> {
        let sections = parse_sections(&format!("{shooting_h}\n{weapon_txt}"));
        let key = |name: &str| sections.iter().find_map(|s| s.text(name));
        Some(Self {
            flying: Vec::new(),
            id: key("weaponid")?.to_string(),
            inventory_id: key("inventoryid")?.to_string(),
            clip_size: key("clipsize")?.parse().ok()?,
            pocket_size: key("pocketsize")?.parse().ok()?,
            reload_seconds: 0.0,
            reload_amount: key("reloadamount")?.parse().ok()?,
            shows_dot: sections
                .iter()
                .find(|s| s.path == "crosshair/geometry")
                .and_then(|s| s.text("exportdata"))
                .is_some_and(|file| file.to_ascii_lowercase().ends_with("dot.kf2")),
            shots_per_second: key("defaultshootingfrequency")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0.0),
            slow_motion_rate: key("shootingfrequencymultiplier")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0.0),
            fire_sound: weapon_cue(weapon_txt, "WEAPONANIM_SHOOT", "A_Play3DSound", 1),
            muzzle: weapon_cue(weapon_txt, "WEAPONANIM_SHOOT", "PS_StartEffect", 0),
            reload_sound: weapon_cue(weapon_txt, "WEAPONANIM_RELOAD", "A_Play3DSound", 1),
            reload_sound_delay: 0.0,
            fire_clip: None,
            reload_clip: None,
            two_handed: key("twohandedanimation")
                .is_some_and(|flag| flag.eq_ignore_ascii_case("yes")),
            projectiles: Vec::new(),
            quiet_projectiles: Vec::new(),
            rounds: Vec::new(),
            dual: key("usemultiplegunbones").is_some_and(|flag| flag.eq_ignore_ascii_case("yes")),
            two_handed_bones: key("twohandedbones")
                .is_some_and(|flag| flag.eq_ignore_ascii_case("yes")),
            melee_reach: None,
            melee_blows: Vec::new(),
            model: weapon_txt
                .split_once("[LOD]")
                .and_then(|(_, lod)| lod.split_once("ExportData"))
                .and_then(|(_, rest)| {
                    let file = rest
                        .trim_start_matches([' ', '\t', '='])
                        .split(';')
                        .next()?;
                    Some(file.trim().trim_start_matches(r"..\").to_string())
                }),
        })
    }
}

/// Reach of a melee projectile: it flies `Speed` for `ExplosionDelay`, then `P_Explosion` reaches
/// its first argument further. HYPOTHESIS: that argument is the blast radius.
pub fn parse_melee_reach(projectile_txt: &str) -> Option<f32> {
    let attribute = |name: &str| {
        projectile_txt.lines().find_map(|line| {
            let (key, value) = line.split_once('=')?;
            let value = value.split(';').next()?.trim();
            key.trim()
                .eq_ignore_ascii_case(name)
                .then_some(value)?
                .parse::<f32>()
                .ok()
        })
    };
    let blast = weapon_cue_line(projectile_txt, "P_Explosion", 0)?
        .parse::<f32>()
        .ok()?;
    Some(attribute("Speed")? * attribute("ExplosionDelay")? + blast)
}

/// Argument `arg` of the first `verb` call in `lines`.
pub(crate) fn weapon_cue_line(text: &str, verb: &str, arg: usize) -> Option<String> {
    let (_, call) = text.split_once(verb)?;
    let args = call.trim_start().strip_prefix('(')?;
    let args = args.split(')').next()?;
    Some(args.split(',').nth(arg)?.trim().to_string())
}

/// `(frame, projectile)` of each `p_createprojectiletobone` in the `[Animation]` of `index`.
pub fn parse_bone_projectiles(skeleton: &str, index: &str) -> Vec<(u32, String)> {
    let header = format!("Index = {index};");
    skeleton
        .lines()
        .skip_while(|line| !line.contains(&header))
        .skip(1)
        .take_while(|line| !line.contains("[Animation]"))
        .filter_map(|line| {
            let frame = line.split_once("Frame =")?.1.split(';').next()?;
            let name = weapon_cue_line(line, "p_createprojectiletobone", 0)?;
            Some((frame.trim().parse().ok()?, name))
        })
        .collect()
}

impl WeaponDef {
    /// What a round creates.
    pub fn round_projectiles(&self, quiet: bool) -> &[ProjectileDef] {
        if quiet && !self.quiet_projectiles.is_empty() {
            &self.quiet_projectiles
        } else {
            &self.projectiles
        }
    }
}

/// `(frame, catalog, cue)` of each `A_Play3DSound` in the `[Animation]` of `index`.
pub fn parse_clip_sounds(skeleton: &str, index: &str) -> Vec<(u32, String, String)> {
    let header = format!("Index = {index};");
    skeleton
        .lines()
        .skip_while(|line| !line.contains(&header))
        .skip(1)
        .take_while(|line| !line.contains("[Animation]"))
        .filter_map(|line| {
            let frame = line.split_once("Frame =")?.1.split(';').next()?;
            let catalog = weapon_cue_line(line, "A_Play3DSound", 0)?;
            let cue = weapon_cue_line(line, "A_Play3DSound", 1)?;
            Some((frame.trim().parse().ok()?, catalog, cue))
        })
        .collect()
}

/// `(frame, projectile, count)` of each `P_CreateProjectileToBone` in the `[Animation]` of `index`.
pub fn parse_clip_projectiles(skeleton: &str, index: &str) -> Vec<(u32, String, u32)> {
    let header = format!("Index = {index};");
    skeleton
        .lines()
        .skip_while(|line| !line.contains(&header))
        .skip(1)
        .take_while(|line| !line.contains("[Animation]"))
        .filter_map(|line| {
            // The skeleton spells the verb in more than one case.
            let line = line.to_ascii_lowercase();
            let frame = line.split_once("frame =")?.1.split(';').next()?;
            let name = weapon_cue_line(&line, "p_createprojectiletobone", 0)?;
            let count = weapon_cue_line(&line, "p_createprojectiletobone", 1)?;
            Some((frame.trim().parse().ok()?, name, count.parse().ok()?))
        })
        .collect()
}

/// Argument `arg` of the first `verb` message in the `[Animation]` of `index`.
pub(crate) fn weapon_cue(weapon_txt: &str, index: &str, verb: &str, arg: usize) -> Option<String> {
    let header = format!("Index = {index};");
    let lines = weapon_txt
        .lines()
        .skip_while(|line| !line.contains(&header))
        .skip(1)
        .take_while(|line| !line.contains("[Animation]"));
    for line in lines {
        let Some((_, call)) = line.split_once(verb) else {
            continue;
        };
        let args = call.trim_start().strip_prefix('(')?;
        return Some(args.split(',').nth(arg)?.trim().to_string());
    }
    None
}

/// The weapon IDs of `weaponpriority.txt` `[BestWeapons]`, strongest first.
pub fn parse_best_weapons(text: &str) -> Vec<String> {
    let Some((_, block)) = text.split_once("[BestWeapons]") else {
        return Vec::new();
    };
    let block = block.split('}').next().unwrap_or("");
    block
        .lines()
        .filter_map(|line| line.split_once("ID =")?.1.split(';').next())
        .map(|id| id.trim().to_string())
        .collect()
}

/// `CHARANIM_SHOOT<WEAPON>` file per weapon, `[Revert]`s followed. The `LOW`, `HIGH` and dodge
/// variants have their own names and stay in the map unused.
pub fn parse_shoot_animations(skeleton: &str) -> BTreeMap<String, String> {
    let after = |line: &str, key: &str| {
        let value = line.split_once(key)?.1.split(';').next()?;
        Some(value.trim().to_string())
    };
    let mut shots = BTreeMap::new();
    let mut reverts = Vec::new();
    for line in skeleton.lines() {
        if line.contains("[Revert]") {
            let from = after(line, "From = CHARANIM_SHOOT");
            reverts.extend(from.zip(after(line, "To = CHARANIM_SHOOT")));
        } else if line.contains("[Animation]") {
            let weapon = after(line, "Index = CHARANIM_SHOOT");
            if let Some((weapon, file)) = weapon.zip(after(line, "Filename =")) {
                shots.insert(weapon.to_ascii_lowercase(), file);
            }
        }
    }
    for (from, to) in reverts {
        if let Some(file) = shots.get(&to.to_ascii_lowercase()).cloned() {
            shots.insert(from.to_ascii_lowercase(), file);
        }
    }
    shots
}

/// Frame of `WS_Animate(WEAPONANIM_RELOAD)` per `CHARANIM_RELOAD<WEAPON>` block.
pub fn parse_reload_sound_frames(skeleton: &str) -> BTreeMap<String, u32> {
    let mut frames = BTreeMap::new();
    let mut weapon = None;
    for line in skeleton.lines() {
        if line.contains("[Animation]") {
            weapon = line
                .split_once("CHARANIM_RELOAD")
                .and_then(|(_, rest)| rest.split(';').next())
                .map(|name| name.trim().to_ascii_lowercase());
        } else if line.contains("WS_Animate") && line.contains("WEAPONANIM_RELOAD") {
            let frame = line
                .split_once("Frame =")
                .and_then(|(_, rest)| rest.split(';').next()?.trim().parse().ok());
            if let (Some(weapon), Some(frame)) = (&weapon, frame) {
                frames.entry(weapon.clone()).or_insert(frame);
            }
        }
    }
    frames
}

/// `(animation file, frame of its C_Reload)` per `CHARANIM_RELOAD<WEAPON>` block of a skeleton.
pub fn parse_reload_animations(skeleton: &str) -> BTreeMap<String, (String, u32)> {
    let mut reloads = BTreeMap::new();
    let mut animation: Option<(String, String)> = None;
    let mut reverts = Vec::new();
    let reload_after = |line: &str, key: &str| {
        let value = line.split_once(key)?.1.split(';').next()?;
        Some(value.trim().to_ascii_lowercase())
    };
    for line in skeleton.lines() {
        if line.contains("[Revert]") {
            let from = reload_after(line, "From = CHARANIM_RELOAD");
            reverts.extend(from.zip(reload_after(line, "To = CHARANIM_RELOAD")));
        } else if line.contains("[Animation]") {
            let field = |key: &str| {
                let value = line.split_once(key)?.1.split(';').next()?;
                Some(value.trim().to_string())
            };
            let weapon = reload_after(line, "CHARANIM_RELOAD");
            animation = weapon.zip(field("Filename ="));
        } else if line.contains("C_Reload") {
            let frame = line
                .split_once("Frame =")
                .and_then(|(_, rest)| rest.split(';').next()?.trim().parse::<u32>().ok());
            if let (Some((weapon, file)), Some(frame)) = (&animation, frame) {
                reloads.insert(weapon.clone(), (file.clone(), frame));
            }
        }
    }
    for (from, to) in reverts {
        if let Some(reload) = reloads.get(&to).cloned() {
            reloads.insert(from, reload);
        }
    }
    reloads
}

/// The rounds of one `InventoryID`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WeaponStock {
    pub clip: u32,
    pub pocket: u32,
    pub infinite: bool,
}

/// The player's weapons, changed by the `C_PickupWeapon` family of script messages.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerInventory {
    /// Weapon definitions by lower-case file name, such as `beretta`.
    pub defs: BTreeMap<String, WeaponDef>,
    /// `weaponpriority.txt` `[BestWeapons]`.
    pub best_weapons: Vec<String>,
    /// Carried weapons, by lower-case file name.
    pub carried: std::collections::BTreeSet<String>,
    /// Rounds by `InventoryID`.
    pub ammo: BTreeMap<String, WeaponStock>,
    /// The weapon in hand, by lower-case file name.
    pub current: Option<String>,
    /// Seconds until the running reload's `C_Reload`.
    pub reloading: Option<f32>,
}

impl PlayerInventory {
    /// The rounds `name` uses.
    pub fn stock(&self, name: &str) -> Option<WeaponStock> {
        self.ammo.get(&self.defs.get(name)?.inventory_id).copied()
    }

    /// The weapon in hand has a round ready.
    pub fn can_fire(&self) -> bool {
        let stock = self.current.as_ref().and_then(|name| self.stock(name));
        self.reloading.is_none() && stock.is_some_and(|stock| stock.infinite || stock.clip > 0)
    }

    /// `WeaponID` of the weapon in hand.
    pub fn weapon_id(&self) -> Option<&str> {
        Some(&self.defs.get(self.current.as_ref()?)?.id)
    }

    /// The HUD's `(clip, pocket)` texts, as `0x4afe14` writes them: with infinite ammo, the
    /// `unlimited` text goes in the pocket slot and the clip slot stays empty.
    pub fn ammo_text(&self, unlimited: &str) -> Option<(String, String)> {
        let stock = self.stock(self.current.as_ref()?)?;
        Some(if stock.infinite {
            (String::new(), unlimited.to_string())
        } else {
            (stock.clip.to_string(), stock.pocket.to_string())
        })
    }

    /// Painkillers to show. The house gives one with `c_pickupweapon` and retail shows none, so
    /// this is the pocket and not the clip.
    pub fn painkillers(&self) -> u32 {
        self.stock("painkiller").map_or(0, |stock| stock.pocket)
    }

    /// `C_RemoveAllWeapons(name)`: strips the player and gives `name` with unlimited ammo.
    pub fn remove_all(&mut self, name: &str) {
        self.carried.clear();
        self.ammo.clear();
        self.current = None;
        self.reloading = None;
        if self
            .defs
            .get(name)
            .is_some_and(|def| def.id != "WEAPONID_EMPTY")
        {
            self.pickup_weapon(name);
            self.set_infinite(name, true);
        }
    }

    /// `C_PickupWeapon(name)`: a full clip the first time. It adds no ammo otherwise: level items
    /// follow it with `C_PickupAmmo`.
    pub fn pickup_weapon(&mut self, name: &str) {
        let Some(def) = self.defs.get(name) else {
            return;
        };
        if !self.carried.insert(name.to_string()) {
            return;
        }
        self.ammo
            .entry(def.inventory_id.clone())
            .or_insert(WeaponStock {
                clip: def.clip_size,
                ..WeaponStock::default()
            });
        let rank = |id: &str| self.best_weapons.iter().position(|best| best == id);
        let stronger = match self.current.as_ref().and_then(|name| self.defs.get(name)) {
            None => def.id != "WEAPONID_PAINKILLER",
            Some(held) => match (rank(&def.id), rank(&held.id)) {
                (Some(new), Some(old)) => new < old,
                (Some(_), None) => true,
                _ => false,
            },
        };
        if stronger {
            self.current = Some(name.to_string());
        }
    }

    /// `C_PickupAmmo(name, amount)`: into the pocket, up to its size.
    pub fn pickup_ammo(&mut self, name: &str, amount: u32) {
        let Some(def) = self.defs.get(name) else {
            return;
        };
        if let Some(stock) = self.ammo.get_mut(&def.inventory_id) {
            stock.pocket = (stock.pocket + amount).min(def.pocket_size);
        }
    }

    /// `C_SetInfiniteAmmo(name, on)`.
    pub fn set_infinite(&mut self, name: &str, on: bool) {
        let Some(def) = self.defs.get(name) else {
            return;
        };
        if let Some(stock) = self.ammo.get_mut(&def.inventory_id) {
            stock.infinite = on;
        }
    }

    /// Spends a round of the weapon in hand. An empty clip starts a reload instead.
    pub fn fire(&mut self) -> bool {
        if self.reloading.is_some() {
            return false;
        }
        let Some(def) = self.current.as_ref().and_then(|name| self.defs.get(name)) else {
            return false;
        };
        let reload_seconds = def.reload_seconds;
        let Some(stock) = self.ammo.get_mut(&def.inventory_id) else {
            return false;
        };
        if stock.infinite {
            return true;
        }
        if stock.clip > 0 {
            stock.clip -= 1;
            return true;
        }
        if stock.pocket > 0 {
            self.reloading = Some(reload_seconds);
        } else {
            self.take_best_loaded();
        }
        false
    }

    /// `weaponpriority.txt`: out of ammo, the player takes the best weapon that can still fire.
    fn take_best_loaded(&mut self) {
        let loaded = |name: &&String| {
            let def = &self.defs[*name];
            let stock = self.ammo.get(&def.inventory_id);
            stock.is_some_and(|s| s.infinite || s.clip + s.pocket > 0)
        };
        let best = self.best_weapons.iter().find_map(|id| {
            let mut carried = self.carried.iter();
            carried
                .find(|name| self.defs[*name].id == *id)
                .filter(loaded)
        });
        if let Some(name) = best {
            self.current = Some(name.clone());
        }
    }

    /// Advances a running reload, which refills the clip at its `C_Reload` frame.
    pub fn step(&mut self, seconds: f32) {
        let Some(left) = self.reloading else {
            return;
        };
        if left > seconds {
            self.reloading = Some(left - seconds);
            return;
        }
        self.reloading = None;
        let Some(def) = self.current.as_ref().and_then(|name| self.defs.get(name)) else {
            return;
        };
        if let Some(stock) = self.ammo.get_mut(&def.inventory_id) {
            let room = def.clip_size - stock.clip;
            let rounds = match u32::try_from(def.reload_amount) {
                Ok(amount) => amount.min(room),
                Err(_) => room,
            };
            let rounds = rounds.min(stock.pocket);
            stock.pocket -= rounds;
            stock.clip += rounds;
            if stock.clip < def.clip_size && stock.pocket > 0 {
                self.reloading = Some(def.reload_seconds);
            }
        }
    }
}

/// A strip font as `P_Font::validate` reads it (`e2mfc.dll` `0x10001e60`).
#[derive(Debug, Clone)]
pub struct HudFont {
    /// Colour with alpha in the top byte.
    sheet: crate::scene::DecodedTexture,
    /// `(start, width)` per character.
    glyphs: Vec<(u32, u32)>,
    height: u32,
}

impl HudFont {
    /// Reads the ruler in the sheet's first row: one run of coloured pixels per character, in
    /// code order. A glyph is one wider than its run and the font two shorter than the sheet.
    pub fn new(sheet: crate::scene::DecodedTexture) -> Option<Self> {
        let width = sheet.width as usize;
        let ruler = sheet.pixels.get(..width)?;
        let mut glyphs = Vec::with_capacity(256);
        let mut x = 0;
        while glyphs.len() < 256 {
            while ruler.get(x)? & 0x00ff_ffff == 0 {
                x += 1;
            }
            let start = x;
            while ruler.get(x)? & 0x00ff_ffff != 0 {
                x += 1;
            }
            glyphs.push((start as u32, (x - start) as u32 + 1));
        }
        let height = sheet.height.checked_sub(2)?;
        Some(Self {
            sheet,
            glyphs,
            height,
        })
    }

    fn glyph(&self, code: char) -> (u32, u32) {
        self.glyphs[code as usize & 0xff]
    }

    /// Text size, as `P_Font::getViewportSize` (`0x10001dc0`): the glyph widths and no spacing.
    pub fn size(&self, text: &str) -> [f32; 2] {
        let width: u32 = text.chars().map(|code| self.glyph(code).1).sum();
        [width as f32, self.height as f32]
    }
}

/// Top-left corner of a `size` box whose `reference` point sits at `position`.
pub fn top_left(reference: ReferencePoint, position: [f32; 2], size: [f32; 2]) -> [f32; 2] {
    let [x, y] = position;
    let [w, h] = size;
    match reference {
        0 => [x, y - h],
        1 => [x, y - h / 2.0],
        2 => [x, y],
        3 => [x - w / 2.0, y],
        4 => [x - w, y],
        5 => [x - w, y - h / 2.0],
        6 => [x - w, y - h],
        7 => [x - w / 2.0, y - h],
        _ => [x - w / 2.0, y - h / 2.0],
    }
}

/// `source` at `alpha` (0..255) over `dst`. The top byte of a HUD pixel is its transparency: 0 is
/// opaque, as in the software frame, and 255 is `gpu::HUD_CLEAR`.
pub fn blend(dst: u32, source: u32, alpha: u32) -> u32 {
    let below = 255 - (dst >> 24);
    let kept = below * (255 - alpha);
    let total = alpha * 255 + kept;
    if total == 0 {
        return dst;
    }
    let channel = |shift: u32| {
        let front = (source >> shift) & 0xff;
        let back = (dst >> shift) & 0xff;
        ((front * alpha * 255 + back * kept + total / 2) / total) << shift
    };
    ((255 - total / 255) << 24) | channel(16) | channel(8) | channel(0)
}

/// A sprite's colour file with its alpha file in the top byte.
fn load_bitmap(
    game_dir: &std::path::Path,
    sprite: &HudSprite,
) -> Option<crate::scene::DecodedTexture> {
    let read = |file: &str| crate::read_game_bytes(game_dir, &format!(r"\data\hud\{file}"));
    let (width, height, colour) =
        crate::scene::decode_texture_blob(&read(&sprite.file)?).or_else(|| {
            let (w, h, grey) = crate::particles::decode_pcx_grey(&read(&sprite.file)?)?;
            let rgb = |g: &u8| u32::from(*g) * 0x0001_0101;
            Some((w, h, grey.iter().map(rgb).collect()))
        })?;
    let (_, _, alpha) = crate::particles::decode_pcx_grey(&read(&sprite.alpha_file)?)?;
    let pixels = colour
        .iter()
        .zip(&alpha)
        .map(|(rgb, a)| (u32::from(*a) << 24) | (rgb & 0x00ff_ffff))
        .collect();
    Some(crate::scene::DecodedTexture {
        path: sprite.file.clone(),
        flag: 0,
        width,
        height,
        pixels,
    })
}

/// `hud.txt` and the bitmaps and font it names.
#[derive(Debug, Clone)]
pub struct HudAssets {
    pub layout: HudLayout,
    pub font: HudFont,
    /// Colour with alpha in the top byte, by `Filename`.
    pub bitmaps: BTreeMap<String, crate::scene::DecodedTexture>,
}

impl HudAssets {
    pub fn load(game_dir: &std::path::Path) -> Option<Self> {
        let text = crate::read_game_bytes(game_dir, r"\data\hud\hud.txt")?;
        let layout = HudLayout::parse(&String::from_utf8_lossy(&text))?;
        let font = load_font(game_dir, r"\data\hud\bitmaps\Hud_Universal_Medium")?;
        let sprites = [
            &layout.health_background,
            &layout.health_pain,
            &layout.hourglass_background,
            &layout.hourglass,
            &layout.painkiller_icon,
        ];
        let bitmaps = sprites
            .into_iter()
            .chain(layout.weapons.values())
            .filter_map(|sprite| Some((sprite.file.clone(), load_bitmap(game_dir, sprite)?)))
            .collect();
        Some(Self {
            layout,
            font,
            bitmaps,
        })
    }
}

/// What the HUD shows this frame.
pub struct HudState<'a> {
    /// Normalized health, 0..1.
    pub health: f32,
    pub slow_motion: &'a SlowMotion,
    pub inventory: &'a PlayerInventory,
}

/// A frame to draw on, `width` x `height`, showing the 640x480 HUD space.
pub struct HudTarget<'a> {
    pub pixels: &'a mut [u32],
    pub width: usize,
    pub height: usize,
}

impl HudTarget<'_> {
    /// `sprite`, showing only its lower `visible` part (0..1), as the pain fill does.
    pub fn draw_sprite(
        &mut self,
        sprite: &HudSprite,
        bitmap: &crate::scene::DecodedTexture,
        visible: f32,
    ) {
        self.draw_tinted(sprite, bitmap, visible, [255, 255, 255, sprite.alpha]);
    }

    /// As [`Self::draw_sprite`], with `rgba` in place of the sprite's own alpha.
    fn draw_tinted(
        &mut self,
        sprite: &HudSprite,
        bitmap: &crate::scene::DecodedTexture,
        visible: f32,
        rgba: [u8; 4],
    ) {
        let size = [sprite.width, sprite.height];
        let [x, y] = top_left(sprite.reference, sprite.position, size);
        let hidden = 1.0 - visible;
        self.blit(
            [
                x,
                y + sprite.height * hidden,
                x + sprite.width,
                y + sprite.height,
            ],
            [0.0, hidden, 1.0, 1.0],
            bitmap,
            rgba,
        );
    }

    /// The whole HUD, as `0x4aee20` updates it.
    pub fn draw_hud(&mut self, assets: &HudAssets, state: &HudState) {
        let layout = &assets.layout;
        let bitmap = |sprite: &HudSprite| assets.bitmaps.get(&sprite.file);
        if let Some(figure) = bitmap(&layout.health_background) {
            self.draw_sprite(&layout.health_background, figure, 1.0);
        }
        if let Some(pain) = bitmap(&layout.health_pain) {
            self.draw_sprite(
                &layout.health_pain,
                pain,
                1.0 - state.health.clamp(0.0, 1.0),
            );
        }
        let color = layout.hourglass_colors.pick(state.slow_motion);
        if let Some(glass) = bitmap(&layout.hourglass_background) {
            self.draw_tinted(&layout.hourglass_background, glass, 1.0, color);
        }
        if let Some(glass) = bitmap(&layout.hourglass) {
            self.draw_tinted(&layout.hourglass, glass, state.slow_motion.level, color);
        }
        let painkillers = state.inventory.painkillers();
        if painkillers > 0 {
            if let Some(cross) = bitmap(&layout.painkiller_icon) {
                self.draw_sprite(&layout.painkiller_icon, cross, 1.0);
            }
            let count = painkillers.to_string();
            self.draw_text(&assets.font, layout.painkiller_count, &count, [255; 4]);
        }
        let Some(id) = state.inventory.weapon_id() else {
            return;
        };
        if let Some(sprite) = layout.weapons.get(id) {
            if let Some(image) = bitmap(sprite) {
                self.draw_sprite(sprite, image, 1.0);
            }
        }
        let unlimited = layout.unlimited_text.get(id).map_or("", String::as_str);
        if let Some((clip, pocket)) = state.inventory.ammo_text(unlimited) {
            self.draw_text(&assets.font, layout.ammo_in_clip, &clip, [255; 4]);
            self.draw_text(&assets.font, layout.ammo_in_pocket, &pocket, [255; 4]);
        }
    }

    /// The lines of a print block.
    pub fn draw_prints(&mut self, font: &HudFont, style: &HudPrintStyle, log: &HudPrintLog) {
        let [r, g, b, _] = style.color;
        for (text, position, alpha) in log.placed(style) {
            let anchor = HudText {
                reference: style.anchor.reference,
                position,
            };
            self.draw_text(font, anchor, text, [r, g, b, alpha]);
        }
    }

    /// The aiming dot of the weapon in hand, if it has one.
    pub fn draw_crosshair(&mut self, inventory: &PlayerInventory) {
        let held = inventory
            .current
            .as_ref()
            .and_then(|name| inventory.defs.get(name));
        if !held.is_some_and(|def| def.shows_dot) {
            return;
        }
        // Retail: 3 pixels of a 1920x1080 frame each way, a 640x360 unit.
        let size = |extent: usize, units: f32| (extent as f32 / units).round().max(1.0) as usize;
        let (width, height) = (size(self.width, 640.0), size(self.height, 360.0));
        let (left, top) = (self.width / 2 - width / 2, self.height / 2 - height / 2);
        for row in top..top + height {
            let start = row * self.width + left;
            self.pixels[start..start + width].fill(0x00ff_ffff);
        }
    }

    /// `text` anchored at `anchor`, tinted by `rgba`.
    pub fn draw_text(&mut self, font: &HudFont, anchor: HudText, text: &str, rgba: [u8; 4]) {
        let size = font.size(text);
        let [mut x, y] = top_left(anchor.reference, anchor.position, size);
        let (sheet_w, sheet_h) = (font.sheet.width as f32, font.sheet.height as f32);
        for code in text.chars() {
            let (start, width) = font.glyph(code);
            let (start, width) = (start as f32, width as f32);
            self.blit(
                [x, y, x + width, y + size[1]],
                [
                    start / sheet_w,
                    // Half a texel in, so the filter does not reach the ruler above.
                    1.5 / sheet_h,
                    (start + width) / sheet_w,
                    (1.0 + size[1]) / sheet_h,
                ],
                &font.sheet,
                rgba,
            );
            x += width;
        }
    }

    /// `uv` (left, top, right, bottom) of `bitmap` into `area`, given in HUD space.
    fn blit(
        &mut self,
        area: [f32; 4],
        uv: [f32; 4],
        bitmap: &crate::scene::DecodedTexture,
        rgba: [u8; 4],
    ) {
        let (scale_x, scale_y) = (self.width as f32 / 640.0, self.height as f32 / 480.0);
        let first = |edge: f32, scale: f32| (edge * scale - 0.5).ceil().max(0.0) as usize;
        let columns = first(area[0], scale_x)..first(area[2], scale_x).min(self.width);
        let rows = first(area[1], scale_y)..first(area[3], scale_y).min(self.height);
        let tint = rgba.map(u32::from);
        for row in rows {
            let along_y = ((row as f32 + 0.5) / scale_y - area[1]) / (area[3] - area[1]);
            let v = uv[1] + (uv[3] - uv[1]) * along_y;
            for column in columns.clone() {
                let along_x = ((column as f32 + 0.5) / scale_x - area[0]) / (area[2] - area[0]);
                let u = uv[0] + (uv[2] - uv[0]) * along_x;
                let texel = crate::render3d::sample_texture(bitmap, u, v);
                let channel =
                    |shift: u32, k: usize| (((texel >> shift) & 0xff) * tint[k] / 255) << shift;
                let source = channel(16, 0) | channel(8, 1) | channel(0, 2);
                let alpha = (texel >> 24) * tint[3] / 255;
                let pixel = &mut self.pixels[row * self.width + column];
                *pixel = blend(*pixel, source, alpha);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::DecodedTexture;

    fn sheet(width: u32, height: u32, pixels: Vec<u32>) -> DecodedTexture {
        DecodedTexture {
            path: String::new(),
            flag: 0,
            width,
            height,
            pixels,
        }
    }

    /// 256 one-pixel ruler ticks two apart, except `A`, three wide, at its slot.
    fn ruler_font() -> HudFont {
        let (width, height) = (2 + 256 * 2 + 4, 6u32);
        let mut pixels = vec![0u32; (width * height) as usize];
        let mut x = 2u32;
        for code in 0..256 {
            let run = if code == 65 { 3 } else { 1 };
            for k in 0..run {
                pixels[(x + k) as usize] = 0xffff_ffff;
            }
            x += run + 1;
        }
        HudFont::new(sheet(width, height, pixels)).expect("font")
    }

    fn inventory() -> PlayerInventory {
        let def = |id: &str, inventory: &str, clip_size, pocket_size| WeaponDef {
            id: id.into(),
            inventory_id: inventory.into(),
            clip_size,
            pocket_size,
            reload_seconds: 1.3,
            reload_amount: -1,
            shows_dot: true,
            ..Default::default()
        };
        let mut defs = BTreeMap::new();
        for (name, weapon) in [
            ("beretta", def("WEAPONID_BERETTA", "INV_BERETTA", 18, 162)),
            (
                "berettadual",
                def("WEAPONID_BERETTADUAL", "INV_BERETTA", 18, 162),
            ),
            ("painkiller", def("WEAPONID_PAINKILLER", "INV_PAIN", 1, 8)),
            ("ingram", def("WEAPONID_INGRAM", "INV_INGRAM", 30, 180)),
            ("leadpipe", def("WEAPONID_LEADPIPE", "INV_PIPE", 1, 1)),
            ("molotov", def("WEAPONID_MOLOTOV", "INV_MOLOTOV", 1, 10)),
        ] {
            defs.insert(name.to_string(), weapon);
        }
        PlayerInventory {
            defs,
            best_weapons: [
                "WEAPONID_INGRAM",
                "WEAPONID_BERETTADUAL",
                "WEAPONID_BERETTA",
                "WEAPONID_LEADPIPE",
            ]
            .map(String::from)
            .to_vec(),
            ..PlayerInventory::default()
        }
    }

    fn stock(clip: u32, pocket: u32, infinite: bool) -> Option<WeaponStock> {
        Some(WeaponStock {
            clip,
            pocket,
            infinite,
        })
    }

    #[test]
    fn a_death_clip_names_the_blood_it_leaves_whatever_the_verbs_case() {
        let text = "\t[Animation] Index = CHARANIM_RANDOMDEATH1; Filename = a.kf2;\n\t\t[Message] Frame = 38; String = \"this->P_CreateProjectileToBone( blood_dummy, 2, torso );\";\n\t\t[Message] Frame = 90; String = \"this->P_CreateprojectileToBone( dummy_smokingbody, 1, shoulders );\";\n\t[Animation] Index = CHARANIM_RANDOMDEATH2; Filename = b.kf2;\n";
        assert_eq!(
            parse_clip_projectiles(text, "CHARANIM_RANDOMDEATH1"),
            [
                (38, "blood_dummy".to_string(), 2),
                (90, "dummy_smokingbody".to_string(), 1)
            ]
        );
    }

    #[test]
    fn slow_motion_reads_its_message_lists() {
        let config = "[slowmotion]\n{\n\t[general]\n\tSlowMotionTime = 15.0;\n\tRegenerationTime = 100.0;\n\tThreshold = 0.1%;\n\tHeartbeatSoundMessage = \"beat\";\n\tHeartbeatSoundInterval = 1.25;\n\n\t[StartMessages]\n\t{\n\t\t[Message] String = \"maxpayne_gamemode->gm_changecontrolledgamespeed( 0.2, 0.3 );\";\n\t\t[Message] String = \"maxpayne_gamemode->A_PlaySound ( characters, slow_motion );\";\n\t}\n\n\t[EndMessages]\n\t{\n\t\t[Message] String = \"end;\";\n//\t\t[Message] String = \"commented;\";\n\t}\n\n\t[CantSwitchOnMessages]\n\t{\n\t\t[Message] String = \"refused;\";\n\t}\n}\n";
        let mut slow =
            SlowMotion::parse(config, "#define BONUS_SLOWMOTION 0.12\n").expect("config");
        assert_eq!(slow.start_messages.len(), 2);
        assert!(slow.start_messages[0].contains("gm_changecontrolledgamespeed( 0.2, 0.3 )"));
        assert_eq!(slow.end_messages, ["end;"]);
        // Starting, ending, refused, and nothing to say.
        slow.active = true;
        assert_eq!(slow.transition(false, true).len(), 2);
        assert!(slow.transition(true, true).is_empty());
        slow.active = false;
        assert_eq!(slow.transition(true, false), ["end;"]);
        assert_eq!(slow.transition(false, true), ["refused;"]);
        assert!(slow.transition(false, false).is_empty());
    }

    #[test]
    fn a_weapon_file_gives_its_id_and_sizes_through_shooting_h() {
        let def = WeaponDef::parse(
            "#define\tBERETTA_CLIPSIZE\t\t18\n#define BERETTA_POCKETSIZE 162\n",
            "[Properties]\nInventoryID = INVENTORYID_BERETTA;\nWeaponID \t= WEAPONID_BERETTA;\nClipsize \t= BERETTA_CLIPSIZE;\nPocketsize\t= BERETTA_POCKETSIZE;\nReloadAmount = -1;\n[Crosshair]\n{\n\t[Geometry]\n\tExportData = ..\\crosshairs\\dot.kf2;\n}\n",
        );
        assert_eq!(
            def,
            Some(WeaponDef {
                id: "WEAPONID_BERETTA".into(),
                inventory_id: "INVENTORYID_BERETTA".into(),
                clip_size: 18,
                pocket_size: 162,
                reload_seconds: 0.0,
                reload_amount: -1,
                shows_dot: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn a_weapon_file_gives_its_shot_and_reload_cues() {
        let def = WeaponDef::parse(
            "#define BERETTA_SHOOTINGFREQUENCY 4\n#define BERETTA_SLOWMOTIONMULTIPLIER 0.25\n",
            "[Animationset]\n{\n\t[Animation] Index = WEAPONANIM_DEFAULT; Filename = d.kf2;\n\t[Animation] Index = WEAPONANIM_SHOOT; Filename = s.kf2;\n\t[Properties]\n\t{\n\t\t[Message] Frame = 0; String = \"this->A_Play3DSound ( weapons, shoot_beretta , \\\"\\\");\";\n\t\t[Message] Frame = 0; String = \"this->PS_StartEffect ( Muzzle_Beretta, \\\"\\\" );\";\n\t}\n\t[Animation] Index = WEAPONANIM_RELOAD; Filename = d.kf2;\n\t[Properties]\n\t{\n\t\t[Message] Frame = 0; String = \"this->A_Play3DSound ( weapons, reload_beretta, \\\"\\\");\";\n\t}\n}\n[Attributes]\nInventoryID = I;\nWeaponID = W;\nClipsize = 1;\nPocketsize = 2;\nReloadAmount = -1;\nDefaultShootingFrequency = BERETTA_SHOOTINGFREQUENCY;\nShootingFrequencyMultiplier = BERETTA_SLOWMOTIONMULTIPLIER;\n",
        )
        .expect("weapon");
        assert_eq!(def.shots_per_second, 4.0);
        assert_eq!(def.slow_motion_rate, 0.25);
        assert_eq!(def.fire_sound.as_deref(), Some("shoot_beretta"));
        assert_eq!(def.muzzle.as_deref(), Some("Muzzle_Beretta"));
        assert_eq!(def.reload_sound.as_deref(), Some("reload_beretta"));
    }

    #[test]
    fn shoot_animations_map_weapons_and_follow_reverts() {
        let text = "\t[Revert] From = CHARANIM_SHOOTMP5; To = CHARANIM_SHOOTINGRAM;\n\t[Animation] Index = CHARANIM_SHOOTBERETTA; Filename = anim\\shootberetta.kf2;\n\t[Animation] Index = CHARANIM_SHOOTINGRAM; Filename = anim\\shootingram.kf2;\n\t[Animation] Index = CHARANIM_WALK; Filename = anim\\walk.kf2;\n";
        let shots = parse_shoot_animations(text);
        assert_eq!(shots["beretta"], r"anim\shootberetta.kf2");
        assert_eq!(shots["mp5"], r"anim\shootingram.kf2");
        assert!(!shots.contains_key("walk"));
    }

    #[test]
    fn reload_sound_frames_come_from_the_weapons_ws_animate() {
        let text = "\t[Animation] Index = CHARANIM_RELOADBERETTA; Filename = r.kf2;\n\t{\n\t\t[Message] Frame = 0; String = \"RightHandWeapon->WS_Hide( FALSE );\";\n\t\t[Message] Frame = 10; String = \"RightHandWeapon->WS_Animate(\" WEAPONANIM_RELOAD \");\";\n\t}\n";
        assert_eq!(parse_reload_sound_frames(text)["beretta"], 10);
    }

    #[test]
    fn a_melee_projectile_reaches_its_flight_and_blast() {
        let text = "[Animationset]\n{\n[Message] Frame = 0; String = \"this->P_Explosion( 0.80, 0.801, 9, 8.99, FALSE );\";\n}\n[Attributes]\nSpeed \t\t= 20;\nDelayedExplosion \t\t= TRUE;\nExplosionDelay\t\t\t= 0.05;\n";
        let reach = parse_melee_reach(text).expect("melee");
        assert!((reach - 1.8).abs() < 1e-5, "{reach}");
        // A bullet does not explode.
        assert_eq!(parse_melee_reach("Speed = 400;\n"), None);
    }

    #[test]
    fn bone_projectiles_list_each_blow_of_the_clip() {
        let text = "[Animation] Index = CHARANIM_SHOOTLEADPIPE; Filename = a.kf2;\n{\n[Message] Frame = 0; String = \"this->C_AnimateTexture( 1 );\";\n[Message] Frame = 7; String = \"this->p_createprojectiletobone( bullet_leadpipe, 1, neck );\";\n[Message] Frame = 31; String = \"this->p_createprojectiletobone( bullet_leadpipe, 1, neck );\";\n}\n[Animation] Index = CHARANIM_SHOOTLOWLEADPIPE; Filename = b.kf2;\n[Message] Frame = 9; String = \"this->p_createprojectiletobone( other, 1, neck );\";\n";
        assert_eq!(
            parse_bone_projectiles(text, "CHARANIM_SHOOTLEADPIPE"),
            [
                (7, "bullet_leadpipe".to_string()),
                (31, "bullet_leadpipe".to_string())
            ]
        );
    }

    #[test]
    fn clip_sounds_list_their_frames_catalogs_and_cues() {
        let text = "[Animation] Index = CHARANIM_RANDOMDEATH1; Filename = anim\\death_minimum.kf2;\n{\n[Message] Frame = 0; String = \"this->A_Play3DSound ( enemy, mobster_death, head );\";\n[Message] Frame = 40; String = \"this->A_Play3DSound ( characters, death_medium, pelvis );\";\n}\n[Animation] Index = CHARANIM_RANDOMDEATH2; Filename = b.kf2;\n[Message] Frame = 1; String = \"this->A_Play3DSound ( other, cue, head );\";\n";
        assert_eq!(
            parse_clip_sounds(text, "CHARANIM_RANDOMDEATH1"),
            [
                (0, "enemy".to_string(), "mobster_death".to_string()),
                (40, "characters".to_string(), "death_medium".to_string()),
            ]
        );
    }

    #[test]
    fn a_weapon_file_says_whether_it_is_two_handed() {
        let file = |flag: &str| {
            format!("[Attributes]\nTwoHandedAnimation \t= {flag};\nInventoryID = I;\nWeaponID = W;\nClipsize = 1;\nPocketsize = 2;\nReloadAmount = -1;\n")
        };
        assert!(
            WeaponDef::parse("", &file("YES"))
                .expect("weapon")
                .two_handed
        );
        assert!(
            !WeaponDef::parse("", &file("NO"))
                .expect("weapon")
                .two_handed
        );
    }

    #[test]
    fn a_shot_lists_its_projectiles_but_only_from_the_shoot_animation() {
        let text = "[Animation] Index = WEAPONANIM_SHOOT; Filename = s.kf2;\n[Message] Frame = 0; String = \"this->P_CreateProjectile ( bullet_shotgun_straight, 1 );\";\n[Message] Frame = 0; String = \"this->P_CreateProjectile ( bullet_pumpshotgun, 2 );\";\n[Animation] Index = WEAPONANIM_SHOOT_NOEFFECTS; Filename = s.kf2;\n[Message] Frame = 0; String = \"this->P_CreateProjectile ( bullet_other, 1 );\";\n";
        assert_eq!(
            parse_shot_projectiles(text, "WEAPONANIM_SHOOT"),
            [
                ("bullet_shotgun_straight".to_string(), 1),
                ("bullet_pumpshotgun".to_string(), 2),
            ]
        );
    }

    #[test]
    fn a_weapon_file_says_which_bones_and_guns_it_uses() {
        let def = WeaponDef::parse(
            "",
            "[Attributes]\nTwoHandedBones \t= YES;\nUseMultipleGunBones = YES;\nInventoryID = I;\nWeaponID = W;\nClipsize = 1;\nPocketsize = 2;\nReloadAmount = -1;\n",
        )
        .expect("weapon");
        assert!(def.dual && def.two_handed_bones && !def.two_handed);
    }

    #[test]
    fn only_a_loaded_idle_weapon_can_fire() {
        let mut inventory = PlayerInventory::default();
        let beretta = WeaponDef {
            inventory_id: "INV".into(),
            ..Default::default()
        };
        inventory.defs.insert("beretta".to_string(), beretta);
        inventory.current = Some("beretta".to_string());
        assert!(!inventory.can_fire());
        let stock = WeaponStock {
            clip: 1,
            ..Default::default()
        };
        inventory.ammo.insert("INV".to_string(), stock);
        assert!(inventory.can_fire());
        inventory.reloading = Some(1.0);
        assert!(!inventory.can_fire());
    }

    #[test]
    fn a_clip_lists_its_rounds_with_and_without_effects() {
        let text = "[Animation] Index = CHARANIM_SHOOTINGRAM; Filename = a.kf2;\n[Message] Frame = 0; String = \"RightHandWeapon->A_Play3DSound ( weapons, shoot_ingram );\";\n[Message] Frame = 2; String = \"RightHandWeapon->WS_AnimateShooting(\" WEAPONANIM_SHOOT \",\" WEAPONANIM_SHOOTEMPTY \");\";\n[Message] Frame = 5; String = \"RightHandWeapon->WS_AnimateShooting(\" WEAPONANIM_SHOOT_NOEFFECTS \",\" WEAPONANIM_SHOOTEMPTY \");\";\n[Animation] Index = CHARANIM_SHOOTLOWINGRAM; Filename = b.kf2;\n";
        let right = |at, quiet| ShotRound {
            at,
            quiet,
            left: false,
        };
        assert_eq!(
            parse_rounds(text, "CHARANIM_SHOOTINGRAM"),
            [right(2.0, false), right(5.0, true)]
        );
        let dual = "[Animation] Index = CHARANIM_SHOOTBERETTADUAL; Filename = a.kf2;\n[Message] Frame = 3; String = \"LeftHandWeapon->WS_AnimateShooting(\" WEAPONANIM_SHOOT \",\" WEAPONANIM_SHOOTEMPTY \");\";\n";
        assert!(parse_rounds(dual, "CHARANIM_SHOOTBERETTADUAL")[0].left);
    }

    #[test]
    fn a_quiet_round_uses_its_own_projectiles_when_it_has_them() {
        let shot = |damage| ProjectileDef {
            count: 1,
            damage,
            ..Default::default()
        };
        let mut def = WeaponDef {
            projectiles: vec![shot(5)],
            quiet_projectiles: vec![shot(6)],
            ..Default::default()
        };
        let damage = |def: &WeaponDef, quiet| def.round_projectiles(quiet)[0].damage;
        assert_eq!([damage(&def, false), damage(&def, true)], [5, 6]);
        def.quiet_projectiles.clear();
        assert_eq!(damage(&def, true), 5);
    }

    #[test]
    fn a_character_hit_names_its_splatter_and_that_file_says_how_it_flies() {
        let bullet = "[Decal]\nString = \"this->D_CreateDecal( bullethit_generic );\";\n\n[CharacterHit]\nString = \"this->P_CreateProjectile( blood_bullet_small, 1 );\";\n[Attributes]\n";
        assert_eq!(
            parse_character_hit(bullet),
            Some(("blood_bullet_small".to_string(), 1))
        );
        assert_eq!(parse_character_hit("[Attributes]\n"), None);
        let blood = "[Decal]\nString =\"this->d_createdecal( blood_small );\";\n[Attributes]\nAccuracy \t= 500;\nSpeed \t\t= 8 ;\nDamage \t\t= 0;\nGravityMultiplier\t\t= 1.0;\n";
        assert_eq!(
            SplatterDef::parse(blood, 1),
            Some(SplatterDef {
                count: 1,
                speed: 8.0,
                accuracy: 500.0,
                gravity: 1.0,
                decal: Some("blood_small".to_string()),
            })
        );
    }

    #[test]
    fn a_projectile_file_gives_damage_spread_and_decal() {
        let text = "[Decal]\nString = \"this->D_CreateDecal( bullethit_shotgun );\";\n[Attributes]\nAccuracy \t= PUMPSHOTGUN_BULLETSPREAD ;\nDamage \t\t= PUMPSHOTGUN_BULLETDAMAGE ;\n";
        let defines = "#define PUMPSHOTGUN_BULLETDAMAGE 5\n#define PUMPSHOTGUN_BULLETSPREAD 150\n";
        assert_eq!(
            ProjectileDef::parse(defines, text, 2),
            Some(ProjectileDef {
                count: 2,
                damage: 5,
                accuracy: 150.0,
                decal: Some("bullethit_shotgun".to_string()),
                character_hit: None,
            })
        );
        assert_eq!(
            ProjectileDef::parse("", "[Attributes]\nDamage = 0;\n", 1),
            None
        );
    }

    #[test]
    fn a_weapon_file_names_its_nearest_model() {
        let def = WeaponDef::parse(
            "",
            "[LOD] { [Geometry] ExportData = ..\\beretta\\beretta_Lod0.kf2; Distance = 5 ; }\n[LOD] { [Geometry] ExportData = ..\\beretta\\beretta_Lod1.kf2; Distance = 9 ;}\n[Attributes]\nInventoryID = I;\nWeaponID = W;\nClipsize = 1;\nPocketsize = 2;\nReloadAmount = -1;\n[Crosshair]\n{\n[Geometry]\nExportData = ..\\crosshairs\\dot.kf2;\n}\n",
        )
        .expect("weapon");
        assert_eq!(def.model.as_deref(), Some(r"beretta\beretta_Lod0.kf2"));
        assert!(def.shows_dot);
    }

    #[test]
    fn best_weapons_keep_their_order_and_stop_at_their_block() {
        let text = "[BestWeapons]\n{\n\t[weapon] ID = WEAPONID_MP5;\n\t[weapon] ID = WEAPONID_BERETTA;\n}\n[CycleWeapons]\n{\n\t[weapon] ID = WEAPONID_M79;\n}\n";
        assert_eq!(
            parse_best_weapons(text),
            ["WEAPONID_MP5", "WEAPONID_BERETTA"]
        );
    }

    #[test]
    fn a_reload_animation_names_its_file_and_c_reload_frame() {
        let text = "\t[Revert] From = CHARANIM_RELOADEMPTY; To = CHARANIM_RELOADGRENADE;\n\t[Animation] Index = CHARANIM_RELOADBERETTA; Filename = anim\\reloadberetta.kf2;\n\t[Properties]\n\t{\n\t\t[Message] Frame = 10; String = \"RightHandWeapon->WS_Animate(\" WEAPONANIM_RELOAD \");\";\n\t\t[Message] Frame = 39; String = \"this->C_Reload();\";\n\t}\n\t[Animation] Index = CHARANIM_JUMP; Filename = anim\\jump.kf2;\n\t[Properties]\n\t{\n\t\t[Message] Frame = 8; String = \"this->C_Reload();\";\n\t}\n";
        let reloads = parse_reload_animations(text);
        assert_eq!(reloads.len(), 1);
        assert_eq!(
            reloads["beretta"],
            (r"anim\reloadberetta.kf2".to_string(), 39)
        );
    }

    #[test]
    fn the_house_script_gives_a_clip_of_18_and_99_in_the_pocket() {
        // `Part0_Level1`: the order of these messages is the level's.
        let mut player = inventory();
        player.remove_all("beretta");
        player.set_infinite("beretta", false);
        player.pickup_weapon("painkiller");
        player.pickup_weapon("beretta");
        player.pickup_ammo("beretta", 99);
        assert_eq!(player.stock("beretta"), stock(18, 99, false));
        assert_eq!(player.current.as_deref(), Some("beretta"));
    }

    #[test]
    fn removing_all_weapons_gives_the_named_one_unlimited_ammo() {
        let mut player = inventory();
        player.pickup_weapon("painkiller");
        player.remove_all("beretta");
        assert_eq!(player.carried.len(), 1);
        assert!(player.stock("beretta").is_some_and(|s| s.infinite));
        assert_eq!(player.current.as_deref(), Some("beretta"));
    }

    #[test]
    fn removing_all_weapons_for_empty_leaves_nothing() {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.remove_all("empty");
        assert!(player.carried.is_empty());
        assert_eq!(player.current, None);
    }

    #[test]
    fn a_painkiller_is_not_taken_in_hand() {
        let mut player = inventory();
        player.pickup_weapon("painkiller");
        assert_eq!(player.current, None);
    }

    #[test]
    fn a_stronger_weapon_is_taken_in_hand_on_pickup() {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.pickup_weapon("ingram");
        assert_eq!(player.current.as_deref(), Some("ingram"));
    }

    #[test]
    fn a_weaker_weapon_stays_in_the_inventory_on_pickup() {
        let mut player = inventory();
        player.pickup_weapon("ingram");
        player.pickup_weapon("beretta");
        assert_eq!(player.current.as_deref(), Some("ingram"));
        assert!(player.carried.contains("beretta"));
    }

    #[test]
    fn a_weapon_outside_the_best_list_is_not_switched_to() {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.pickup_weapon("molotov");
        assert_eq!(player.current.as_deref(), Some("beretta"));
    }

    #[test]
    fn the_second_beretta_switches_to_dual_with_the_same_rounds() {
        // `level_items/beretta.txt`: both pickups, then `C_PickupAmmo(beretta, 10)`.
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.fire();
        player.pickup_weapon("berettadual");
        player.pickup_ammo("beretta", 10);
        assert_eq!(player.current.as_deref(), Some("berettadual"));
        assert_eq!(player.stock("berettadual"), stock(17, 10, false));
        assert_eq!(player.stock("beretta"), stock(17, 10, false));
    }

    #[test]
    fn ammo_stops_at_the_pocket_size() {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.pickup_ammo("beretta", 500);
        assert_eq!(player.stock("beretta"), stock(18, 162, false));
    }

    fn emptied_beretta(pocket: u32) -> PlayerInventory {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.pickup_ammo("beretta", pocket);
        for _ in 0..18 {
            assert!(player.fire());
        }
        player
    }

    #[test]
    fn an_empty_clip_reloads_over_time_and_cannot_fire_meanwhile() {
        let mut player = emptied_beretta(20);
        assert!(!player.fire(), "the empty clip starts the reload");
        player.step(1.0);
        assert!(!player.fire());
        assert_eq!(player.stock("beretta"), stock(0, 20, false));
        player.step(0.4);
        assert_eq!(player.stock("beretta"), stock(18, 2, false));
        assert!(player.fire());
    }

    #[test]
    fn a_reload_of_one_round_repeats_until_the_clip_is_full() {
        let mut player = emptied_beretta(20);
        player.defs.get_mut("beretta").expect("def").reload_amount = 1;
        player.fire();
        player.step(1.4);
        assert_eq!(player.stock("beretta"), stock(1, 19, false));
        assert!(player.reloading.is_some());
        for _ in 0..17 {
            player.step(1.4);
        }
        assert_eq!(player.stock("beretta"), stock(18, 2, false));
        assert_eq!(player.reloading, None);
    }

    #[test]
    fn a_reverted_reload_uses_the_animation_it_points_to() {
        let text = "\t[Animation] Index = CHARANIM_RELOADMP5; Filename = anim\\reloadmp5.kf2;\n\t\t[Message] Frame = 39; String = \"this->C_Reload();\";\n\t[Revert] From = CHARANIM_RELOADSNIPER; To = CHARANIM_RELOADMP5;\n";
        let reloads = parse_reload_animations(text);
        assert_eq!(reloads["sniper"], reloads["mp5"]);
    }

    #[test]
    fn with_no_rounds_left_the_best_loaded_weapon_is_taken_in_hand() {
        let mut player = inventory();
        player.pickup_weapon("leadpipe");
        player.set_infinite("leadpipe", true);
        player.pickup_weapon("beretta");
        for _ in 0..18 {
            player.fire();
        }
        assert!(!player.fire());
        assert_eq!(player.reloading, None);
        assert_eq!(player.current.as_deref(), Some("leadpipe"));
    }

    #[test]
    fn nothing_in_hand_does_not_fire() {
        assert!(!PlayerInventory::default().fire());
    }

    #[test]
    fn infinite_ammo_never_runs_out() {
        let mut player = inventory();
        player.remove_all("beretta");
        for _ in 0..100 {
            assert!(player.fire());
        }
    }

    fn gauge() -> SlowMotion {
        SlowMotion {
            level: 1.0,
            active: false,
            seconds: 15.0,
            threshold: 0.001,
            heartbeat_message: "beat".into(),
            heartbeat_interval: 1.25,
            heartbeat_in: 0.0,
            regeneration_seconds: 100.0,
            kill_bonus: 0.12,
            start_messages: Vec::new(),
            end_messages: Vec::new(),
            refused_messages: Vec::new(),
        }
    }

    #[test]
    fn globalconfig_gives_the_gauges_length_and_threshold() {
        let text = "[slowmotion]\n{\n\t[general]\n\tSlowMotionTime\t\t\t= 15.0;\t\t\t// length\n\tThreshold\t\t\t\t= 0.1%;\t\t\t// must be exceeded\n\tHeartbeatSoundMessage\t= \"beat\";\n\tHeartbeatSoundInterval\t= 1.25;\t\t\t// interval\n\tRegenerationTime\t= 100.0;\t// takes this many seconds\n}\n";
        let characters = "#define\tBONUS_SLOWMOTION\t\t\t\t0.12\n";
        assert_eq!(SlowMotion::parse(text, characters), Some(gauge()));
    }

    #[test]
    fn the_heartbeat_starts_with_slow_motion_and_repeats_at_its_interval() {
        let mut slow = gauge();
        assert_eq!(slow.heartbeat(0.1), None, "not while inactive");
        slow.step(true, 0.1);
        assert_eq!(slow.heartbeat(0.1), Some("beat"));
        assert_eq!(slow.heartbeat(1.0), None);
        assert_eq!(slow.heartbeat(0.3), Some("beat"));
    }

    #[test]
    fn the_heartbeat_restarts_with_the_next_slow_motion() {
        let mut slow = gauge();
        slow.step(true, 0.1);
        assert!(slow.heartbeat(0.1).is_some());
        slow.step(false, 0.1);
        assert_eq!(slow.heartbeat(0.1), None);
        slow.step(true, 0.1);
        assert!(slow.heartbeat(0.1).is_some());
    }

    #[test]
    fn the_gauge_refills_at_its_regeneration_rate_while_inactive() {
        let mut slow = gauge();
        slow.level = 0.5;
        slow.step(false, 10.0);
        assert!((slow.level - 0.6).abs() < 1e-5, "{}", slow.level);
        slow.level = 0.999;
        slow.step(false, 10.0);
        assert_eq!(slow.level, 1.0);
    }

    #[test]
    fn a_kill_fills_the_gauge_by_its_bonus_up_to_full() {
        let mut slow = gauge();
        slow.level = 0.5;
        slow.reward_kill();
        assert!((slow.level - 0.62).abs() < 1e-5);
        slow.level = 0.95;
        slow.reward_kill();
        assert_eq!(slow.level, 1.0);
    }

    #[test]
    fn slow_motion_drains_the_gauge_only_while_active() {
        let mut slow = gauge();
        slow.step(false, 3.0);
        assert_eq!(slow.level, 1.0);
        slow.step(true, 3.0);
        assert!(slow.active);
        assert!((slow.level - 0.8).abs() < 1e-5);
        slow.step(false, 1.0);
        assert!(!slow.active);
    }

    #[test]
    fn slow_motion_ends_when_the_gauge_is_empty_and_will_not_start_below_the_threshold() {
        let mut slow = gauge();
        slow.step(true, 20.0);
        assert_eq!(slow.level, 0.0);
        assert!(!slow.active);
        slow.step(true, 0.1);
        assert!(!slow.active);
    }

    #[test]
    fn the_hourglass_colour_follows_the_gauge() {
        let colors = HourglassColors {
            active: [1; 4],
            normal: [2; 4],
            unavailable: [3; 4],
        };
        let mut slow = gauge();
        assert_eq!(colors.pick(&slow), [2; 4]);
        slow.active = true;
        assert_eq!(colors.pick(&slow), [1; 4]);
        slow.active = false;
        slow.level = 0.0;
        assert_eq!(colors.pick(&slow), [3; 4]);
    }

    #[test]
    fn ammo_text_is_the_clip_and_the_pocket() {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.pickup_ammo("beretta", 99);
        assert_eq!(
            player.ammo_text("-"),
            Some(("18".to_string(), "99".to_string()))
        );
    }

    #[test]
    fn unlimited_ammo_shows_its_text_in_the_pocket_slot_alone() {
        let mut player = inventory();
        player.remove_all("beretta");
        assert_eq!(
            player.ammo_text("-"),
            Some((String::new(), "-".to_string()))
        );
        assert_eq!(PlayerInventory::default().ammo_text("-"), None);
    }

    #[test]
    fn a_picked_up_painkiller_counts_only_through_its_pocket() {
        let mut player = inventory();
        player.pickup_weapon("painkiller");
        assert_eq!(player.painkillers(), 0);
        player.pickup_ammo("painkiller", 3);
        assert_eq!(player.painkillers(), 3);
    }

    #[test]
    fn hud_txt_reads_the_hourglass_colours_and_unlimited_text() {
        let text = format!(
            "{HUD}\n[SlowMotion]\n{{\n[SlowMotionSpecific]\nColorActive = ffffffff;\nColorNormal = 788088d0;\nColorUnavailable = 000000d0;\n}}\n"
        );
        let text = text.replace(
            "id = WEAPONID_BERETTA;",
            "id = WEAPONID_BERETTA;\nUnlimitedAmmoText = \"-\";",
        );
        let hud = HudLayout::parse(&text).expect("hud");
        assert_eq!(hud.hourglass_colors.normal, [0x78, 0x80, 0x88, 0xd0]);
        assert_eq!(hud.hourglass_colors.unavailable, [0, 0, 0, 0xd0]);
        assert_eq!(hud.unlimited_text["WEAPONID_BERETTA"], "-");
    }

    #[test]
    fn a_glyph_is_one_wider_than_its_ruler_run_and_the_font_two_shorter_than_its_sheet() {
        assert_eq!(ruler_font().size("A"), [4.0, 4.0]);
    }

    #[test]
    fn text_is_as_wide_as_its_glyphs_with_no_spacing() {
        assert_eq!(ruler_font().size("AB"), [6.0, 4.0]);
    }

    #[test]
    fn a_sheet_without_256_ruler_runs_is_no_font() {
        assert!(HudFont::new(sheet(4, 3, vec![0xffff_ffff; 12])).is_none());
    }

    #[test]
    fn reference_points_name_the_box_corner_at_the_position() {
        let at = |reference| top_left(reference, [100.0, 200.0], [10.0, 20.0]);
        assert_eq!(at(0), [100.0, 180.0], "down left");
        assert_eq!(at(1), [100.0, 190.0], "left");
        assert_eq!(at(2), [100.0, 200.0], "up left");
        assert_eq!(at(3), [95.0, 200.0], "up");
        assert_eq!(at(4), [90.0, 200.0], "up right");
        assert_eq!(at(5), [90.0, 190.0], "right");
        assert_eq!(at(6), [90.0, 180.0], "down right");
        assert_eq!(at(7), [95.0, 180.0], "down");
        assert_eq!(at(8), [95.0, 190.0], "center");
    }

    #[test]
    fn blending_over_the_frame_mixes_and_stays_opaque() {
        assert_eq!(blend(0x0000_0000, 0x00ff_ffff, 128), 0x0080_8080);
        assert_eq!(blend(0x0040_4040, 0x00ff_0000, 255), 0x00ff_0000);
    }

    #[test]
    fn blending_over_a_clear_hud_pixel_keeps_the_colour_and_records_the_alpha() {
        assert_eq!(blend(crate::gpu::HUD_CLEAR, 0x00ff_8000, 128), 0x7fff_8000);
    }

    fn solid(rgb: u32) -> DecodedTexture {
        sheet(4, 4, vec![0xff00_0000 | rgb; 16])
    }

    /// The health figure's box: x 2.5..47.5, y 388..478 in HUD space.
    fn figure() -> HudSprite {
        HudSprite {
            reference: 7,
            position: [25.0, 478.0],
            width: 45.0,
            height: 90.0,
            alpha: 255,
            ..HudSprite::default()
        }
    }

    #[test]
    fn a_sprite_lands_at_its_640x480_box_scaled_to_the_frame() {
        let (width, height) = (1280, 960);
        let mut pixels = vec![0u32; width * height];
        let mut target = HudTarget {
            pixels: &mut pixels,
            width,
            height,
        };
        target.draw_sprite(&figure(), &solid(0x00ff_0000), 1.0);
        let at = |x: usize, y: usize| pixels[y * width + x];
        assert_eq!(at(50, 800), 0x00ff_0000);
        assert_eq!(at(3, 800), 0);
        assert_eq!(at(100, 800), 0);
        assert_eq!(at(50, 770), 0);
        assert_eq!(at(50, 950), 0x00ff_0000);
    }

    #[test]
    fn a_partly_visible_sprite_shows_its_lower_part() {
        let (width, height) = (1280, 960);
        let mut pixels = vec![0u32; width * height];
        let mut target = HudTarget {
            pixels: &mut pixels,
            width,
            height,
        };
        target.draw_sprite(&figure(), &solid(0x00ff_0000), 0.5);
        assert_eq!(pixels[800 * width + 50], 0, "upper half");
        assert_eq!(pixels[900 * width + 50], 0x00ff_0000, "lower half");
    }

    #[test]
    fn a_sprite_takes_its_vertex_alpha() {
        let (width, height) = (640, 480);
        let mut pixels = vec![0u32; width * height];
        let mut target = HudTarget {
            pixels: &mut pixels,
            width,
            height,
        };
        let sprite = HudSprite {
            alpha: 128,
            ..figure()
        };
        target.draw_sprite(&sprite, &solid(0x00ff_ffff), 1.0);
        assert_eq!(pixels[430 * width + 25], 0x0080_8080);
    }

    #[test]
    fn right_anchored_text_ends_at_its_position() {
        let font = ruler_font();
        let (width, height) = (640, 480);
        let mut pixels = vec![0u32; width * height];
        let mut target = HudTarget {
            pixels: &mut pixels,
            width,
            height,
        };
        let anchor = HudText {
            reference: 5,
            position: [100.0, 50.0],
        };
        // `A` is 4 wide; the sheet's rows below the ruler are empty but for this column.
        let mut filled = font.clone();
        for row in 1..6 {
            for k in 0..4 {
                let x = filled.glyphs[65].0 + k;
                filled.sheet.pixels[(row * filled.sheet.width + x) as usize] = 0xffff_ffff;
            }
        }
        target.draw_text(&filled, anchor, "A", [255; 4]);
        assert_eq!(pixels[50 * width + 98], 0x00ff_ffff);
        assert_eq!(pixels[50 * width + 101], 0);
        assert_eq!(pixels[50 * width + 94], 0);
    }

    /// Solid bitmaps for each sprite: figure grey, pain red, hourglass white.
    fn test_assets() -> HudAssets {
        let at = |x: f32, y: f32, w: f32, h: f32, file: &str, alpha: u8| HudSprite {
            reference: 7,
            position: [x, y],
            width: w,
            height: h,
            file: file.into(),
            alpha_file: String::new(),
            alpha,
        };
        let layout = HudLayout {
            health_background: at(25.0, 478.0, 45.0, 90.0, "figure", 255),
            health_pain: at(25.0, 478.0, 45.0, 90.0, "pain", 255),
            hourglass_background: at(60.0, 478.0, 16.0, 54.0, "glass", 255),
            hourglass: at(60.0, 478.0, 14.0, 50.0, "glass", 255),
            painkiller_icon: at(120.0, 478.0, 12.0, 20.0, "cross", 255),
            hourglass_colors: HourglassColors {
                active: [255; 4],
                normal: [0, 255, 0, 255],
                unavailable: [0, 0, 0, 255],
            },
            ..HudLayout::default()
        };
        let bitmaps = [
            ("figure", 0x0080_8080),
            ("pain", 0x00ff_0000),
            ("glass", 0x00ff_ffff),
            ("cross", 0x0000_00ff),
        ]
        .map(|(name, rgb)| (name.to_string(), solid(rgb)))
        .into_iter()
        .collect();
        HudAssets {
            layout,
            font: ruler_font(),
            bitmaps,
        }
    }

    fn drawn(health: f32, slow: &SlowMotion, inventory: &PlayerInventory) -> Vec<u32> {
        let mut pixels = vec![0u32; 640 * 480];
        let mut target = HudTarget {
            pixels: &mut pixels,
            width: 640,
            height: 480,
        };
        let state = HudState {
            health,
            slow_motion: slow,
            inventory,
        };
        target.draw_hud(&test_assets(), &state);
        pixels
    }

    #[test]
    fn at_full_health_only_the_figure_shows() {
        let pixels = drawn(1.0, &gauge(), &inventory());
        assert_eq!(pixels[400 * 640 + 25], 0x0080_8080);
        assert_eq!(pixels[470 * 640 + 25], 0x0080_8080);
    }

    #[test]
    fn pain_fills_the_figure_from_the_feet() {
        let pixels = drawn(0.5, &gauge(), &inventory());
        assert_eq!(pixels[400 * 640 + 25], 0x0080_8080);
        assert_eq!(pixels[470 * 640 + 25], 0x00ff_0000);
    }

    #[test]
    fn the_hourglass_empties_from_the_top_in_its_state_colour() {
        let mut slow = gauge();
        slow.level = 0.5;
        let pixels = drawn(1.0, &slow, &inventory());
        assert_eq!(pixels[470 * 640 + 60], 0x0000_ff00);
    }

    #[test]
    fn the_painkiller_icon_shows_only_with_painkillers() {
        let mut player = inventory();
        player.pickup_weapon("painkiller");
        assert_eq!(drawn(1.0, &gauge(), &player)[470 * 640 + 120], 0);
        player.pickup_ammo("painkiller", 2);
        assert_eq!(drawn(1.0, &gauge(), &player)[470 * 640 + 120], 0x0000_00ff);
    }

    #[test]
    fn retail_hud_assets_load_with_every_sprite() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        if crate::read_game_bytes(&game, r"\data\hud\hud.txt").is_none() {
            return;
        }
        let assets = HudAssets::load(&game).expect("assets");
        let layout = &assets.layout;
        for sprite in [
            &layout.health_background,
            &layout.health_pain,
            &layout.hourglass,
            &layout.hourglass_background,
            &layout.painkiller_icon,
            &layout.weapons["WEAPONID_BERETTA"],
        ] {
            let bitmap = assets.bitmaps.get(&sprite.file);
            assert!(bitmap.is_some(), "{}", sprite.file);
        }
        // The figure's corner is transparent and its middle is not.
        let figure = &assets.bitmaps[&layout.health_background.file];
        assert_eq!(figure.pixels[0] >> 24, 0);
        let middle = (figure.height / 2 * figure.width + figure.width / 2) as usize;
        assert!(figure.pixels[middle] >> 24 > 200);
    }

    #[test]
    fn the_ruler_row_does_not_bleed_into_scaled_text() {
        // The sheet below the ruler is empty, so nothing may show at any scale.
        let font = ruler_font();
        let (width, height) = (1280, 960);
        let mut pixels = vec![0u32; width * height];
        let mut target = HudTarget {
            pixels: &mut pixels,
            width,
            height,
        };
        let anchor = HudText {
            reference: 2,
            position: [100.0, 50.0],
        };
        target.draw_text(&font, anchor, "A", [255; 4]);
        assert!(pixels.iter().all(|px| *px == 0));
    }

    fn dot_pixels(player: &PlayerInventory, width: usize, height: usize) -> Vec<(usize, usize)> {
        let mut pixels = vec![0u32; width * height];
        let mut target = HudTarget {
            pixels: &mut pixels,
            width,
            height,
        };
        target.draw_crosshair(player);
        let white = |(_, px): &(usize, &u32)| **px == 0x00ff_ffff;
        pixels
            .iter()
            .enumerate()
            .filter(white)
            .map(|(i, _)| (i % width, i / width))
            .collect()
    }

    #[test]
    fn the_dot_is_three_pixels_square_at_the_centre_of_a_1080p_frame() {
        // Measured in retail frames `original/0104` and `0106`.
        let mut player = inventory();
        player.pickup_weapon("beretta");
        let dot = dot_pixels(&player, 1920, 1080);
        assert_eq!(dot.len(), 9);
        assert_eq!(dot[0], (959, 539));
        assert_eq!(dot[8], (961, 541));
    }

    #[test]
    fn no_weapon_shows_no_dot() {
        assert!(dot_pixels(&inventory(), 960, 600).is_empty());
    }

    #[test]
    fn a_weapon_with_the_invisible_crosshair_shows_no_dot() {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.defs.get_mut("beretta").expect("def").shows_dot = false;
        assert!(dot_pixels(&player, 960, 600).is_empty());
    }

    #[test]
    fn retail_font_rules_match_its_digits() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let Some(font) = load_font(&game, r"\data\hud\bitmaps\Hud_Universal_Medium") else {
            return;
        };
        assert_eq!(font.glyphs[b'8' as usize], (298, 8));
        assert_eq!(font.size("18"), [16.0, 21.0]);
    }

    const HUD: &str = r#"
#define DOWN 7
#define	RIGHT 5
#define	LEFT 1
#define DOWNRIGHT 6
#define WEAPON_ALPHA				192
#define	WEAPON_WIDTH				95
#define	WEAPON_POSITION				( 625, 471.5, 0 )
/*
[ConstantSprite]
Width = 200;
*/
[Health]
{
	[Visibility]
	Duration = 6.0 ;
	[Sprite]
	// reference point must be DOWN for this sprite
	ReferencePoint = DOWN;
	Position = ( 25, 478, 0 );
	Width = 45;
	Height = 90.0;
	Filename = "bitmaps\health_icon_foreground.jpg";
	AlphaFilename = "bitmaps\health_icon_foreground_alpha.pcx";
	DefaultVertexAlpha = 255;
	[BackgroundSprite]
	ReferencePoint = DOWN;
	Position = ( 25, 478, 0 );
	Width = 45;
	Height = 90.0;
	Filename = "bitmaps\health_icon_background.jpg";
	AlphaFilename = "bitmaps\health_icon_background_alpha.pcx";
	DefaultVertexAlpha = 128;
}
[SlowMotion]
{
	[Sprite]
	ReferencePoint = DOWN;
	Position = ( 49, 475, 0 );
	Width = 14;
	Height = 50.0;
	Filename = "bitmaps\hourglass_foreground.pcx";
	AlphaFilename = "bitmaps\hourglass_foreground_alpha.pcx";
	DefaultVertexAlpha = 128; // color overrides this in slowmotion sprite
	[BackgroundSprite]
	ReferencePoint = DOWN;
	Position = ( 49, 477, 0 );
	Width = 16;
	Height = 54.0;
	Filename = "bitmaps\hourglass_background.pcx";
	AlphaFilename = "bitmaps\hourglass_background_alpha.pcx";
	DefaultVertexAlpha = 128;
}
[Weapons]
{
	[AmmoInClipsText]
	ReferencePoint = RIGHT;
	Position = ( 582, 442.5, 0 );
	[AmmoInPocketText]
	ReferencePoint = LEFT;
	Position = ( 597, 442.5, 0 );
	[Painkiller]
	{
		[AmountText]
		ReferencePoint = LEFT;
		Position = ( 69, 467.5, 0);
		[Sprite]
		ReferencePoint = RIGHT;
		Position = ( 69, 466.5, 0);
		Width = 12;
		Height = 20;
		Filename = "bitmaps\health_icon_cross.jpg";
		AlphaFilename = "bitmaps\health_icon_cross_alpha.pcx";
		DefaultVertexAlpha = 179;
	}
	[ActiveWeapon]
	{
		[WeaponSpecific]
		id = WEAPONID_BERETTA;
		[Sprite]
	 	ReferencePoint = DOWNRIGHT;
		Position = WEAPON_POSITION ;
		Width = WEAPON_WIDTH ;
		Height = 35 ;
	    Filename = "bitmaps\weapons\beretta.jpg";
	    AlphaFilename = "bitmaps\weapons\beretta_alpha.pcx";
	    DefaultVertexAlpha = WEAPON_ALPHA ;
	}
	[ActiveWeapon]
	{
		[WeaponSpecific]
		id = WEAPONID_M79;
		[Sprite]
		ReferencePoint = DOWNRIGHT;
		Position = WEAPON_POSITION ;
		Width = WEAPON_WIDTH ;
		Height = 35 ;
		Filename = "bitmaps\weapons\m79.jpg";
		AlphaFilename = "bitmaps\weapons\m79_alpha.pcx";
		DefaultVertexAlpha = WEAPON_ALPHA ;
	}
}
"#;

    fn tip_style() -> HudPrintStyle {
        HudPrintStyle {
            anchor: HudText {
                reference: 2,
                position: [4.0, 100.0],
            },
            spacing: 20.0,
            max_lines: 3,
            color: [0xff, 0xe8, 0xa0, 0xff],
            duration: 10.0,
            fade_out: 0.5,
        }
    }

    #[test]
    fn hud_txt_reads_the_print_blocks() {
        let text = format!(
            "{HUD}\n#define UPLEFT 2\n[Print]\n{{\n[PrintSpecific]\nYSpacing = 20.0;\nMaximumLines = 10;\nColor = c0c0c0ff;\n[Visibility]\nDuration = 3;\nFadeInTime = 0.05;\nFadeOutTime = 0.1;\n[Text]\nReferencePoint = RIGHT;\nPosition = ( 620, 400.5, 0 );\n}}\n[PrintTip]\n{{\n[PrintSpecific]\nYSpacing = 20.0;\nMaximumLines = 3;\nColor = ffe8a0ff;\n[Visibility]\nDuration = 10;\nFadeOutTime = 0.5;\n[Text]\nReferencePoint = UPLEFT;\nPosition = ( 4, 100, 0 );\n}}\n"
        );
        let hud = HudLayout::parse(&text).expect("hud");
        assert_eq!(hud.tip, tip_style());
        assert_eq!(hud.print.anchor.position, [620.0, 400.5]);
        assert_eq!(hud.print.max_lines, 10);
        assert_eq!(hud.print.color, [0xc0, 0xc0, 0xc0, 0xff]);
    }

    #[test]
    fn the_string_table_reads_ids_and_text() {
        let table = parse_string_table(
            "[OBJ_A]\t\t\t\tString = \"Primo.\";\n\n[TIP_07]\t\tString = \"Rispondi al telefono.\";\n",
        );
        assert_eq!(table["TIP_07"], "Rispondi al telefono.");
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn tips_stack_down_from_their_anchor() {
        let style = tip_style();
        let mut log = HudPrintLog::default();
        log.push(&style, "a".into());
        log.push(&style, "b".into());
        let placed = log.placed(&style);
        assert_eq!(placed[0], ("a", [4.0, 100.0], 255));
        assert_eq!(placed[1], ("b", [4.0, 120.0], 255));
    }

    #[test]
    fn right_anchored_text_stacks_up_with_the_newest_lowest() {
        let style = HudPrintStyle {
            anchor: HudText {
                reference: 5,
                position: [620.0, 400.0],
            },
            ..tip_style()
        };
        let mut log = HudPrintLog::default();
        log.push(&style, "a".into());
        log.push(&style, "b".into());
        let placed = log.placed(&style);
        assert_eq!(placed[1].1, [620.0, 400.0]);
        assert_eq!(placed[0].1, [620.0, 380.0]);
    }

    #[test]
    fn a_line_fades_after_its_duration_and_then_goes() {
        let style = tip_style();
        let mut log = HudPrintLog::default();
        log.push(&style, "a".into());
        log.step(&style, 10.25);
        assert_eq!(log.placed(&style)[0].2, 127);
        log.step(&style, 0.5);
        assert!(log.lines.is_empty());
    }

    #[test]
    fn only_the_newest_lines_stay_when_the_block_is_full() {
        let style = tip_style();
        let mut log = HudPrintLog::default();
        for text in ["a", "b", "c", "d"] {
            log.push(&style, text.into());
        }
        assert_eq!(log.lines.len(), 3);
        assert_eq!(log.lines[0].0, "b");
    }

    #[test]
    fn tip_placeholders_name_the_engines_keys() {
        assert_eq!(expand_keys("premi #USE# ora"), "premi E ora");
        assert_eq!(expand_keys("#SHOOT# e #BULLETTIME#"), "Mouse 1 e Mouse 2");
        assert_eq!(expand_keys("#TOTALTIME#"), "#TOTALTIME#");
    }

    #[test]
    fn long_text_breaks_at_spaces_within_the_width() {
        let font = ruler_font();
        // Each glyph but `A` is 2 wide in this font.
        let lines = wrap_text(&font, "bb bb bb", 11.0);
        assert_eq!(lines, ["bb bb", "bb"]);
    }

    #[test]
    fn hud_txt_places_the_health_figure_and_its_pain_fill() {
        let hud = HudLayout::parse(HUD).expect("hud");
        assert_eq!(hud.health_pain.position, [25.0, 478.0]);
        assert_eq!(hud.health_pain.reference, 7);
        assert_eq!(
            (hud.health_pain.width, hud.health_pain.height),
            (45.0, 90.0)
        );
        assert_eq!(hud.health_pain.alpha, 255);
        assert_eq!(hud.health_background.alpha, 128);
        assert_eq!(
            hud.health_background.file,
            r"bitmaps\health_icon_background.jpg"
        );
    }

    #[test]
    fn hud_txt_defines_expand_inside_the_weapon_blocks() {
        let hud = HudLayout::parse(HUD).expect("hud");
        let beretta = &hud.weapons["WEAPONID_BERETTA"];
        assert_eq!(beretta.position, [625.0, 471.5]);
        assert_eq!((beretta.width, beretta.alpha), (95.0, 192));
        assert_eq!(beretta.alpha_file, r"bitmaps\weapons\beretta_alpha.pcx");
        assert_eq!(hud.weapons["WEAPONID_M79"].file, r"bitmaps\weapons\m79.jpg");
    }

    #[test]
    fn hud_txt_nested_blocks_keep_their_own_sprites() {
        let hud = HudLayout::parse(HUD).expect("hud");
        assert_eq!(hud.painkiller_icon.position, [69.0, 466.5]);
        assert_eq!(hud.painkiller_count.position, [69.0, 467.5]);
        assert_eq!(hud.ammo_in_clip.reference, 5);
        assert_eq!(hud.ammo_in_pocket.position, [597.0, 442.5]);
        assert_eq!(hud.hourglass.height, 50.0);
        assert_eq!(hud.hourglass_background.position, [49.0, 477.0]);
    }

    #[test]
    fn a_commented_out_block_is_ignored() {
        assert!(HudLayout::parse("[Health]\n{\n/* [Sprite] */\n}\n").is_none());
    }

    #[test]
    fn retail_hud_txt_names_every_weapon() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let Some(bytes) = crate::read_game_bytes(&game, r"\data\hud\hud.txt") else {
            return;
        };
        let hud = HudLayout::parse(&String::from_utf8_lossy(&bytes)).expect("retail hud");
        assert_eq!(hud.weapons.len(), 17);
        assert_eq!(hud.weapons["WEAPONID_BERETTA"].height, 35.0);
        assert_eq!(hud.weapons["WEAPONID_MOLOTOV"].width, 190.0);
    }
}
