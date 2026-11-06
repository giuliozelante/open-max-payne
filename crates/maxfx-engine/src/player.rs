//! What Max's body is doing: which clip plays, where it carries him and what it says on the way.
//! The clips hold the numbers; this holds only the order they follow each other in.

use crate::clips::{CharacterClips, ClipDef};
use crate::kf2::Kf2Anim;
use crate::render3d::clip_seconds;
use std::sync::Arc;

/// The share of a direction that counts as leaning that way: `sin 22.5°`, so eight clips share
/// the circle evenly.
const OCTANT: f32 = 0.382_683_43;

/// What a press of the dodge button starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DodgeKind {
    /// The `CHARANIM_DODGE*` roll.
    Roll,
    /// The `CHARANIM_SHOOTDODGE*` dive.
    Dive,
}

/// The player's hands on the controls this frame, and where the world left him.
#[derive(Debug, Clone, Copy, Default)]
pub struct Controls {
    /// -1, 0 or 1.
    pub forward: i32,
    pub right: i32,
    pub jump: bool,
    pub crouch: bool,
    pub dodge: Option<DodgeKind>,
    pub two_handed: bool,
    pub grounded: bool,
    /// Radians, as the camera turns.
    pub yaw: f32,
    /// `AirborneSpeed`: metres per second the keys give in the air.
    pub airborne_speed: f32,
}

/// One frame of motion.
#[derive(Debug, Clone, Default)]
pub struct MotionStep {
    /// The clip to draw; `None` is the stand.
    pub clip: Option<Arc<Kf2Anim>>,
    /// World `(x, z)` in metres per second.
    pub velocity: [f32; 2],
    /// The clip messages this frame passed, in frame order.
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Ground,
    Crouch,
    Jump {
        left_ground: bool,
    },
    Fall,
    Land,
    Roll,
    Dive,
    GetUp,
    Dead,
}

/// Max's body from frame to frame.
#[derive(Debug, Clone, Default)]
pub struct PlayerMotion {
    state: State,
    death_over: bool,
    index: Option<String>,
    /// Seconds into the clip; negative until its first frame has passed.
    time: f32,
    /// Heading a roll or dive set off on, which the mouse no longer changes.
    heading: f32,
    /// The path of the clip a dodge set off with.
    path: ClipDef,
    /// The clip whose messages a dive follows.
    script: String,
}

impl PlayerMotion {
    /// Seconds into the clip playing.
    /// The index of the clip Max plays, if not the stand.
    pub fn index(&self) -> Option<&str> {
        self.index.as_deref()
    }

    pub fn time(&self) -> f32 {
        self.time.max(0.0)
    }

    /// In a roll or a dive, and so unable to start another.
    pub fn dodging(&self) -> bool {
        matches!(self.state, State::Roll | State::Dive | State::GetUp)
    }

    /// In a dive, whose clips already aim the arms.
    pub fn diving(&self) -> bool {
        self.state == State::Dive
    }

    /// Falls down dead along `CHARANIM_RANDOMDEATH1`, the clip every Max death reverts to.
    pub fn die(&mut self, clips: &CharacterClips, yaw: f32) {
        if self.state == State::Dead {
            return;
        }
        let index = "CHARANIM_RANDOMDEATH1".to_string();
        self.path = clips.table.get(&index).cloned().unwrap_or_default();
        self.heading = yaw;
        self.begin(State::Dead, index);
    }

    /// The death clip has played to its end.
    pub fn death_over(&self) -> bool {
        self.death_over
    }

    /// Crouching, for the camera's lower point.
    pub fn crouching(&self) -> bool {
        self.state == State::Crouch
    }

    /// A dodge button press would start one now.
    pub fn can_dodge(&self, grounded: bool) -> bool {
        grounded && matches!(self.state, State::Ground | State::Crouch | State::Land)
    }

    fn begin(&mut self, state: State, index: String) {
        self.state = state;
        self.index = Some(index);
        self.time = -1.0;
    }

