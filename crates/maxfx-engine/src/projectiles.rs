//! Projectiles that really fly: grenades, molotovs and 40 mm rounds. A bullet has
//! `GravityMultiplier = 0` and stays a ray; everything here falls, bounces and explodes as its
//! `database/projectiles/<name>.txt` says.

/// How far off a surface a projectile stays, so its next move does not start inside it.
const SURFACE_GAP: f32 = 0.01;

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// One `P_Explosion` of a projectile's `PROJECTILEANIM_EXPLODE` clip.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Blast {
    /// Clip frame of the message.
    pub frame: u32,
    /// The same in seconds, once the clip's rate is known.
    pub at: f32,
    /// `hotspotDistance`: full damage up to here.
    pub hotspot: f32,
    /// `falloffDistance`: nothing past here.
    pub falloff: f32,
    pub hotspot_damage: f32,
    pub falloff_damage: f32,
}

impl Blast {
    /// Damage at `distance`, interpolated from the hotspot to the falloff.
    pub fn damage_at(&self, distance: f32) -> f32 {
        if distance > self.falloff {
            return 0.0;
        }
        let span = (self.falloff - self.hotspot).max(1e-6);
        let along = ((distance - self.hotspot) / span).clamp(0.0, 1.0);
        self.hotspot_damage + (self.falloff_damage - self.hotspot_damage) * along
    }
}

/// What `database/projectiles/<name>.txt` says about a flying projectile.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlyingDef {
    /// `Direction`, in the thrower's aim: x right, y up, z forward.
    pub direction: [f32; 3],
    pub accuracy: f32,
    /// `Speed`, metres per second.
    pub speed: f32,
    /// `GravityMultiplier`.
    pub gravity: f32,
    /// `RicochetMaximumAngle`, degrees off the surface. 0 never bounces.
    pub ricochet_angle: f32,
    pub ricochet_speed: f32,
    /// `RicochetMinimumSpeed`: slower than this it stops bouncing.
    pub ricochet_minimum: f32,
    /// `DamagesCharacter`: off for spent cases and clips, which fall but are no weapon.
    pub damages_character: bool,
    /// `CollisionExplosion`.
    pub explodes_on_contact: bool,
    /// `ExplosionDelay` of a `DelayedExplosion`: seconds from the throw.
    pub fuse: Option<f32>,
    pub decal: Option<String>,
    pub blasts: Vec<Blast>,
    /// First `[LOD]` `ExportData`, relative to the projectile's folder, until the loader resolves it.
    pub model: Option<String>,
    /// `A_Play3DSound` of `PROJECTILEANIM_RICOCHET`, a `sounds/projectiles.txt` cue.
    pub bounce_sound: Option<String>,
    /// `PS_StartEffect` of its explosion. Some leave it to their decal block.
    pub effect: Option<String>,
    /// `A_PlayFloating3DSound` of its explosion.
    pub sound: Option<String>,
    /// `CAM_AnimateInPlace` of its explosion.
    pub shake: Option<String>,
    /// `P_CreateProjectile` of its explosion, with the count.
    pub frags: Vec<(FlyingDef, u32)>,
    /// `P_CreateProjectile` names of the explosion, until the loader resolves them.
    pub frag_names: Vec<(String, u32)>,
    /// Filename of `PROJECTILEANIM_EXPLODE`, relative to `database\\projectiles\\`.
    pub explode_clip: Option<String>,
}

