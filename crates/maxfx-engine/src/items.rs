//! Level items: `data/database/level_items/*.txt`, a model and pickup rules per item type.
//! Evidence: docs/rea/2026-11-04-hud.md.

use crate::hud::PlayerInventory;
use crate::scene::LdbScene;
use std::collections::{BTreeMap, BTreeSet};

/// `max_payne.txt` `CapsuleTop`: how far above the feet the player touches an item.
const PLAYER_HEIGHT: f32 = 1.95;

/// One `level_items/<type>.txt`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LevelItemDef {
    /// `[LOD]` `ExportData`, relative to the item's own folder.
    pub model: String,
    pub radius: f32,
    pub cant_timeout: f32,
    pub weapon_id: String,
    pub if_new_weapon: bool,
    pub if_ammo_not_full: bool,
    /// Messages ready for `dispatch_message`, `activator` already the player.
    pub on_first_pickup: Vec<String>,
    pub on_pickup: Vec<String>,
    pub on_cant_pickup: Vec<String>,
}

impl LevelItemDef {
    /// `headers` is `level_items.h`, for the numbers the file names.
    pub fn parse(headers: &str, text: &str) -> Option<Self> {
        let defines: BTreeMap<&str, &str> = headers
            .lines()
            .filter_map(|line| {
                let mut words = line.strip_prefix("#define")?.split_whitespace();
                Some((words.next()?, words.next()?))
            })
            .collect();
        let expand = |word: &str| {
            let (name, comma) = match word.strip_suffix(',') {
                Some(name) => (name, ","),
                None => (word, ""),
            };
            format!("{}{comma}", defines.get(name).copied().unwrap_or(name))
        };
        let mut def = Self::default();
        let mut block = None;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("//") {
                continue;
            }
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("[onfirstpickup]") {
                block = Some(&mut def.on_first_pickup);
            } else if lower.starts_with("[onpickup]") {
                block = Some(&mut def.on_pickup);
            } else if lower.starts_with("[oncantpickup]") {
                block = Some(&mut def.on_cant_pickup);
            } else if lower.starts_with("[message]") {
                let Some((_, value)) = line.split_once('=') else {
                    continue;
                };
                let message = value.trim().trim_end_matches(';').replace(['\\', '"'], "");
                let words: Vec<String> = message.split_whitespace().map(expand).collect();
                let message = words.join(" ").replacen("activator->", "player->", 1);
                if let Some(block) = block.as_mut() {
                    block.push(message);
                }
            }
        }
        let property = |name: &str| {
            text.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                let value = value.split(';').next()?.trim();
                key.trim().eq_ignore_ascii_case(name).then(|| expand(value))
            })
        };
        def.model = property("ExportData")?;
        def.radius = property("TriggerRadius")?.parse().ok()?;
        def.cant_timeout = property("CantPickupMessagesTimeout")?.parse().ok()?;
        def.weapon_id = property("MatchingWeaponID")?;
        let flag = |name| property(name).is_some_and(|value| value.eq_ignore_ascii_case("true"));
        def.if_new_weapon = flag("PickupIfNewWeapon");
        def.if_ammo_not_full = flag("PickupIfAmmoNotFull");
        Some(def)
    }

    /// Whether the player may take this item now.
    pub fn can_pick_up(&self, inventory: &PlayerInventory) -> bool {
        let Some((name, weapon)) = inventory
            .defs
            .iter()
            .find(|(_, weapon)| weapon.id == self.weapon_id)
        else {
            return false;
        };
        let pocket = inventory.stock(name).map_or(0, |stock| stock.pocket);
        (self.if_new_weapon && !inventory.carried.contains(name))
            || (self.if_ammo_not_full && pocket < weapon.pocket_size)
    }
}

/// One item lying in the level.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveItem {
    /// The scene item's name, which `LI_ActivateLevelItem` addresses.
    pub name: String,
    pub kind: String,
    /// World position.
    pub pos: [f32; 3],
    pub taken: bool,
}

/// The level's items and what the player has done with them.
#[derive(Debug, Clone, Default)]
pub struct LevelItems {
    pub defs: BTreeMap<String, LevelItemDef>,
    pub items: Vec<LiveItem>,
    /// `max_payne.txt` `CapsuleRadius`.
    pub body_radius: f32,
    /// Types picked up once: the next ones run `[OnPickUp]`.
    picked: BTreeSet<String>,
    /// Seconds until each type may repeat its `[OnCantPickup]`.
    refused: BTreeMap<String, f32>,
}