    fn start_dodge(&mut self, clips: &CharacterClips, c: &Controls, kind: DodgeKind) {
        let hands = hands(c.two_handed);
        let way = direction_name(c.forward, c.right);
        let (state, index, script) = match kind {
            DodgeKind::Roll => {
                let index = format!("CHARANIM_{hands}DODGE{way}");
                (State::Roll, index.clone(), index)
            }
            DodgeKind::Dive => (
                State::Dive,
                format!("CHARANIM_{hands}SHOOTDODGE{way}"),
                format!("CHARANIM_{hands}SHOOTDODGEFORWARD"),
            ),
        };
        self.path = clips.table.get(&index).cloned().unwrap_or_default();
        self.script = script;
        self.heading = c.yaw;
        self.begin(state, index);
    }

    fn start_jump(&mut self, c: &Controls) {
        let name = match (c.forward > 0, c.right.signum()) {
            (true, 1) => "JUMPFORWARDRIGHT",
            (true, -1) => "JUMPFORWARDLEFT",
            (true, _) => "JUMPFORWARD",
            _ => "JUMP",
        };
        let index = format!("CHARANIM_{}{name}", hands(c.two_handed));
        self.begin(State::Jump { left_ground: false }, index);
    }

    fn land(&mut self, c: &Controls) {
        let index = format!("CHARANIM_{}LAND", hands(c.two_handed));
        self.begin(State::Land, index);
    }

    fn fall(&mut self, c: &Controls) {
        let index = format!("CHARANIM_{}FALL", hands(c.two_handed));
        self.begin(State::Fall, index);
    }

    /// On his feet, the keys pick the clip.
    fn stand(&mut self, c: &Controls) {
        let moving = c.forward != 0 || c.right != 0;
        if c.crouch && !moving {
            if self.state != State::Crouch {
                let index = format!("CHARANIM_{}CROUCH", hands(c.two_handed));
                self.begin(State::Crouch, index);
            }
            return;
        }
        if self.state != State::Ground {
            self.time = -1.0;
        }
        self.state = State::Ground;
        self.index = crate::clips::run_index(c.forward, c.right, c.two_handed);
    }

    /// What the controls and the floor change before the clip advances.
    fn react(&mut self, clips: &CharacterClips, c: &Controls) {
        match self.state {
            State::Ground | State::Crouch | State::Land => {
                let moving = c.forward != 0 || c.right != 0;
                if let Some(kind) = c.dodge.filter(|_| c.grounded && moving) {
                    self.start_dodge(clips, c, kind);
                } else if c.jump && c.grounded {
                    // Jump clips only face forwards, so any other way rolls.
                    if c.forward < 0 || (c.forward == 0 && c.right != 0) {
                        self.start_dodge(clips, c, DodgeKind::Roll);
                    } else {
                        self.start_jump(c);
                    }
                } else if !c.grounded {
                    self.fall(c);
                } else if self.state != State::Land || moving {
                    self.stand(c);
                }
            }
            State::Jump { left_ground } => {
                if left_ground && c.grounded {
                    self.land(c);
                } else if !c.grounded {
                    self.state = State::Jump { left_ground: true };
                }
            }
            State::Fall if c.grounded => self.land(c),
            State::Fall | State::Roll | State::Dive | State::GetUp | State::Dead => {}
        }
    }

    /// A clip that ran out hands over to what follows it.
    fn finish(&mut self, c: &Controls) {
        match self.state {
            State::Jump { .. } if !c.grounded => self.fall(c),
            State::Dive if c.grounded => {
                let way = quarter_name(self.heading_from(c.yaw));
                let hands = hands(c.two_handed);
                self.begin(
                    State::GetUp,
                    format!("CHARANIM_{hands}SHOOTDODGE{way}_GETUP"),
                );
            }
            State::Jump { .. } | State::Land | State::Roll | State::GetUp => {
                self.state = State::Ground;
                self.index = None;
                self.time = 0.0;
                self.stand(c);
            }
            _ => {}
        }
    }