impl FlyingDef {
    /// `None` for what does not fall, which is every bullet.
    pub fn parse(defines: &str, text: &str) -> Option<Self> {
        let sections = crate::hud::parse_sections(&format!("{defines}\n{text}"));
        let key = |name: &str| sections.iter().find_map(|s| s.text(name));
        let number = |name: &str| key(name).and_then(|value| value.parse().ok());
        let flag = |name: &str| key(name).is_some_and(|value| value.eq_ignore_ascii_case("true"));
        let gravity = number("gravitymultiplier").filter(|gravity: &f32| *gravity > 0.0)?;
        let clip = |index: &str| -> Vec<&str> {
            text.lines()
                .skip_while(|line| !line.contains(index))
                .skip(1)
                .take_while(|line| !line.contains("[Animation]"))
                .collect()
        };
        let explode = clip("PROJECTILEANIM_EXPLODE");
        let ricochet = clip("PROJECTILEANIM_RICOCHET");
        Some(Self {
            direction: key("direction").and_then(parse_vector).unwrap_or_default(),
            accuracy: number("accuracy").unwrap_or(0.0),
            speed: number("speed").unwrap_or(0.0),
            gravity,
            ricochet_angle: number("ricochetmaximumangle").unwrap_or(0.0),
            ricochet_speed: number("ricochetspeedmultiplier").unwrap_or(0.0),
            ricochet_minimum: number("ricochetminimumspeed").unwrap_or(0.0),
            damages_character: flag("damagescharacter"),
            explodes_on_contact: flag("collisionexplosion"),
            fuse: flag("delayedexplosion")
                .then(|| number("explosiondelay"))
                .flatten(),
            decal: crate::hud::weapon_cue_line(text, "D_CreateDecal", 0),
            blasts: explode.iter().copied().filter_map(parse_blast).collect(),
            model: text.lines().find_map(|line| {
                let lower = line.to_ascii_lowercase();
                let at = lower.find("exportdata")? + "exportdata".len();
                let file = line[at..].split_once('=')?.1.split(';').next()?;
                Some(file.trim().to_string())
            }),
            bounce_sound: clip_cue(&ricochet, "A_Play3DSound", 1),
            effect: clip_cue(&explode, "PS_StartEffect", 0),
            sound: clip_cue(&explode, "A_PlayFloating3DSound", 1),
            shake: clip_cue(&explode, "CAM_AnimateInPlace", 0),
            frags: Vec::new(),
            frag_names: explode.iter().copied().filter_map(parse_frag).collect(),
            explode_clip: text
                .lines()
                .find(|line| line.contains("PROJECTILEANIM_EXPLODE"))
                .and_then(|line| line.split_once("Filename =")?.1.split(';').next())
                .map(|file| file.trim().to_string()),
        })
    }
}

/// Argument `arg` of the first `verb` among a clip's `[Message]` lines.
fn clip_cue(lines: &[&str], verb: &str, arg: usize) -> Option<String> {
    lines
        .iter()
        .find_map(|line| crate::hud::weapon_cue_line(line, verb, arg))
}

