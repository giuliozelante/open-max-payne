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