    /// `(forward, right)` of the dodge's heading against the way Max faces at `yaw`.
    fn heading_from(&self, yaw: f32) -> (f32, f32) {
        let [x, z] = turn(self.path.end, self.heading);
        let (sin, cos) = yaw.sin_cos();
        let length = (x * x + z * z).sqrt().max(1e-6);
        ((x * sin + z * cos) / length, (x * cos - z * sin) / length)
    }

    /// In a dive the clip follows the crosshair: each one is the same dive seen from another side.
    fn follow_aim(&mut self, c: &Controls) {
        if self.state == State::Dive {
            let way = octant_name(self.heading_from(c.yaw));
            let hands = hands(c.two_handed);
            self.index = Some(format!("CHARANIM_{hands}SHOOTDODGE{way}"));
        }
    }

    fn velocity(&self, def: &ClipDef, seconds: f32, c: &Controls) -> [f32; 2] {
        let keys = [c.forward as f32, c.right as f32];
        let length = keys[0].hypot(keys[1]).max(1.0);
        let air = c.airborne_speed / length;
        let air = turn([keys[1] * air, 0.0, keys[0] * air], c.yaw);
        let clip = |yaw: f32| turn(def.velocity(seconds), yaw);
        match self.state {
            // A run's `EndPosition` is a speed: retail Max covers about 4 m/s on a 4.2 clip.
            State::Ground => turn(def.end, c.yaw),
            State::GetUp => clip(c.yaw),
            State::Jump { .. } => {
                let [x, z] = clip(c.yaw);
                [x + air[0], z + air[1]]
            }
            State::Fall => air,
            State::Roll | State::Dive | State::Dead => turn(
                path_velocity(&self.path, seconds, self.time()),
                self.heading,
            ),
            State::Crouch | State::Land => [0.0; 2],
        }
    }

    /// Moves on `seconds`.
    pub fn step(&mut self, clips: &CharacterClips, c: &Controls, seconds: f32) -> MotionStep {
        self.react(clips, c);
        self.follow_aim(c);
        let Some((def, clip)) = self.index.as_deref().and_then(|index| clips.get(index)) else {
            let air = self.state == State::Fall;
            let velocity = air.then(|| self.velocity(&ClipDef::default(), 1.0, c));
            return MotionStep {
                velocity: velocity.unwrap_or_default(),
                ..Default::default()
            };
        };
        let (def, clip) = (def.clone(), clip.clone());
        let length = clip_seconds(&clip);
        let rate = clip.bones.first().map_or(0.0, |bone| bone.fps);
        let script = match self.state {
            State::Dive => clips.table.get(&self.script).unwrap_or(&def),
            _ => &def,
        };
        let before = self.time;
        let after = self.time() + seconds;
        let velocity = self.velocity(&def, length, c);
        let mut messages = due(script, rate, before, after.min(length));
        self.time = after;
        if after >= length {
            match self.state {
                State::Ground => {
                    self.time = after - length;
                    messages.extend(due(script, rate, -1.0, self.time));
                }
                State::Crouch | State::Fall | State::Dive => self.time = length * 0.999,
                State::Dead => {
                    self.time = length * 0.999;
                    self.death_over = true;
                }
                _ => {}
            }
            self.finish(c);
        } else if self.state == State::Dive
            && clips.abort_dive_frame > 0
            && after * rate >= clips.abort_dive_frame as f32
        {
            self.finish(c);
        }
        MotionStep {
            clip: Some(clip),
            velocity,
            messages,
        }
    }
}

fn hands(two_handed: bool) -> &'static str {
    if two_handed {
        "2_"
    } else {
        ""
    }
}

/// `(x, z)` of a `(right, up, forward)` vector for a body turned `yaw`.
fn turn([right, _, forward]: [f32; 3], yaw: f32) -> [f32; 2] {
    let (sin, cos) = yaw.sin_cos();
    [right * cos + forward * sin, forward * cos - right * sin]
}