/// `( x, y, z )`.
fn parse_vector(text: &str) -> Option<[f32; 3]> {
    let text = text.trim().strip_prefix('(')?.strip_suffix(')')?;
    let mut parts = text.split(',').map(|part| part.trim().parse().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

/// A `[Message]` line holding a `P_Explosion`.
fn parse_blast(line: &str) -> Option<Blast> {
    let frame = line.split_once("Frame =")?.1.split(';').next()?;
    let arg = |index| {
        crate::hud::weapon_cue_line(line, "P_Explosion", index)?
            .parse()
            .ok()
    };
    Some(Blast {
        frame: frame.trim().parse().ok()?,
        at: 0.0,
        hotspot: arg(0)?,
        falloff: arg(1)?,
        hotspot_damage: arg(2)?,
        falloff_damage: arg(3)?,
    })
}

/// A `[Message]` line holding a `P_CreateProjectile`.
fn parse_frag(line: &str) -> Option<(String, u32)> {
    let name = crate::hud::weapon_cue_line(line, "P_CreateProjectile", 0)?;
    let count = crate::hud::weapon_cue_line(line, "P_CreateProjectile", 1)?;
    Some((name, count.parse().ok()?))
}

/// What a flying projectile did during one step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flight {
    Flying,
    Bounced,
    /// Where it went off.
    Exploded([f32; 3]),
}

/// One projectile in the air.
#[derive(Debug, Clone, PartialEq)]
pub struct Flyer {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub age: f32,
    /// Lying still, waiting for its fuse.
    pub resting: bool,
    /// Normal of the surface it touched last, zero before it has touched one.
    pub normal: [f32; 3],
}

impl Flyer {
    /// Advance by `seconds`. `surface` gives the fraction of a move that hits something, and the
    /// normal there.
    pub fn step(
        &mut self,
        def: &FlyingDef,
        gravity: [f32; 3],
        seconds: f32,
        surface: impl Fn([f32; 3], [f32; 3]) -> Option<(f32, [f32; 3])>,
    ) -> Flight {
        self.age += seconds;
        if def.fuse.is_some_and(|fuse| self.age >= fuse) {
            return Flight::Exploded(self.pos);
        }
        if self.resting {
            return Flight::Flying;
        }
        self.vel = std::array::from_fn(|c| self.vel[c] + gravity[c] * def.gravity * seconds);
        let delta = self.vel.map(|c| c * seconds);
        let Some((t, normal)) = surface(self.pos, delta) else {
            self.pos = std::array::from_fn(|c| self.pos[c] + delta[c]);
            return Flight::Flying;
        };
        let hit: [f32; 3] = std::array::from_fn(|c| self.pos[c] + delta[c] * t);
        self.normal = normal;
        let speed = dot(self.vel, self.vel).sqrt();
        let into = dot(self.vel, normal);
        // HYPOTHESIS: the angle is measured off the surface, so 90 takes every hit and 0 none.
        let angle = (into.abs() / speed.max(1e-6)).asin().to_degrees();
        if def.ricochet_angle <= 0.0 || angle > def.ricochet_angle {
            if def.explodes_on_contact {
                return Flight::Exploded(hit);
            }
            self.rest(hit, normal);
            return Flight::Flying;
        }
        let bounced: [f32; 3] =
            std::array::from_fn(|c| (self.vel[c] - 2.0 * into * normal[c]) * def.ricochet_speed);
        // HYPOTHESIS: `RicochetMinimumSpeed` is tested after the bounce has slowed it.
        if dot(bounced, bounced).sqrt() < def.ricochet_minimum {
            if def.explodes_on_contact {
                return Flight::Exploded(hit);
            }
            self.rest(hit, normal);
            return Flight::Flying;
        }
        self.vel = bounced;
        self.pos = std::array::from_fn(|c| hit[c] + normal[c] * SURFACE_GAP);
        Flight::Bounced
    }

    fn rest(&mut self, hit: [f32; 3], normal: [f32; 3]) {
        self.resting = true;
        self.vel = [0.0; 3];
        self.pos = std::array::from_fn(|c| hit[c] + normal[c] * SURFACE_GAP);
    }
}

/// `direction` of a projectile file, turned from the thrower's aim into the world.
pub fn launch_direction(direction: [f32; 3], aim: [f32; 3]) -> [f32; 3] {
    let flat = (aim[0] * aim[0] + aim[2] * aim[2]).sqrt().max(1e-6);
    let right = [aim[2] / flat, 0.0, -aim[0] / flat];
    let up = [
        aim[1] * right[2],
        aim[2] * right[0] - aim[0] * right[2],
        -aim[1] * right[0],
    ];
    let turned: [f32; 3] = std::array::from_fn(|c| {
        right[c] * direction[0] + up[c] * direction[1] + aim[c] * direction[2]
    });
    let length = dot(turned, turned).sqrt().max(1e-6);
    turned.map(|c| c / length)
}

/// How far `point` is from a standing character, whose axis rises `height` from `feet`.
pub fn distance_to_character(point: [f32; 3], feet: [f32; 3], height: f32) -> f32 {
    let nearest = [feet[0], point[1].clamp(feet[1], feet[1] + height), feet[2]];
    let gap: [f32; 3] = std::array::from_fn(|c| point[c] - nearest[c]);
    dot(gap, gap).sqrt()
}

/// An explosion running its clip: each `P_Explosion` goes off at its frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Explosion {
    pub pos: [f32; 3],
    pub blasts: Vec<Blast>,
    pub age: f32,
}

