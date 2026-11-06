//! Health rules from `difficulty.txt`.

use crate::hud::parse_sections;

/// `[normal]` / `[default]`, the difficulty a new game starts on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Difficulty {
    /// `EnemyHealthMultiplier`.
    pub enemy_health: f32,
    /// `PlayerHealthMultiplier`.
    pub player_health: f32,
}

impl Difficulty {
    pub fn parse(text: &str) -> Option<Self> {
        let sections = parse_sections(text);
        let block = sections
            .iter()
            .find(|section| section.path == "difficulty/normal/default")?;
        Some(Self {
            enemy_health: block.number("enemyhealthmultiplier")?,
            player_health: block.number("playerhealthmultiplier")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str =
        "[difficulty]\n{\n\t[normal]\n\t{\n\t\t[properties]\n\t\tSaveGameLimit = 0;\n\
        \t\t[minimum]\n\t\tEnemyHealthMultiplier = 0.25;\n\t\tPlayerHealthMultiplier = 2.0;\n\
        \t\t[default]\n\t\tEnemyHealthMultiplier = 0.8;\n\t\tPlayerHealthMultiplier = 1.0;\n\
        \t}\n\t[nightmare]\n\t{\n\t\t[default]\n\t\tEnemyHealthMultiplier = 1.2;\n\
        \t\tPlayerHealthMultiplier = 0.4;\n\t}\n}\n";

    #[test]
    fn the_normal_default_multipliers_are_read() {
        let difficulty = Difficulty::parse(TEXT).expect("parsed");
        assert_eq!(difficulty.enemy_health, 0.8);
        assert_eq!(difficulty.player_health, 1.0);
    }

    #[test]
    fn a_file_without_the_normal_default_block_gives_nothing() {
        assert_eq!(Difficulty::parse("[difficulty]\n{\n}\n"), None);
    }
}