fn direction_name(forward: i32, right: i32) -> &'static str {
    match (forward.signum(), right.signum()) {
        (1, 0) | (0, 0) => "FORWARD",
        (-1, 0) => "BACKWARD",
        (0, 1) => "RIGHT",
        (0, _) => "LEFT",
        (1, 1) => "FORWARDRIGHT",
        (1, _) => "FORWARDLEFT",
        (_, 1) => "BACKWARDRIGHT",
        _ => "BACKWARDLEFT",
    }
}

/// The nearest of eight directions to a unit `(forward, right)`.
fn octant_name((forward, right): (f32, f32)) -> &'static str {
    let side = |share: f32| {
        if share.abs() < OCTANT {
            0
        } else {
            share.signum() as i32
        }
    };
    direction_name(side(forward), side(right))
}

/// The nearest of the four directions a get-up clip exists for.
fn quarter_name((forward, right): (f32, f32)) -> &'static str {
    if forward.abs() >= right.abs() {
        direction_name(forward.signum() as i32, 0)
    } else {
        direction_name(0, right.signum() as i32)
    }
}

/// Clip speed at `time` along the waypoints a path clip names, taken as evenly spaced in time.
fn path_velocity(def: &ClipDef, length: f32, time: f32) -> [f32; 3] {
    let Some([first, second]) = def.middle else {
        return def.velocity(length);
    };
    if time >= length * 0.999 {
        return [0.0; 3];
    }
    let points = [[0.0; 3], first, second, def.end];
    let leg = ((time / length * 3.0) as usize).min(2);
    let (from, to) = (points[leg], points[leg + 1]);
    std::array::from_fn(|c| (to[c] - from[c]) * 3.0 / length)
}

/// The messages of `def` on frames after `from` and up to `to`, in seconds at `rate` frames per
/// second, in frame order.
fn due(def: &ClipDef, rate: f32, from: f32, to: f32) -> Vec<String> {
    let mut passed: Vec<_> = def
        .messages
        .iter()
        .filter(|(frame, _)| {
            let at = *frame as f32 / rate.max(1.0);
            at > from && at <= to
        })
        .collect();
    passed.sort_by_key(|(frame, _)| *frame);
    passed.into_iter().map(|(_, text)| text.clone()).collect()
}

/// `def` sends `GM_EnableBulletTime( false )`.
pub fn stops_bullet_time(def: &ClipDef) -> bool {
    def.messages
        .iter()
        .any(|(_, text)| clip_effect(text) == Some(ClipEffect::BulletTimeOff))
}

/// The sounds of a reload clip's `(frame, message)`s, in seconds at `rate` frames per second.
/// `C_Reload` is left out: the inventory times the refill itself.
pub fn reload_clip_sounds(def: &ClipDef, rate: f32) -> Vec<(f32, String)> {
    let sound =
        |(_, text): &&(u32, String)| matches!(clip_effect(text), Some(ClipEffect::Sound { .. }));
    let timed = |(frame, text): &(u32, String)| (*frame as f32 / rate.max(1.0), text.clone());
    def.messages.iter().filter(sound).map(timed).collect()
}

/// What a clip message does to the player.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipEffect {
    /// `C_Jump`: metres per second upward.
    Jump(f32),
    /// `A_Play3DSound` or `A_PlaySound`.
    Sound { catalog: String, cue: String },
    /// `P_CreateProjectileToBone`.
    Projectile(String),
    /// `C_Reload`.
    Reload,
    /// `GM_EnableBulletTime( false )`.
    BulletTimeOff,
    /// Anything the scene answers itself.
    Script,
}

