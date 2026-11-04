//! Bullet holes: `data/database/decals/decals.txt`, one block per decal, a bitmap per material.
//! Evidence: docs/rea/2026-11-04-hud.md.

use crate::particles::{Sprite, SpriteTexture};
use crate::scene::DecodedTexture;
use std::collections::{BTreeMap, VecDeque};

/// `globalconfig.txt` `MaximumDecalTrianglesPerRoom` 2000, as quads, for the whole level.
const MAX_DECALS: usize = 1000;
/// How far a decal sits off its surface, toward the shooter.
const SURFACE_OFFSET: f32 = 0.01;

/// One `[Materials]` entry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DecalMaterial {
    pub file: String,
    pub alpha_file: String,
    pub min_radius: f32,
    pub max_radius: f32,
    /// `RotationRandom`, degrees.
    pub rotation: f32,
    pub flip: bool,
}

/// One decal block of `decals.txt`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DecalSet {
    /// `[NoDecalMaterial]` names, lower case.
    pub no_decal: Vec<String>,
    /// By lower-case material category.
    pub materials: BTreeMap<String, DecalMaterial>,
}

impl DecalSet {
    pub fn parse(text: &str, block: &str) -> Option<Self> {
        let prefix = format!("{}/materials/", block.to_ascii_lowercase());
        let mut materials = BTreeMap::new();
        for section in crate::hud::parse_sections(text) {
            let Some(name) = section.path.strip_prefix(&prefix) else {
                continue;
            };
            let material = DecalMaterial {
                file: section.file("filename")?,
                alpha_file: section.file("alphafilename")?,
                min_radius: section.number("minimumradius")?,
                max_radius: section.number("maximumradius")?,
                rotation: section.number("rotationrandom")?,
                flip: section
                    .text("bitmapfliprandom")
                    .is_some_and(|flip| flip.eq_ignore_ascii_case("true")),
            };
            materials.insert(name.to_string(), material);
        }
        let start = text
            .to_ascii_lowercase()
            .find(&format!("[{}]", block.to_ascii_lowercase()))?;
        let no_decal = text[start..]
            .lines()
            .skip(1)
            .take_while(|line| !line.trim_start().starts_with("[Materials]"))
            .filter_map(|line| {
                let name = line.split_once("[NoDecalMaterial]")?.1.split_once('"')?.1;
                Some(name.split('"').next()?.to_ascii_lowercase())
            })
            .collect();
        (!materials.is_empty()).then_some(Self {
            no_decal,
            materials,
        })
    }

    /// The entry for a surface: its category, else `Default`, unless the category takes no decals.
    pub fn material(&self, category: &str) -> Option<&DecalMaterial> {
        let category = category.to_ascii_lowercase();
        if self.no_decal.contains(&category) {
            return None;
        }
        self.materials
            .get(&category)
            .or_else(|| self.materials.get("default"))
    }
}

/// A placed decal.
#[derive(Debug, Clone, PartialEq)]
pub struct Decal {
    pub pos: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub size: f32,
    pub file: String,
    /// The dynamic mesh it sits on: it goes when that mesh is hidden.
    pub owner: Option<usize>,
}

