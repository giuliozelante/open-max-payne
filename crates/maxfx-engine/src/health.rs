//! Health rules from `difficulty.txt`.

use crate::hud::parse_sections;
use crate::scene::CharacterOrder;

/// Where immortality stops health, in points (`0x3dcccccd` at 0x6960f2).
const IMMORTAL_FLOOR: f32 = 0.1;

/// `[normal]` / `[default]`, the difficulty a new game starts on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Difficulty {
    /// `EnemyHealthMultiplier`.
    pub enemy_health: f32,
    /// `PlayerHealthMultiplier`.
    pub player_health: f32,
    /// `HealthRegenerateMultiplier`: fraction of maximum health per second.
    pub regenerate: f32,
    /// `HealthGainedImmediately`: fraction of maximum health a heal can give at once.
    pub gained_immediately: f32,
    /// `AutoAimCapsuleRadiusMultiplier` and `AutoAimCapsuleHeightMultiplier`, at least 1 (0x6771af).
    pub auto_aim: [f32; 2],
}

impl Difficulty {
    pub fn parse(text: &str) -> Option<Self> {
        let sections = parse_sections(text);
        let block = sections
            .iter()
            .find(|section| section.path == "difficulty/normal/default")?;
        let percent = |key: &str| {
            let text = block.text(key)?.strip_suffix('%')?;
            Some(text.trim().parse::<f32>().ok()? / 100.0)
        };
        Some(Self {
            enemy_health: block.number("enemyhealthmultiplier")?,
            player_health: block.number("playerhealthmultiplier")?,
            regenerate: percent("healthregeneratemultiplier")?,
            gained_immediately: percent("healthgainedimmediately")?,
            auto_aim: [
                "autoaimcapsuleradiusmultiplier",
                "autoaimcapsuleheightmultiplier",
            ]
            .map(|key| block.number(key).unwrap_or(1.0).max(1.0)),
        })
    }
}

/// `globalconfig.txt` `[character]` falling damage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Falling {
    /// `DamageFallingThreshold`, in metres per second.
    threshold: f32,
    /// `RelativeFallingDamage`, as a fraction.
    relative: f32,
    /// `DamageFallingSpeedDeduct`.
    deduct: f32,
    /// `FallingDamageExponent`.
    exponent: f32,
}

impl Falling {
    pub fn parse(globalconfig: &str) -> Option<Self> {
        let sections = parse_sections(globalconfig);
        let block = sections
            .iter()
            .find(|section| section.path == "character")?;
        let relative = block.text("relativefallingdamage")?.strip_suffix('%')?;
        Some(Self {
            threshold: block.number("damagefallingthreshold")?,
            relative: relative.trim().parse::<f32>().ok()? / 100.0,
            deduct: block.number("damagefallingspeeddeduct")?,
            exponent: block.number("fallingdamageexponent")?,
        })
    }

    /// Points lost landing at `speed`, for a character of `max` health.
    pub fn damage(&self, speed: f32, max: f32) -> f32 {
        if speed < self.threshold {
            return 0.0;
        }
        (speed - self.deduct).powf(self.exponent) * max * self.relative
    }
}

/// `DamageFlash*` of `globalconfig.txt`, the red flash of a hurt player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageFlash {
    points: [f32; 2],
    color: [u32; 2],
    fade_out: [f32; 2],
    stay: [f32; 2],
}

impl DamageFlash {
    pub fn parse(globalconfig: &str) -> Option<Self> {
        let sections = parse_sections(globalconfig);
        let block = sections
            .iter()
            .find(|section| section.path == "character")?;
        let pair = |key: &str| {
            let at = |end: &str| block.number(&format!("damageflash{end}{key}"));
            Some([at("minimum")?, at("maximum")?])
        };
        let color = |end: &str| {
            let text = block.text(&format!("damageflash{end}color"))?;
            u32::from_str_radix(text.trim(), 16).ok()
        };
        Some(Self {
            points: pair("hitpoints")?,
            color: [color("minimum")?, color("maximum")?],
            fade_out: pair("fadeouttime")?,
            stay: pair("stayintime")?,
        })
    }

    /// The flash for losing `points` at once. HYPOTHESIS: linear between the two ends.
    pub fn of(&self, points: f32) -> Option<crate::scene::ScreenFlash> {
        if points < self.points[0] {
            return None;
        }
        let t = ((points - self.points[0]) / (self.points[1] - self.points[0])).clamp(0.0, 1.0);
        let mix = |[a, b]: [f32; 2]| a + (b - a) * t;
        let channel = |shift: u32| {
            let [a, b] = self.color.map(|color| ((color >> shift) & 0xff) as f32);
            (mix([a, b]).round() as u32) << shift
        };
        Some(crate::scene::ScreenFlash {
            color: channel(24) | channel(16) | channel(8) | channel(0),
            fade_in: 0.0,
            hold: mix(self.stay),
            fade_out: mix(self.fade_out),
            age: 0.0,
        })
    }
}

