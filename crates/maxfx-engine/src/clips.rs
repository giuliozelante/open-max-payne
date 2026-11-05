//! A character's clips: the `[Animation]` table of its skeleton, with the `[Modifiers]` of its
//! skin over it. Each clip names its file, how far it carries the character and what it says on
//! which frame, so everything a character does on the move is data.

use crate::kf2::Kf2Anim;
use std::collections::BTreeMap;
use std::sync::Arc;

/// One `[Animation]` block.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClipDef {
    /// `Filename`, relative to the skeleton's folder.
    pub file: String,
    /// `EndPosition`: `(right, up, forward)` the clip has carried the character when it ends.
    pub end: [f32; 3],
    /// `MiddlePosition1` and `MiddlePosition2`.
    pub middle: Option<[[f32; 3]; 2]>,
    /// `(frame, message)` in file order.
    pub messages: Vec<(u32, String)>,
}

impl ClipDef {
    /// The clip's movement in metres per second, for a clip of `seconds`.
    pub fn velocity(&self, seconds: f32) -> [f32; 3] {
        if seconds > 0.0 {
            self.end.map(|c| c / seconds)
        } else {
            [0.0; 3]
        }
    }
}

/// Every clip of a character by `CHARANIM_*` index.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClipTable {
    clips: BTreeMap<String, ClipDef>,
    reverts: BTreeMap<String, String>,
}

impl ClipTable {
    /// `skeleton`'s table with `skin`'s lines applied after it, each file with its own defines.
    pub fn parse(skeleton: &str, skin: &str) -> Self {
        let mut table = Self::default();
        table.read(skeleton);
        table.read(skin);
        table
    }

    fn read(&mut self, text: &str) {
        let mut defines = BTreeMap::new();
        let mut current = None;
        for line in text.lines() {
            let line = line.split("//").next().unwrap_or_default();
            if let Some(define) = line.trim().strip_prefix("#define") {
                if let Some((name, value)) = define.trim().split_once(char::is_whitespace) {
                    defines.insert(name.to_string(), value.trim().to_string());
                }
                continue;
            }
            let line = crate::hud::expand(line, &defines);
            if let Some((index, file)) = value(&line, "Index").zip(value(&line, "Filename")) {
                self.reverts.remove(index);
                let file = file.to_string();
                self.clips.insert(
                    index.to_string(),
                    ClipDef {
                        file,
                        ..Default::default()
                    },
                );
                current = Some(index.to_string());
            } else if let Some((from, to)) = value(&line, "From").zip(value(&line, "To")) {
                self.clips.remove(from);
                self.reverts.insert(from.to_string(), to.to_string());
            } else if let Some(clip) = current.as_ref().and_then(|at| self.clips.get_mut(at)) {
                if line.contains("[Movement]") {
                    clip.end = vector(&line, "EndPosition").unwrap_or_default();
                    clip.middle = vector(&line, "MiddlePosition1")
                        .zip(vector(&line, "MiddlePosition2"))
                        .map(|(first, second)| [first, second]);
                } else if line.contains("[Message]") {
                    clip.messages.extend(message(&line));
                }
            }
        }
    }

    /// The clip of `index`, through its `[Revert]`s.
    pub fn get(&self, index: &str) -> Option<&ClipDef> {
        let mut index = index;
        for _ in 0..=self.reverts.len() {
            if let Some(clip) = self.clips.get(index) {
                return Some(clip);
            }
            index = self.reverts.get(index)?;
        }
        None
    }

    /// Every `Filename` of the table.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.clips.values().map(|clip| clip.file.as_str())
    }
}

/// A character's table with its clips read.
#[derive(Debug, Clone, Default)]
pub struct CharacterClips {
    pub table: ClipTable,
    /// The clips by `Filename`.
    pub anims: BTreeMap<String, Arc<Kf2Anim>>,
    /// The skin's `AirborneSpeed`, in metres per second.
    pub airborne_speed: f32,
}

