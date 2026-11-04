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
}

/// One `[Section]` of `hud.txt`: its lower-case path, such as `health/sprite`, and its keys.
struct Section {
    path: String,
    keys: BTreeMap<String, String>,
}

impl Section {
    fn text(&self, key: &str) -> Option<&str> {
        self.keys.get(key).map(String::as_str)
    }

    fn number(&self, key: &str) -> Option<f32> {
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

    fn file(&self, key: &str) -> Option<String> {
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
fn parse_sections(text: &str) -> Vec<Section> {
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

impl HudLayout {
    pub fn parse(text: &str) -> Option<Self> {
        let sections = parse_sections(text);
        let find = |path: &str| sections.iter().find(|s| s.path == path);
        let sprite = |path: &str| find(path)?.sprite();
        let anchor = |path: &str| find(path)?.anchor();
        let mut weapons = BTreeMap::new();
        let mut id = None;
        for section in &sections {
            match section.path.as_str() {
                "weapons/activeweapon/weaponspecific" => id = section.text("id"),
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

/// What `database/weapons/<name>.txt` says about a weapon's ammunition.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WeaponDef {
    /// `WeaponID`, the key of the weapon's `[ActiveWeapon]` sprite.
    pub id: String,
    pub clip_size: u32,
    pub pocket_size: u32,
}

impl WeaponDef {
    /// Reads a weapon file, with `shooting.h` for the sizes it names.
    pub fn parse(shooting_h: &str, weapon_txt: &str) -> Option<Self> {
        let sections = parse_sections(&format!("{shooting_h}\n{weapon_txt}"));
        let key = |name: &str| sections.iter().find_map(|s| s.text(name));
        Some(Self {
            id: key("weaponid")?.to_string(),
            clip_size: key("clipsize")?.parse().ok()?,
            pocket_size: key("pocketsize")?.parse().ok()?,
        })
    }
}

/// One weapon the player carries.
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
    pub weapons: BTreeMap<String, WeaponStock>,
    /// The weapon in hand, by lower-case file name.
    pub current: Option<String>,
}

impl PlayerInventory {
    /// `C_RemoveAllWeapons(name)`: strips the player and gives `name` with unlimited ammo.
    pub fn remove_all(&mut self, name: &str) {
        self.weapons.clear();
        self.current = None;
        if self.defs.contains_key(name) {
            self.pickup_weapon(name);
            self.set_infinite(name, true);
        }
    }

    /// `C_PickupWeapon(name)`: a full clip. A weapon already carried gains nothing: the house
    /// script picks up a Beretta it has and the retail HUD still reads `18 + 99`.
    pub fn pickup_weapon(&mut self, name: &str) {
        let Some(def) = self.defs.get(name) else {
            return;
        };
        self.weapons.entry(name.to_string()).or_insert(WeaponStock {
            clip: def.clip_size,
            ..WeaponStock::default()
        });
        if self.current.is_none() && def.id != "WEAPONID_PAINKILLER" {
            self.current = Some(name.to_string());
        }
    }

    /// `C_PickupAmmo(name, amount)`: into the pocket, up to its size.
    pub fn pickup_ammo(&mut self, name: &str, amount: u32) {
        if let (Some(def), Some(stock)) = (self.defs.get(name), self.weapons.get_mut(name)) {
            stock.pocket = (stock.pocket + amount).min(def.pocket_size);
        }
    }

    /// `C_SetInfiniteAmmo(name, on)`.
    pub fn set_infinite(&mut self, name: &str, on: bool) {
        if let Some(stock) = self.weapons.get_mut(name) {
            stock.infinite = on;
        }
    }

    /// Spends a round of the weapon in hand; `false` when it has none.
    pub fn fire(&mut self) -> bool {
        let Some(name) = &self.current else {
            return false;
        };
        let (Some(def), Some(stock)) = (self.defs.get(name), self.weapons.get_mut(name)) else {
            return false;
        };
        if stock.infinite {
            return true;
        }
        if stock.clip == 0 {
            let reload = stock.pocket.min(def.clip_size);
            stock.pocket -= reload;
            stock.clip = reload;
        }
        if stock.clip == 0 {
            return false;
        }
        stock.clip -= 1;
        true
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
            [255, 255, 255, sprite.alpha],
        );
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
                    1.0 / sheet_h,
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
        let def = |id: &str, clip_size, pocket_size| WeaponDef {
            id: id.into(),
            clip_size,
            pocket_size,
        };
        let mut defs = BTreeMap::new();
        defs.insert("beretta".to_string(), def("WEAPONID_BERETTA", 18, 162));
        defs.insert("painkiller".to_string(), def("WEAPONID_PAINKILLER", 1, 8));
        PlayerInventory {
            defs,
            ..PlayerInventory::default()
        }
    }

    fn stock(clip: u32, pocket: u32, infinite: bool) -> WeaponStock {
        WeaponStock {
            clip,
            pocket,
            infinite,
        }
    }

    #[test]
    fn a_weapon_file_gives_its_id_and_sizes_through_shooting_h() {
        let def = WeaponDef::parse(
            "#define\tBERETTA_CLIPSIZE\t\t18\n#define BERETTA_POCKETSIZE 162\n",
            "[Properties]\nWeaponID \t= WEAPONID_BERETTA;\nClipsize \t= BERETTA_CLIPSIZE;\nPocketsize\t= BERETTA_POCKETSIZE;\n",
        );
        assert_eq!(
            def,
            Some(WeaponDef {
                id: "WEAPONID_BERETTA".into(),
                clip_size: 18,
                pocket_size: 162,
            })
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
        assert_eq!(player.weapons["beretta"], stock(18, 99, false));
        assert_eq!(player.current.as_deref(), Some("beretta"));
    }

    #[test]
    fn removing_all_weapons_gives_the_named_one_unlimited_ammo() {
        let mut player = inventory();
        player.pickup_weapon("painkiller");
        player.remove_all("beretta");
        assert_eq!(player.weapons.len(), 1);
        assert!(player.weapons["beretta"].infinite);
        assert_eq!(player.current.as_deref(), Some("beretta"));
    }

    #[test]
    fn removing_all_weapons_for_empty_leaves_nothing() {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.remove_all("empty");
        assert!(player.weapons.is_empty());
        assert_eq!(player.current, None);
    }

    #[test]
    fn a_painkiller_is_not_taken_in_hand() {
        let mut player = inventory();
        player.pickup_weapon("painkiller");
        assert_eq!(player.current, None);
    }

    #[test]
    fn ammo_stops_at_the_pocket_size() {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.pickup_ammo("beretta", 500);
        assert_eq!(player.weapons["beretta"].pocket, 162);
    }

    #[test]
    fn firing_empties_the_clip_then_reloads_from_the_pocket() {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        player.pickup_ammo("beretta", 20);
        for _ in 0..18 {
            assert!(player.fire());
        }
        assert_eq!(player.weapons["beretta"], stock(0, 20, false));
        assert!(player.fire());
        assert_eq!(player.weapons["beretta"], stock(17, 2, false));
    }

    #[test]
    fn with_nothing_left_a_weapon_does_not_fire() {
        let mut player = inventory();
        player.pickup_weapon("beretta");
        for _ in 0..18 {
            player.fire();
        }
        assert!(!player.fire());
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