/// Max's health, in points. `docs/rea/2026-11-06-character-health.md` has the addresses.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerHealth {
    pub health: f32,
    pub max: f32,
    /// Healing still to arrive.
    pending: f32,
    regenerate: f32,
    gained_immediately: f32,
    pub immortal: bool,
    pub dodge_immortal: bool,
    pub invulnerable: bool,
}

impl PlayerHealth {
    /// `skin_max` is the skin's `MaximumHealth`.
    pub fn new(skin_max: f32, difficulty: &Difficulty) -> Self {
        let max = skin_max * difficulty.player_health;
        Self {
            health: max,
            max,
            pending: 0.0,
            regenerate: difficulty.regenerate,
            gained_immediately: difficulty.gained_immediately,
            immortal: false,
            dodge_immortal: false,
            invulnerable: false,
        }
    }

    /// Normalized health for the HUD.
    pub fn fraction(&self) -> f32 {
        self.health / self.max
    }

    /// 0x695fb0.
    fn set(&mut self, health: f32) {
        if self.invulnerable {
            return;
        }
        let mut health = health.clamp(0.0, self.max);
        if (self.immortal || self.dodge_immortal) && self.health > 0.0 {
            health = health.max(IMMORTAL_FLOOR);
        }
        self.health = health;
    }

    /// `C_SetHealth`, 0x699ad0.
    pub fn set_fraction(&mut self, fraction: f32) {
        self.set(fraction * self.max);
    }

    /// `C_CauseDamage`, 0x695b10.
    pub fn damage(&mut self, points: f32) {
        self.set(self.health - points);
    }

    /// `C_AddHealth`, 0x695a80: part now, the rest through [`Self::step`].
    pub fn add(&mut self, points: f32) {
        let now = points.min(self.gained_immediately * self.max);
        self.pending += points - now;
        if self.health > 0.0 {
            self.set(self.health + now);
        }
    }

    /// 0x68f510.
    pub fn step(&mut self, seconds: f32) {
        if self.pending <= 0.0 || self.health <= 0.0 {
            return;
        }
        let gain = (self.regenerate * seconds * self.max).min(self.pending);
        self.set(self.health + gain);
        self.pending -= gain;
    }