impl LevelItems {
    /// Run the pickup rules for the items the player at `feet` touches.
    pub fn step(&mut self, scene: &mut LdbScene, feet: [f32; 3], seconds: f32) {
        for wait in self.refused.values_mut() {
            *wait -= seconds;
        }
        for item in self.items.iter_mut().filter(|item| !item.taken) {
            let Some(def) = self.defs.get(&item.kind) else {
                continue;
            };
            if !scene.item_active(&item.name) {
                continue;
            }
            let [dx, dy, dz] = [0, 1, 2].map(|c| item.pos[c] - feet[c]);
            // HYPOTHESIS: the trigger sphere touches the capsule. Measured to the player's centre,
            // the van's items are 0.87 m from the closest spot outside, past their 0.75 m radius.
            let reach = def.radius + self.body_radius;
            if dx.hypot(dz) > reach || !(-def.radius..=PLAYER_HEIGHT).contains(&dy) {
                continue;
            }
            let messages = if !def.can_pick_up(&scene.inventory) {
                if self.refused.get(&item.kind).is_some_and(|wait| *wait > 0.0) {
                    continue;
                }
                self.refused.insert(item.kind.clone(), def.cant_timeout);
                &def.on_cant_pickup
            } else {
                item.taken = true;
                if self.picked.insert(item.kind.clone()) {
                    &def.on_first_pickup
                } else {
                    &def.on_pickup
                }
            };
            for message in messages {
                scene.dispatch_message("", message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::{WeaponDef, WeaponStock};

    const HEADERS: &str = "#define AMMOBOX_RADIUS\t\t\t\t0.75\n#define CANTPICKUP_TIMEOUT\t\t\t5.0\n#define AMMOBOX_BERETTA\t\t\t\t30\n#define PICKUPAMMO_BERETTA\t\t\t10\n";
    const BERETTA: &str = r#"#include <..\level_items.h>
[LOD]
{
    [Geometry]
	ExportData = ..\..\weapons\beretta\beretta_LevelItem.kf2;
    Distance = PROJECTILE_LOD_DRAW_ALWAYS;
}

[Properties]
TriggerRadius = AMMOBOX_RADIUS;
CantPickupMessagesTimeout = CANTPICKUP_TIMEOUT;
MatchingWeaponID = WEAPONID_BERETTA;
PickupIfNewWeapon = true;
PickupIfAmmoNotFull = true;

[OnFirstPickup]
{
	[Message] string = "activator->C_PickupWeapon( "beretta" );";
	[Message] string = "activator->C_PickupAmmo( "beretta"," PICKUPAMMO_BERETTA ");";
	[Message] string = "maxpayne_hudmode->mphm_print( \"HUDPRINT_PICKUP_BERETTA\" );";
//	[Message] string = "maxpayne_gamemode->A_PlaySound(characters, get_painkillers);";
}

[OnPickUp]
{
	[Message] string = "activator->c_pickupammo( beretta, " AMMOBOX_BERETTA " );";
}

[OnCantPickup]
{
	[Message] string = "maxpayne_hudmode->mphm_print( HUDPRINT_FULL );";
}
"#;

    #[test]
    fn an_item_file_gives_its_rules_and_messages() {
        let def = LevelItemDef::parse(HEADERS, BERETTA).expect("item");
        assert_eq!(def.model, r"..\..\weapons\beretta\beretta_LevelItem.kf2");
        assert_eq!((def.radius, def.cant_timeout), (0.75, 5.0));
        assert_eq!(def.weapon_id, "WEAPONID_BERETTA");
        assert!(def.if_new_weapon && def.if_ammo_not_full);
        assert_eq!(
            def.on_first_pickup,
            [
                "player->C_PickupWeapon( beretta );",
                "player->C_PickupAmmo( beretta, 10 );",
                "maxpayne_hudmode->mphm_print( HUDPRINT_PICKUP_BERETTA );",
            ]
        );
        assert_eq!(def.on_pickup, ["player->c_pickupammo( beretta, 30 );"]);
        assert_eq!(def.on_cant_pickup.len(), 1);
    }

    fn inventory(carried: bool, pocket: u32) -> PlayerInventory {
        let mut inventory = PlayerInventory::default();
        inventory.defs.insert(
            "beretta".to_string(),
            WeaponDef {
                id: "WEAPONID_BERETTA".into(),
                inventory_id: "INV".into(),
                pocket_size: 162,
                ..Default::default()
            },
        );
        if carried {
            inventory.carried.insert("beretta".to_string());
            let stock = WeaponStock {
                pocket,
                ..Default::default()
            };
            inventory.ammo.insert("INV".to_string(), stock);
        }
        inventory
    }

    fn level(pocket: u32) -> (LevelItems, LdbScene) {
        let mut items = LevelItems::default();
        let def = LevelItemDef::parse(HEADERS, BERETTA).expect("item");
        items.defs.insert("beretta".to_string(), def);
        for x in [0.0, 0.5, 9.0] {
            items.items.push(LiveItem {
                name: format!("::Room::item{x}"),
                kind: "beretta".to_string(),
                pos: [x, 0.2, 0.0],
                taken: false,
            });
        }
        let scene = LdbScene {
            inventory: inventory(true, pocket),
            ..Default::default()
        };
        (items, scene)
    }

    #[test]
    fn walking_over_items_takes_the_near_ones_first_pickup_then_later_ones() {
        let (mut items, mut scene) = level(0);
        items.step(&mut scene, [0.0; 3], 0.1);
        let taken: Vec<bool> = items.items.iter().map(|item| item.taken).collect();
        assert_eq!(taken, [true, true, false]);
        // `[OnFirstPickup]` gives 10 rounds, then `[OnPickUp]` 30.
        assert_eq!(scene.inventory.stock("beretta").map(|s| s.pocket), Some(40));
        assert_eq!(scene.hud_prints.len(), 1);
    }

    #[test]
    fn an_item_is_reached_by_the_players_body_not_only_its_centre() {
        let (mut items, mut scene) = level(0);
        items.body_radius = 0.48;
        items.step(&mut scene, [0.0, 0.0, 1.0], 0.1);
        assert!(items.items[0].taken, "1 m away is inside 0.75 + 0.48");
        items.step(&mut scene, [9.0, 0.0, 1.3], 0.1);
        assert!(!items.items[2].taken, "1.3 m away is not");
    }

    #[test]
    fn an_inactive_item_waits_for_the_message_that_activates_it() {
        let (mut items, mut scene) = level(0);
        scene.items.push(crate::scene::SceneItem {
            name: "::Room::item0".to_string(),
            item_type: "beretta".to_string(),
            room_id: 0,
            pos: [0.0; 3],
            active: true,
        });
        scene.fsms.push(crate::scene::SceneFsm {
            startup_actions: vec!["::Room::item0->LI_ActivateLevelItem(false);".to_string()],
            ..Default::default()
        });
        scene.apply_startup_items();
        items.step(&mut scene, [0.0, 0.0, 0.0], 0.1);
        assert!(!items.items[0].taken);
        assert!(scene.dispatch_message("", "::Room::item0->li_activatelevelitem(true);"));
        items.step(&mut scene, [0.0, 0.0, 0.0], 0.1);
        assert!(items.items[0].taken);
    }

    #[test]
    fn a_refused_item_stays_and_repeats_its_message_after_the_timeout() {
        let (mut items, mut scene) = level(162);
        items.items.truncate(1);
        items.step(&mut scene, [0.0; 3], 0.1);
        items.step(&mut scene, [0.0; 3], 1.0);
        assert!(!items.items[0].taken);
        assert_eq!(scene.hud_prints.len(), 1);
        items.step(&mut scene, [0.0; 3], 5.0);
        assert_eq!(scene.hud_prints.len(), 2);
    }

    #[test]
    fn an_item_above_the_player_is_out_of_reach() {
        let (mut items, mut scene) = level(0);
        items.step(&mut scene, [0.0, -3.0, 0.0], 0.1);
        assert!(items.items.iter().all(|item| !item.taken));
    }

    #[test]
    fn an_item_is_taken_for_a_new_weapon_or_missing_ammo_only() {
        let weapon = LevelItemDef::parse(HEADERS, BERETTA).expect("item");
        assert!(weapon.can_pick_up(&inventory(false, 0)));
        assert!(weapon.can_pick_up(&inventory(true, 100)));
        assert!(!weapon.can_pick_up(&inventory(true, 162)));
        let ammo = LevelItemDef {
            if_new_weapon: false,
            ..weapon
        };
        assert!(ammo.can_pick_up(&inventory(false, 0)));
        assert!(!ammo.can_pick_up(&inventory(true, 162)));
    }
}