/// The effect of a clip message, or `None` when nothing here answers it.
pub fn clip_effect(text: &str) -> Option<ClipEffect> {
    let message = crate::scene::Message::parse(text)?;
    let arg = |index: usize| message.args.get(index).map(|arg| arg.to_string());
    match message.verb.to_ascii_lowercase().as_str() {
        "c_jump" => Some(ClipEffect::Jump(arg(0)?.parse().ok()?)),
        "a_play3dsound" | "a_playsound" => Some(ClipEffect::Sound {
            catalog: arg(0)?,
            cue: arg(1)?,
        }),
        "p_createprojectiletobone" => Some(ClipEffect::Projectile(arg(0)?)),
        "c_reload" => Some(ClipEffect::Reload),
        "gm_enablebullettime" => arg(0)
            .filter(|on| on.eq_ignore_ascii_case("false"))
            .map(|_| ClipEffect::BulletTimeOff),
        "gm_changegamespeed" => Some(ClipEffect::Script),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clips::ClipTable;
    use crate::kf2::{AnimBone, Kf2Anim};

    /// A clip of `frames` at 10 frames per second.
    fn anim(frames: u32) -> Arc<Kf2Anim> {
        Arc::new(Kf2Anim {
            bones: vec![AnimBone {
                name: "pelvis".to_string(),
                parent: None,
                duration: frames,
                fps: 10.0,
                keys: Vec::new(),
            }],
        })
    }

    const TABLE: &str = "\
        [Animation] Index = CHARANIM_RUN; Filename = run;\n\
        [Movement] EndPosition = (0,0,4);\n\
        [Message] Frame = 2; String = \"this->P_CreateProjectileToBone(dummy_footstep_right,1,foot-r);\";\n\
        [Animation] Index = CHARANIM_WALKBACKWARD; Filename = fall;\n\
        [Movement] EndPosition = (0,0,-3);\n\
        [Animation] Index = CHARANIM_RELOADBERETTA; Filename = land;\n\
        [Message] Frame = 0; String = \"maxpayne_gamemode->gm_enablebullettime( false );\";\n\
        [Animation] Index = CHARANIM_STRAFERIGHT; Filename = run;\n\
        [Movement] EndPosition = (4,0,0);\n\
        [Animation] Index = CHARANIM_CROUCH; Filename = crouch;\n\
        [Animation] Index = CHARANIM_JUMP; Filename = jump;\n\
        [Message] Frame = 1; String = \"this->C_Jump( 7.5 );\";\n\
        [Animation] Index = CHARANIM_JUMPFORWARD; Filename = jump;\n\
        [Movement] EndPosition = (0,0,1);\n\
        [Animation] Index = CHARANIM_FALL; Filename = fall;\n\
        [Animation] Index = CHARANIM_LAND; Filename = land;\n\
        [Animation] Index = CHARANIM_RANDOMDEATH1; Filename = land;\n\
        [Movement] EndPosition = (0,0,-1);\n\
        [Animation] Index = CHARANIM_DODGEFORWARD; Filename = jump;\n\
        [Movement] EndPosition = (0,0,2);\n\
        [Animation] Index = CHARANIM_SHOOTDODGEFORWARD; Filename = dive;\n\
        [Movement] EndPosition = (0,0,6); MiddlePosition1 = (0,0,3); MiddlePosition2 = (0,0,5);\n\
        [Message] Frame = 1; String = \"this->C_Reload();\";\n\
        [Animation] Index = CHARANIM_SHOOTDODGEBACKWARD; Filename = back;\n\
        [Movement] EndPosition = (0,0,-6); MiddlePosition1 = (0,0,-3); MiddlePosition2 = (0,0,-5);\n\
        [Animation] Index = CHARANIM_SHOOTDODGERIGHT; Filename = side;\n\
        [Animation] Index = CHARANIM_SHOOTDODGELEFT; Filename = side;\n\
        [Animation] Index = CHARANIM_SHOOTDODGEFORWARD_GETUP; Filename = up;\n\
        [Movement] EndPosition = (0,0,1);\n\
        [Animation] Index = CHARANIM_SHOOTDODGEBACKWARD_GETUP; Filename = up2;\n";

    fn clips() -> CharacterClips {
        let files = [
            ("run", 10),
            ("crouch", 10),
            ("jump", 5),
            ("fall", 20),
            ("land", 2),
            ("dive", 30),
            ("back", 30),
            ("side", 30),
            ("up", 5),
            ("up2", 5),
        ];
        CharacterClips {
            table: ClipTable::parse(TABLE, ""),
            anims: files
                .iter()
                .map(|(name, frames)| (name.to_string(), anim(*frames)))
                .collect(),
            airborne_speed: 2.0,
            abort_dive_frame: 20,
        }
    }

    fn grounded() -> Controls {
        Controls {
            grounded: true,
            airborne_speed: 2.0,
            ..Default::default()
        }
    }

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn standing_still_plays_no_clip() {
        let step = PlayerMotion::default().step(&clips(), &grounded(), 0.1);
        assert!(step.clip.is_none());
        assert_eq!(step.velocity, [0.0; 2]);
    }

    #[test]
    fn a_held_key_moves_at_the_clip_speed_along_the_yaw() {
        let c = Controls {
            forward: 1,
            ..grounded()
        };
        let step = PlayerMotion::default().step(&clips(), &c, 0.1);
        assert!(near(step.velocity, [0.0, 4.0]));
        let c = Controls {
            right: 1,
            ..grounded()
        };
        let step = PlayerMotion::default().step(&clips(), &c, 0.1);
        assert!(near(step.velocity, [4.0, 0.0]));
    }

    #[test]
    fn a_run_clip_gives_its_speed_whatever_its_length() {
        let c = Controls {
            forward: -1,
            ..grounded()
        };
        let step = PlayerMotion::default().step(&clips(), &c, 0.1);
        assert!(near(step.velocity, [0.0, -3.0]));
    }

    #[test]
    fn a_clip_that_turns_bullet_time_off_says_so() {
        let clips = clips();
        let reload = clips.table.get("CHARANIM_RELOADBERETTA").expect("reload");
        assert!(stops_bullet_time(reload));
        let run = clips.table.get("CHARANIM_RUN").expect("run");
        assert!(!stops_bullet_time(run));
    }

    #[test]
    fn a_looping_run_says_its_message_on_every_lap() {
        let c = Controls {
            forward: 1,
            ..grounded()
        };
        let clips = clips();
        let mut motion = PlayerMotion::default();
        let said: usize = (0..20)
            .map(|_| motion.step(&clips, &c, 0.1).messages.len())
            .sum();
        assert_eq!(said, 2);
    }

    #[test]
    fn a_jump_waits_for_its_frame_then_falls_and_lands() {
        let clips = clips();
        let mut motion = PlayerMotion::default();
        let jump = Controls {
            jump: true,
            ..grounded()
        };
        assert!(motion.step(&clips, &jump, 0.05).messages.is_empty());
        let said = motion.step(&clips, &grounded(), 0.1).messages;
        assert_eq!(said, ["this->C_Jump( 7.5 );"]);
        let air = Controls {
            grounded: false,
            ..grounded()
        };
        for _ in 0..6 {
            motion.step(&clips, &air, 0.1);
        }
        assert_eq!(motion.state, State::Fall);
        motion.step(&clips, &grounded(), 0.05);
        assert_eq!(motion.state, State::Land);
        motion.step(&clips, &grounded(), 0.3);
        assert_eq!(motion.state, State::Ground);
    }

    #[test]
    fn a_forward_jump_adds_the_keys_to_its_clip_in_the_air() {
        let c = Controls {
            forward: 1,
            jump: true,
            ..grounded()
        };
        let step = PlayerMotion::default().step(&clips(), &c, 0.1);
        assert!(near(step.velocity, [0.0, 2.0 + 2.0]));
    }

    #[test]
    fn walking_off_a_ledge_falls() {
        let c = Controls {
            grounded: false,
            ..grounded()
        };
        let mut motion = PlayerMotion::default();
        motion.step(&clips(), &c, 0.1);
        assert_eq!(motion.state, State::Fall);
    }

    #[test]
    fn crouching_holds_the_last_frame_until_released_or_moved() {
        let clips = clips();
        let mut motion = PlayerMotion::default();
        let c = Controls {
            crouch: true,
            ..grounded()
        };
        for _ in 0..30 {
            motion.step(&clips, &c, 0.1);
        }
        assert!(motion.crouching());
        assert!((0.9..1.0).contains(&motion.time()));
        motion.step(&clips, &grounded(), 0.1);
        assert!(!motion.crouching());
    }

    #[test]
    fn a_dive_follows_its_waypoints_along_the_heading_it_set_off_on() {
        let clips = clips();
        let mut motion = PlayerMotion::default();
        let c = Controls {
            forward: 1,
            dodge: Some(DodgeKind::Dive),
            ..grounded()
        };
        let step = motion.step(&clips, &c, 0.5);
        assert!(near(step.velocity, [0.0, 3.0]));
        let turned = Controls {
            yaw: std::f32::consts::FRAC_PI_2,
            ..grounded()
        };
        let step = motion.step(&clips, &turned, 0.6);
        assert!(near(step.velocity, [0.0, 3.0]));
        let step = motion.step(&clips, &turned, 0.1);
        assert!(near(step.velocity, [0.0, 2.0]));
    }

    #[test]
    fn a_dive_changes_clip_with_the_crosshair_but_not_its_clock() {
        let clips = clips();
        let mut motion = PlayerMotion::default();
        let c = Controls {
            forward: 1,
            dodge: Some(DodgeKind::Dive),
            ..grounded()
        };
        motion.step(&clips, &c, 0.5);
        let behind = Controls {
            yaw: std::f32::consts::PI,
            ..grounded()
        };
        motion.step(&clips, &behind, 0.1);
        assert_eq!(motion.index.as_deref(), Some("CHARANIM_SHOOTDODGEBACKWARD"));
        assert!((motion.time() - 0.6).abs() < 1e-5);
        let side = Controls {
            yaw: -std::f32::consts::FRAC_PI_2,
            ..grounded()
        };
        motion.step(&clips, &side, 0.1);
        assert_eq!(motion.index.as_deref(), Some("CHARANIM_SHOOTDODGERIGHT"));
    }

    #[test]
    fn a_dive_says_the_forward_clips_messages_whichever_clip_shows() {
        let clips = clips();
        let mut motion = PlayerMotion::default();
        let c = Controls {
            forward: -1,
            dodge: Some(DodgeKind::Dive),
            ..grounded()
        };
        let step = motion.step(&clips, &c, 0.2);
        assert_eq!(step.messages, ["this->C_Reload();"]);
    }

    #[test]
    fn a_dive_ends_on_the_ground_with_the_get_up_of_its_side() {
        let clips = clips();
        let mut motion = PlayerMotion::default();
        let c = Controls {
            forward: 1,
            dodge: Some(DodgeKind::Dive),
            ..grounded()
        };
        motion.step(&clips, &c, 0.1);
        let air = Controls {
            grounded: false,
            ..grounded()
        };
        motion.step(&clips, &air, 3.0);
        assert!(motion.diving(), "still in the air");
        let behind = Controls {
            yaw: std::f32::consts::PI,
            ..grounded()
        };
        motion.step(&clips, &behind, 0.1);
        assert_eq!(
            motion.index.as_deref(),
            Some("CHARANIM_SHOOTDODGEBACKWARD_GETUP")
        );
        motion.step(&clips, &grounded(), 1.0);
        assert_eq!(motion.state, State::Ground);
    }

    #[test]
    fn a_dive_on_the_ground_gets_up_at_its_abort_frame() {
        let clips = clips();
        let mut motion = PlayerMotion::default();
        let c = Controls {
            forward: 1,
            dodge: Some(DodgeKind::Dive),
            ..grounded()
        };
        motion.step(&clips, &c, 1.9);
        assert!(motion.diving());
        motion.step(&clips, &grounded(), 0.2);
        assert_eq!(
            motion.index.as_deref(),
            Some("CHARANIM_SHOOTDODGEFORWARD_GETUP")
        );
    }

    #[test]
    fn a_roll_takes_its_clip_along_its_heading_and_returns_to_the_ground() {
        let clips = clips();
        let mut motion = PlayerMotion::default();
        let c = Controls {
            forward: 1,
            dodge: Some(DodgeKind::Roll),
            ..grounded()
        };
        let step = motion.step(&clips, &c, 0.1);
        assert!(near(step.velocity, [0.0, 4.0]));
        motion.step(&clips, &grounded(), 1.0);
        assert_eq!(motion.state, State::Ground);
    }

    #[test]
    fn a_jump_backwards_or_sideways_rolls_as_no_jump_clip_faces_there() {
        for (forward, right, index) in [
            (-1, 0, "CHARANIM_DODGEBACKWARD"),
            (0, -1, "CHARANIM_DODGELEFT"),
            (0, 1, "CHARANIM_DODGERIGHT"),
            (-1, 1, "CHARANIM_DODGEBACKWARDRIGHT"),
        ] {
            let c = Controls {
                forward,
                right,
                jump: true,
                ..grounded()
            };
            let mut motion = PlayerMotion::default();
            motion.step(&clips(), &c, 0.01);
            assert_eq!(motion.state, State::Roll, "{index}");
            assert_eq!(motion.index.as_deref(), Some(index));
        }
    }

    #[test]
    fn a_jump_forwards_or_on_the_spot_stays_a_jump() {
        for forward in [0, 1] {
            let c = Controls {
                forward,
                jump: true,
                ..grounded()
            };
            let mut motion = PlayerMotion::default();
            motion.step(&clips(), &c, 0.01);
            assert!(matches!(motion.state, State::Jump { .. }));
        }
    }

    #[test]
    fn max_falls_along_his_death_clip_and_stays_down() {
        let clips = clips();
        let mut motion = PlayerMotion::default();
        motion.die(&clips, 0.0);
        let step = motion.step(&clips, &grounded(), 0.05);
        assert!(step.clip.is_some());
        assert!(!motion.death_over());
        let jump = Controls {
            jump: true,
            ..grounded()
        };
        motion.step(&clips, &jump, 1.0);
        assert!(motion.death_over());
        assert_eq!(motion.state, State::Dead);
        motion.die(&clips, 0.0);
        assert!(motion.death_over(), "a second death changes nothing");
    }

    #[test]
    fn a_clips_messages_come_with_their_seconds() {
        let said = clips().timed_messages("CHARANIM_SHOOTDODGEFORWARD");
        assert_eq!(said, [(0.1, "this->C_Reload();".to_string())]);
        assert!(clips().timed_messages("CHARANIM_NOTHING").is_empty());
    }

    #[test]
    fn a_shotgun_reload_keeps_the_sound_of_its_character_clip() {
        let def = ClipDef {
            messages: vec![
                (2, "RightHandWeapon->WS_Animate(4);".to_string()),
                (
                    17,
                    "this->A_Play3DSound( weapons, Reload_PumpShotgun, gun);".to_string(),
                ),
                (17, "this->C_Reload();".to_string()),
            ],
            ..Default::default()
        };
        let sounds = reload_clip_sounds(&def, 10.0);
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].0, 1.7);
        assert!(sounds[0].1.contains("Reload_PumpShotgun"));
    }

    #[test]
    fn a_dodge_needs_a_direction() {
        let c = Controls {
            dodge: Some(DodgeKind::Dive),
            ..grounded()
        };
        let mut motion = PlayerMotion::default();
        motion.step(&clips(), &c, 0.1);
        assert_eq!(motion.state, State::Ground);
    }

    #[test]
    fn clip_messages_become_effects() {
        assert_eq!(
            clip_effect("this->C_Jump(  3   );"),
            Some(ClipEffect::Jump(3.0))
        );
        assert_eq!(
            clip_effect("this->A_Play3DSound ( characters, jump_medium, pelvis );"),
            Some(ClipEffect::Sound {
                catalog: "characters".to_string(),
                cue: "jump_medium".to_string()
            })
        );
        assert_eq!(
            clip_effect("this->P_Createprojectiletobone(dummy_footstep_right,1,foot-r);"),
            Some(ClipEffect::Projectile("dummy_footstep_right".to_string()))
        );
        assert_eq!(
            clip_effect("maxpayne_gamemode->gm_enablebullettime( false );"),
            Some(ClipEffect::BulletTimeOff)
        );
        assert_eq!(clip_effect("this->C_SetDodgeImmortal( true );"), None);
    }
}