impl Explosion {
    /// The blasts due after `seconds` more. The explosion is over once none are left.
    pub fn step(&mut self, seconds: f32) -> Vec<Blast> {
        self.age += seconds;
        let age = self.age;
        let (due, later) = self.blasts.iter().partition(|blast| blast.at <= age);
        self.blasts = later;
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRENADE: &str = "[LOD] { [Geometry] exportdata = ..\\..\\weapons\\grenade\\grenade_lod1.kf2; Distance = 1; }\n[Animationset]\n{\n\t[Animation]\n\tIndex = PROJECTILEANIM_DEFAULT; Filename = a.kf2;\n\t[Animation]\n\tIndex = PROJECTILEANIM_RICOCHET; Filename = b.kf2;\n\t[Properties]\n\t{\n\t\t[Message] Frame = 0; String = \"this->A_Play3DSound( projectiles, Bullethit_Grenade, \\\"\\\");\";\n\t}\n\t[Animation]\n\tIndex = PROJECTILEANIM_EXPLODE; Filename = c.kf2;\n\t[Properties]\n\t{\n\t\t[Message] Frame = 0; String = \"this->CAM_AnimateInPlace( explosion );\";\n\t\t[Message] Frame = 0; String =\"this->PS_StartEffect ( explosion_grenade, \\\"\\\" );\";\n\t\t[Message] Frame = 0; String =\"this->A_PlayFloating3DSound( projectiles, bullethit_40mm, \\\"\\\");\";\n\t\t[Message] Frame = 0; String = \"this->D_CreateDecal( grenade );\";\n\t\t[Message] Frame = 0; String = \"this->P_Explosion( 2.0,  5.0, 50, 10, FALSE, TRUE, FALSE, FALSE );\";\n\t\t[Message] Frame = 1; String = \"this->P_Explosion( 2.0, 10.0, 20, 5, FALSE, TRUE, FALSE, FALSE );\";\n\t}\n}\n[Attributes]\nDirection \t= ( 0, 0.1, 1 );\nAccuracy \t= 20;\nSpeed \t\t= 20;\nRicochetMaximumAngle \t= 90;\nRicochetSpeedMultiplier = 0.2;\nRicochetMinimumSpeed \t= 3;\nCollisionExplosion \t\t= FALSE;\nDelayedExplosion \t\t= TRUE;\nExplosionDelay\t\t\t= 2.0;\nGravityMultiplier\t\t= 0.5;\n";

    fn grenade() -> FlyingDef {
        FlyingDef::parse("", GRENADE).expect("grenade flies")
    }

    #[test]
    fn a_grenade_file_gives_its_flight_and_its_blasts() {
        let def = grenade();
        assert_eq!(def.direction, [0.0, 0.1, 1.0]);
        assert_eq!((def.speed, def.gravity, def.fuse), (20.0, 0.5, Some(2.0)));
        assert_eq!(def.ricochet_angle, 90.0);
        assert!(!def.explodes_on_contact);
        assert_eq!(def.decal.as_deref(), Some("grenade"));
        assert_eq!(def.blasts.len(), 2);
        assert_eq!(def.blasts[1].frame, 1);
        assert_eq!(def.blasts[1].falloff, 10.0);
    }

    #[test]
    fn a_grenade_file_names_what_it_looks_and_sounds_like() {
        let def = grenade();
        let model = def.model.as_deref();
        assert_eq!(model, Some(r"..\..\weapons\grenade\grenade_lod1.kf2"));
        assert_eq!(def.bounce_sound.as_deref(), Some("Bullethit_Grenade"));
        assert_eq!(def.effect.as_deref(), Some("explosion_grenade"));
        assert_eq!(def.sound.as_deref(), Some("bullethit_40mm"));
        assert_eq!(def.shake.as_deref(), Some("explosion"));
    }

    #[test]
    fn what_does_not_fall_is_a_bullet() {
        let bullet = "[Attributes]\nSpeed = 50;\nGravityMultiplier = 0.0;\n";
        assert_eq!(FlyingDef::parse("", bullet), None);
    }

    #[test]
    fn a_blast_hurts_fully_in_its_hotspot_and_fades_to_its_falloff() {
        let blast = Blast {
            hotspot: 2.0,
            falloff: 6.0,
            hotspot_damage: 50.0,
            falloff_damage: 10.0,
            ..Default::default()
        };
        assert_eq!(blast.damage_at(1.0), 50.0);
        assert_eq!(blast.damage_at(4.0), 30.0);
        assert_eq!(blast.damage_at(6.5), 0.0);
    }

    #[test]
    fn a_throw_rises_above_the_aim_as_its_file_says() {
        let dir = launch_direction([0.0, 0.4, 1.0], [1.0, 0.0, 0.0]);
        let length = (1.0f32 + 0.16).sqrt();
        assert!((dir[0] - 1.0 / length).abs() < 1e-5, "{dir:?}");
        assert!((dir[1] - 0.4 / length).abs() < 1e-5, "{dir:?}");
        assert!(dir[2].abs() < 1e-5, "{dir:?}");
    }

    #[test]
    fn a_throw_aimed_upward_rises_square_to_the_aim() {
        let dir = launch_direction([0.0, 1.0, 0.0], [0.6, 0.8, 0.0]);
        assert!((dir[0] + 0.8).abs() < 1e-5, "{dir:?}");
        assert!((dir[1] - 0.6).abs() < 1e-5, "{dir:?}");
    }

    #[test]
    fn a_character_is_as_near_as_the_closest_point_of_its_axis() {
        assert_eq!(distance_to_character([3.0, 1.0, 0.0], [0.0; 3], 2.0), 3.0);
        assert_eq!(distance_to_character([0.0, 5.0, 0.0], [0.0; 3], 2.0), 3.0);
    }

    #[test]
    fn blasts_go_off_once_each_at_their_times() {
        let blast = |at| Blast {
            at,
            ..Default::default()
        };
        let mut running = Explosion {
            pos: [0.0; 3],
            blasts: vec![blast(0.0), blast(0.5)],
            age: 0.0,
        };
        assert_eq!(running.step(0.1).len(), 1);
        assert_eq!(running.step(0.1).len(), 0);
        assert_eq!(running.step(0.4).len(), 1);
        assert!(running.blasts.is_empty());
    }

    fn flyer(vel: [f32; 3]) -> Flyer {
        Flyer {
            pos: [0.0, 1.0, 0.0],
            vel,
            age: 0.0,
            resting: false,
            normal: [0.0; 3],
        }
    }

    /// The floor at y = 0.
    fn floor(from: [f32; 3], delta: [f32; 3]) -> Option<(f32, [f32; 3])> {
        let end = from[1] + delta[1];
        (end < 0.0).then(|| (from[1] / -delta[1], [0.0, 1.0, 0.0]))
    }

    #[test]
    fn gravity_bends_a_flight_by_the_multiplier() {
        let mut thrown = flyer([0.0, 0.0, 10.0]);
        let flight = thrown.step(&grenade(), [0.0, -20.0, 0.0], 0.1, |_, _| None);
        assert_eq!(flight, Flight::Flying);
        assert!((thrown.vel[1] + 1.0).abs() < 1e-5);
        assert!((thrown.pos[2] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_grenade_bounces_slower_then_lies_still_until_its_fuse() {
        let def = grenade();
        let mut thrown = flyer([0.0, -30.0, 0.0]);
        assert_eq!(thrown.step(&def, [0.0; 3], 0.1, floor), Flight::Bounced);
        assert!((thrown.vel[1] - 6.0).abs() < 1e-4);

        let mut thrown = flyer([0.0, -12.0, 0.0]);
        thrown.step(&def, [0.0; 3], 0.1, floor);
        assert!(thrown.resting);
        thrown.age = 1.95;
        let flight = thrown.step(&def, [0.0; 3], 0.1, floor);
        assert!(matches!(flight, Flight::Exploded(_)));
    }

    #[test]
    fn a_contact_projectile_goes_off_where_it_lands() {
        let def = FlyingDef {
            explodes_on_contact: true,
            gravity: 0.5,
            ..Default::default()
        };
        let mut thrown = flyer([0.0, -30.0, 0.0]);
        let flight = thrown.step(&def, [0.0; 3], 0.1, floor);
        assert_eq!(flight, Flight::Exploded([0.0, 0.0, 0.0]));
    }
}