impl CharacterClips {
    /// The clip of `index` and its file.
    pub fn get(&self, index: &str) -> Option<(&ClipDef, &Arc<Kf2Anim>)> {
        let def = self.table.get(index)?;
        Some((def, self.anims.get(&def.file)?))
    }
}

/// `Key = value;` of a line, the key in any case.
fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let lower = line.to_ascii_lowercase();
    let key = key.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(&key) {
        let start = from + found;
        from = start + key.len();
        let whole = !lower[..start].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
        if let Some(rest) = line[from..]
            .trim_start()
            .strip_prefix('=')
            .filter(|_| whole)
        {
            return Some(rest.split(';').next()?.trim());
        }
    }
    None
}

/// `Key = ( x, y, z );` of a line. Defines leave a space after a minus sign.
fn vector(line: &str, key: &str) -> Option<[f32; 3]> {
    let text = value(line, key)?.trim_matches(|c| c == '(' || c == ')');
    let mut parts = text.split(',').map(|part| part.replace(' ', "").parse());
    let mut next = || parts.next()?.ok();
    Some([next()?, next()?, next()?])
}

/// `[Message] Frame = n; String = "a" b "c";` with its pieces joined.
fn message(line: &str) -> Option<(u32, String)> {
    let frame = value(line, "Frame")?.parse().ok()?;
    let (_, text) = line.split_once("String")?;
    let text = text.split_once('=')?.1;
    let text = text.get(text.find('"')?..=text.rfind('"')?)?;
    Some((frame, text.replace('"', "")))
}