/// The level's decals, oldest first.
#[derive(Debug, Clone, Default)]
pub struct DecalField {
    pub set: DecalSet,
    pub decals: VecDeque<Decal>,
    pub textures: BTreeMap<SpriteTexture, DecodedTexture>,
    seed: u32,
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-8);
    v.map(|c| c / length)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl DecalField {
    /// Reads the bullet decals and their bitmaps from the game data.
    pub fn load(game_dir: &std::path::Path) -> Self {
        let read = |file: &str| {
            crate::read_game_bytes(game_dir, &format!(r"\data\database\decals\{file}"))
        };
        let set = read("decals.txt")
            .and_then(|bytes| {
                DecalSet::parse(&String::from_utf8_lossy(&bytes), "bullethit_generic")
            })
            .unwrap_or_default();
        let bitmap = |material: &DecalMaterial| {
            let decode = |file: &str| crate::scene::decode_texture_blob(&read(file)?);
            let (width, height, colour) = decode(&material.file)?;
            let (_, _, alpha) = decode(&material.alpha_file)?;
            let pixels = colour
                .iter()
                .zip(&alpha)
                .map(|(rgb, grey)| ((grey & 0x00ff_0000) << 8) | (rgb & 0x00ff_ffff))
                .collect();
            Some(DecodedTexture {
                path: material.file.clone(),
                flag: 0,
                width,
                height,
                pixels,
            })
        };
        let textures = set
            .materials
            .values()
            .filter_map(|material| Some((texture_key(&material.file), bitmap(material)?)))
            .collect();
        Self {
            set,
            textures,
            ..Self::default()
        }
    }

    fn random(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Places a decal for a shot along `dir` that hit a surface of `category` at `point`.
    pub fn add(
        &mut self,
        category: &str,
        point: [f32; 3],
        normal: [f32; 3],
        dir: [f32; 3],
        owner: Option<usize>,
    ) -> bool {
        let Some(material) = self.set.material(category).cloned() else {
            return false;
        };
        let along = normal[0] * dir[0] + normal[1] * dir[1] + normal[2] * dir[2];
        let n = if along > 0.0 {
            normal.map(|c| -c)
        } else {
            normal
        };
        let reference = if n[1].abs() < 0.9 {
            [0.0, 1.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        let right = normalize(cross(reference, n));
        let up = cross(n, right);
        let (roll, spread) = (self.random() - 0.5, self.random());
        let (sin, cos) = (roll * material.rotation).to_radians().sin_cos();
        let mirror = if material.flip && self.random() < 0.5 {
            -1.0
        } else {
            1.0
        };
        let radius = material.min_radius + (material.max_radius - material.min_radius) * spread;
        if self.decals.len() == MAX_DECALS {
            self.decals.pop_front();
        }
        self.decals.push_back(Decal {
            pos: [0, 1, 2].map(|k| point[k] + n[k] * SURFACE_OFFSET),
            right: [0, 1, 2].map(|k| (right[k] * cos + up[k] * sin) * mirror),
            up: [0, 1, 2].map(|k| up[k] * cos - right[k] * sin),
            size: radius * 2.0,
            file: material.file,
            owner,
        });
        true
    }

    /// Decals to draw; `hidden` says whether a dynamic mesh is hidden.
    pub fn sprites(&self, hidden: impl Fn(usize) -> bool) -> Vec<Sprite> {
        self.decals
            .iter()
            .filter(|decal| !decal.owner.is_some_and(&hidden))
            .map(|decal| Sprite {
                pos: decal.pos,
                size: decal.size,
                rgba: [1.0; 4],
                additive: false,
                texture: texture_key(&decal.file),
                facing: Some([decal.right, decal.up]),
            })
            .collect()
    }

    pub fn texture(&self, key: &SpriteTexture) -> Option<&DecodedTexture> {
        self.textures.get(key)
    }
}

fn texture_key(file: &str) -> SpriteTexture {
    SpriteTexture {
        effect: format!("decal:{file}"),
        emitter: 0,
        sequence: 0,
        frame: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"
[other] // not this one
{
	[Materials]
	{
		[Wood]
		Filename = "bitmaps\other.jpg";
		AlphaFilename = "bitmaps\other_a.jpg";
		MinimumRadius = 1;
		MaximumRadius = 2;
		RotationRandom = 0;
		BitmapFlipRandom = FALSE;
	}
}
[bullethit_generic] // bullet holes
{
	[Message]
	String ="this->PS_StartEffect( bullethit_generic, \"\");";

	[NoDecalMaterial] Name = "Character";
	[NoDecalMaterial] Name = "Nodecals";

	[Materials]
	{

		[Default]
		Filename = "bitmaps\hole_default.jpg";
		AlphaFilename = "bitmaps\hole_default_a.jpg";
		MinimumRadius = 0.02;
		MaximumRadius = 0.02;
		RotationRandom = 0;
		BitmapFlipRandom = FALSE;
		DualSided = FALSE;

		[Wood]
		Filename = "bitmaps\Hole_Wood.jpg";
		AlphaFilename = "bitmaps\Hole_Wood_alpha.jpg";
		MinimumRadius = 0.040;
		MaximumRadius = 0.090;
		RotationRandom = 140.0;
		BitmapFlipRandom = TRUE;
		DualSided = FALSE;
	}
}
"#;

    fn field() -> DecalField {
        DecalField {
            set: DecalSet::parse(TEXT, "bullethit_generic").expect("set"),
            ..DecalField::default()
        }
    }

    #[test]
    fn retail_bullet_decals_load_with_their_bitmaps() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        if crate::read_game_bytes(&game, r"\data\database\decals\decals.txt").is_none() {
            return;
        }
        let field = DecalField::load(&game);
        assert!(
            field.set.materials.len() >= 20,
            "{:?}",
            field.set.materials.keys()
        );
        assert!(field.set.material("wood").is_some());
        assert!(
            field.set.material("externalwall").is_some(),
            "falls back to Default"
        );
        assert!(field.set.material("skybox").is_none());
        for material in field.set.materials.values() {
            let texture = field.texture(&texture_key(&material.file));
            let texture = texture.unwrap_or_else(|| panic!("{}", material.file));
            assert!(
                texture.pixels.iter().any(|px| px >> 24 > 128),
                "{}",
                material.file
            );
        }
    }

    #[test]
    fn a_block_reads_its_own_materials() {
        let set = DecalSet::parse(TEXT, "bullethit_generic").expect("set");
        let wood = &set.materials["wood"];
        assert_eq!(wood.file, r"bitmaps\Hole_Wood.jpg");
        assert_eq!(wood.alpha_file, r"bitmaps\Hole_Wood_alpha.jpg");
        assert_eq!((wood.min_radius, wood.max_radius), (0.04, 0.09));
        assert_eq!(wood.rotation, 140.0);
        assert!(wood.flip);
    }

    #[test]
    fn a_block_reads_its_excluded_materials_only() {
        let set = DecalSet::parse(TEXT, "bullethit_generic").expect("set");
        assert_eq!(set.no_decal, ["character", "nodecals"]);
        assert!(DecalSet::parse(TEXT, "other")
            .expect("other")
            .no_decal
            .is_empty());
    }

    #[test]
    fn a_surface_uses_its_category_else_default_unless_excluded() {
        let set = DecalSet::parse(TEXT, "bullethit_generic").expect("set");
        assert_eq!(set.material("Wood").map(|m| m.max_radius), Some(0.09));
        assert_eq!(set.material("stucco").map(|m| m.max_radius), Some(0.02));
        assert_eq!(set.material("Nodecals"), None);
    }

    #[test]
    fn a_decal_lies_on_the_surface_just_toward_the_shooter() {
        let mut decals = field();
        assert!(decals.add(
            "stucco",
            [1.0, 2.0, 3.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, 1.0],
            None
        ));
        let decal = &decals.decals[0];
        assert_eq!(decal.pos, [1.0, 2.0, 3.0 - SURFACE_OFFSET]);
        assert_eq!(decal.size, 0.04);
        assert_eq!(decal.right[2], 0.0);
        assert_eq!(decal.up, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_wall_hit_from_behind_faces_the_shooter() {
        let mut decals = field();
        decals.add("stucco", [0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], None);
        assert_eq!(decals.decals[0].pos, [0.0, 0.0, -SURFACE_OFFSET]);
    }

    #[test]
    fn a_floor_decal_lies_flat() {
        let mut decals = field();
        decals.add("stucco", [0.0; 3], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], None);
        let decal = &decals.decals[0];
        assert_eq!(decal.right[1], 0.0);
        assert_eq!(decal.up[1], 0.0);
    }

    #[test]
    fn a_surface_that_takes_no_decals_gets_none() {
        let mut decals = field();
        assert!(!decals.add(
            "character",
            [0.0; 3],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            None
        ));
        assert!(decals.decals.is_empty());
    }

    #[test]
    fn the_oldest_decal_goes_when_the_field_is_full() {
        let mut decals = field();
        for k in 0..MAX_DECALS + 1 {
            decals.add(
                "stucco",
                [k as f32, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, -1.0, 0.0],
                None,
            );
        }
        assert_eq!(decals.decals.len(), MAX_DECALS);
        assert_eq!(decals.decals[0].pos[0], 1.0);
    }

    #[test]
    fn a_decal_turns_into_an_oriented_sprite_and_goes_with_its_mesh() {
        let mut decals = field();
        decals.add(
            "stucco",
            [0.0; 3],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, 1.0],
            Some(3),
        );
        let sprites = decals.sprites(|_| false);
        assert_eq!(sprites.len(), 1);
        assert_eq!(sprites[0].size, 0.04);
        assert_eq!(sprites[0].facing.map(|f| f[1]), Some([0.0, 1.0, 0.0]));
        assert_eq!(sprites[0].texture.effect, r"decal:bitmaps\hole_default.jpg");
        assert!(decals.sprites(|mesh| mesh == 3).is_empty());
    }
}