    pub fn obey(&mut self, order: &CharacterOrder) {
        match order {
            CharacterOrder::SetHealth(fraction) => self.set_fraction(*fraction),
            CharacterOrder::AddHealth(points) => self.add(*points),
            CharacterOrder::CauseDamage(points) => self.damage(*points),
            CharacterOrder::SetImmortal(on) => self.immortal = *on,
            CharacterOrder::SetDodgeImmortal(on) => self.dodge_immortal = *on,
            CharacterOrder::SetInvulnerable(on) => self.invulnerable = *on,
            CharacterOrder::Teleport(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str =
        "[difficulty]\n{\n\t[normal]\n\t{\n\t\t[properties]\n\t\tSaveGameLimit = 0;\n\
        \t\t[minimum]\n\t\tEnemyHealthMultiplier = 0.25;\n\t\tPlayerHealthMultiplier = 2.0;\n\
        \t\t[default]\n\t\tEnemyHealthMultiplier = 0.8;\n\t\tPlayerHealthMultiplier = 1.0;\n\
        \t\tHealthRegenerateMultiplier = 4%;\n\t\tHealthGainedImmediately = 5%;\n\
        \t}\n\t[nightmare]\n\t{\n\t\t[default]\n\t\tEnemyHealthMultiplier = 1.2;\n\
        \t\tPlayerHealthMultiplier = 0.4;\n\t}\n}\n";

    const CONFIG: &str = "[character]\nDamageFallingThreshold = 15.0; // velocity\n\
        RelativeFallingDamage = 1%;\nDamageFallingSpeedDeduct = 10.0;\nFallingDamageExponent = 2.1;\n\
        DamageFlashMinimumHitPoints = 1;\nDamageFlashMinimumColor = A5000010;\n\
        DamageFlashMinimumFadeOutTime = 0.05;\nDamageFlashMinimumStayInTime = 0.1;\n\
        DamageFlashMaximumHitPoints = 50;\nDamageFlashMaximumColor = A50000D0;\n\
        DamageFlashMaximumFadeOutTime = 1.0;\nDamageFlashMaximumStayInTime = 0.1;\n";

    #[test]
    fn five_metres_are_safe_and_ten_kill_as_the_config_says() {
        let falling = Falling::parse(CONFIG).expect("falling");
        // Gravity -20: 5 m lands at 14.1 m/s, 10 m at 20 m/s.
        assert_eq!(falling.damage(14.1, 60.0), 0.0);
        assert!(falling.damage(20.0, 60.0) >= 60.0);
        let at_threshold = falling.damage(15.0, 60.0);
        assert!((at_threshold - 5f32.powf(2.1) * 0.6).abs() < 1e-3);
    }

    #[test]
    fn a_flash_grows_with_the_points_lost_and_stops_at_its_ends() {
        let flash = DamageFlash::parse(CONFIG).expect("flash");
        assert_eq!(flash.of(0.5), None);
        let small = flash.of(1.0).expect("small");
        assert_eq!(small.color, 0xA500_0010);
        assert_eq!(small.fade_out, 0.05);
        let big = flash.of(80.0).expect("big");
        assert_eq!(big.color, 0xA500_00D0);
        assert_eq!(big.fade_out, 1.0);
        assert_eq!(big.hold, 0.1);
    }

    #[test]
    fn the_normal_default_multipliers_are_read() {
        let difficulty = Difficulty::parse(TEXT).expect("parsed");
        assert_eq!(difficulty.enemy_health, 0.8);
        assert_eq!(difficulty.player_health, 1.0);
    }

    fn max() -> PlayerHealth {
        let difficulty = Difficulty {
            enemy_health: 0.8,
            player_health: 1.0,
            regenerate: 0.04,
            gained_immediately: 0.05,
            auto_aim: [1.0, 1.0],
        };
        PlayerHealth::new(60.0, &difficulty)
    }

    #[test]
    fn percent_keys_are_read_as_fractions() {
        let difficulty = Difficulty::parse(TEXT).expect("parsed");
        assert_eq!(difficulty.regenerate, 0.04);
        assert_eq!(difficulty.gained_immediately, 0.05);
    }

    #[test]
    fn max_starts_full_and_a_fraction_sets_health() {
        let mut max = max();
        assert_eq!(max.fraction(), 1.0);
        max.set_fraction(0.5);
        assert_eq!(max.health, 30.0);
    }

    #[test]
    fn damage_subtracts_points_and_stops_at_zero() {
        let mut max = max();
        max.damage(15.0);
        assert_eq!(max.health, 45.0);
        max.damage(100.0);
        assert_eq!(max.health, 0.0);
    }

    #[test]
    fn healing_gives_a_capped_part_at_once_and_regenerates_the_rest() {
        let mut max = max();
        max.set_fraction(0.5);
        max.add(12.0);
        // 5% of 60 at once; 9 left for later at 4% of 60 per second.
        assert_eq!(max.health, 33.0);
        max.step(1.0);
        assert!((max.health - 35.4).abs() < 1e-4, "{}", max.health);
        max.step(100.0);
        assert!((max.health - 42.0).abs() < 1e-4, "{}", max.health);
    }

    #[test]
    fn the_dead_do_not_heal() {
        let mut max = max();
        max.damage(60.0);
        max.add(12.0);
        max.step(10.0);
        assert_eq!(max.health, 0.0);
    }

    #[test]
    fn immortal_max_stops_at_a_tenth_of_a_point() {
        let mut max = max();
        max.immortal = true;
        max.damage(100.0);
        assert!((max.health - 0.1).abs() < 1e-6, "{}", max.health);
        let mut dodging = self::tests::max();
        dodging.dodge_immortal = true;
        dodging.damage(100.0);
        assert!((dodging.health - 0.1).abs() < 1e-6, "{}", dodging.health);
    }

    #[test]
    fn orders_reach_max() {
        let mut max = max();
        max.obey(&CharacterOrder::CauseDamage(30.0));
        assert_eq!(max.health, 30.0);
        max.obey(&CharacterOrder::SetInvulnerable(true));
        max.obey(&CharacterOrder::CauseDamage(30.0));
        assert_eq!(max.health, 30.0);
        max.obey(&CharacterOrder::SetInvulnerable(false));
        max.obey(&CharacterOrder::SetHealth(1.0));
        assert_eq!(max.fraction(), 1.0);
    }

    #[test]
    fn invulnerable_max_takes_nothing() {
        let mut max = max();
        max.invulnerable = true;
        max.damage(10.0);
        assert_eq!(max.health, 60.0);
    }

    #[test]
    fn a_file_without_the_normal_default_block_gives_nothing() {
        assert_eq!(Difficulty::parse("[difficulty]\n{\n}\n"), None);
    }
}