/// The `CHARANIM_*` index of a run in the direction `forward` and `right` hold (each -1, 0 or 1),
/// or `None` standing still.
pub fn run_index(forward: i32, right: i32, two_handed: bool) -> Option<String> {
    let name = match (forward.signum(), right.signum()) {
        (0, 0) => return None,
        (1, 0) => "RUN",
        (-1, 0) => "WALKBACKWARD",
        (0, 1) => "STRAFERIGHT",
        (0, _) => "STRAFELEFT",
        (1, 1) => "FORWARDRIGHT",
        (1, _) => "FORWARDLEFT",
        (_, 1) => "BACKWARDRIGHT",
        _ => "BACKWARDLEFT",
    };
    let hands = if two_handed { "2_" } else { "" };
    Some(format!("CHARANIM_{hands}{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKELETON: &str = "#define ENEMY_RUN 3.0\n\
        [Skeleton]\n{\n\
        \t[Animation] Index = CHARANIM_RUN; Filename = anim\\run.kf2;\n\
        \t[Properties]\n\t{\n\
        \t\t[Movement] EndPosition = (0,0, ENEMY_RUN ); Rotation = (0,0,0);\n\
        \t}\n\
        \t[Animation] Index = CHARANIM_JUMP; Filename = anim\\jumpUp.kf2;\n\
        \t[Properties]\n\t{\n\
        \t\t//[Message] Frame = 1; String = \"this->off();\";\n\
        \t\t[Message] Frame = 10; String = \"this->C_Jump( 7.5  );\";\n\
        \t}\n\
        \t[Revert] From = CHARANIM_W_JUMP; To = CHARANIM_JUMP;\n\
        \t[Revert] From = CHARANIM_W_2_JUMP; To = CHARANIM_W_JUMP;\n\
        \t[Animation] Index = CHARANIM_WALK; Filename = anim\\walk.kf2;\n\
        }\n";

    const SKIN: &str = "#define PLAYER_MOVEMENT 4.2\n#define PLAYER_DODGE 3.5\n\
        #define PLAYER_JUMPSIDE 3\n\
        [Modifiers]\n{\n\
        \t[Animation]\n\tIndex = CHARANIM_RUN; Filename = anim\\run.kf2;\n\
        \t[Properties]\n\t{\n\
        \t\t[Movement] EndPosition = (0,0, PLAYER_MOVEMENT ); Rotation = (0,0,0);\n\
        \t\t[Message] Frame = 12; String = \"this->P_Createprojectiletobone(dummy_footstep_right,1,foot-r);\";\n\
        \t}\n\
        \t[Animation] Index = CHARANIM_DODGELEFT; Filename = anim\\dodgeleft.kf2;\n\
        \t[Properties]\n\t{\n\
        \t\t[Movement] EndPosition = ( - PLAYER_DODGE , 0, 0 ); Rotation = ( 0, 0, 0 ); Middleposition1 = ( -1.15, 0, 1.15 ); MiddlePosition2 = ( -2.70, 0, 2.70 );\n\
        \t\t[Message] Frame = 10; String = \"this->C_Jump( \" PLAYER_JUMPSIDE \"  );\";\n\
        \t}\n\
        \t[Revert] From = CHARANIM_WALK; To = CHARANIM_RUN;\n\
        }\n";

    #[test]
    fn a_skin_replaces_the_skeleton_clip_with_its_own_movement() {
        let table = ClipTable::parse(SKELETON, SKIN);
        let run = table.get("CHARANIM_RUN").expect("run");
        assert_eq!(run.file, r"anim\run.kf2");
        assert_eq!(run.end, [0.0, 0.0, 4.2]);
        assert_eq!(run.messages[0].0, 12);
    }

    #[test]
    fn a_define_after_a_minus_sign_and_inside_a_message_expands() {
        let table = ClipTable::parse(SKELETON, SKIN);
        let dodge = table.get("CHARANIM_DODGELEFT").expect("dodge");
        assert_eq!(dodge.end, [-3.5, 0.0, 0.0]);
        assert_eq!(dodge.middle.expect("path")[1], [-2.7, 0.0, 2.7]);
        assert_eq!(dodge.messages, [(10, "this->C_Jump(  3   );".to_string())]);
    }

    #[test]
    fn a_commented_message_is_not_read() {
        let table = ClipTable::parse(SKELETON, SKIN);
        assert_eq!(table.get("CHARANIM_JUMP").expect("jump").messages.len(), 1);
    }

    #[test]
    fn a_revert_chain_ends_on_a_real_clip() {
        let table = ClipTable::parse(SKELETON, SKIN);
        let jump = table.get("CHARANIM_W_2_JUMP").expect("revert");
        assert_eq!(jump.file, r"anim\jumpUp.kf2");
    }

    #[test]
    fn a_skin_revert_overrides_the_skeleton_clip() {
        let table = ClipTable::parse(SKELETON, SKIN);
        let walk = table.get("CHARANIM_WALK").expect("walk");
        assert_eq!(walk.file, r"anim\run.kf2");
    }

    #[test]
    fn a_clip_moves_its_end_position_over_its_length() {
        let clip = ClipDef {
            end: [0.0, 0.0, 4.2],
            ..Default::default()
        };
        assert!((clip.velocity(0.8)[2] - 5.25).abs() < 1e-5);
        assert_eq!(clip.velocity(0.0), [0.0; 3]);
    }

    #[test]
    fn eight_directions_name_eight_clips() {
        let index = |f, r| run_index(f, r, false).expect("moving");
        assert_eq!(index(1, 0), "CHARANIM_RUN");
        assert_eq!(index(-1, 0), "CHARANIM_WALKBACKWARD");
        assert_eq!(index(0, -1), "CHARANIM_STRAFELEFT");
        assert_eq!(index(1, 1), "CHARANIM_FORWARDRIGHT");
        assert_eq!(index(-1, -1), "CHARANIM_BACKWARDLEFT");
        assert_eq!(run_index(0, 0, false), None);
    }

    #[test]
    fn a_two_handed_weapon_names_the_two_handed_run() {
        assert_eq!(run_index(1, 0, true).as_deref(), Some("CHARANIM_2_RUN"));
    }
}
