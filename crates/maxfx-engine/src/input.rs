//! Key bindings: retail's slot table, its defaults, and the key names tips print.
//! See `docs/rea/2026-11-07-key-bindings.md`.

use std::collections::BTreeMap;

/// One of a slot's four buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// A DirectInput scan code.
    Key(u32),
    /// A mouse button, 0 for the first.
    Mouse(u32),
}

/// A slot's analog source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Analog {
    MouseLeftRight,
    MouseUpDown,
    MouseWheel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Slot {
    /// Buttons 0 and 2 drive one direction, 1 and 3 the other.
    pub buttons: [Option<Button>; 4],
    pub analog: Option<Analog>,
}

/// The 32 slots at `0x8a9168`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bindings {
    pub slots: [Slot; 32],
}

fn button(kind: u32, value: u32) -> Option<Button> {
    match kind {
        1 => Some(Button::Key(value)),
        2 => Some(Button::Mouse(value)),
        _ => None,
    }
}

fn analog(kind: u32) -> Option<Analog> {
    match kind {
        1 => Some(Analog::MouseLeftRight),
        2 => Some(Analog::MouseUpDown),
        3 => Some(Analog::MouseWheel),
        _ => None,
    }
}

/// `(slot, button, type, value)` written by `0x4cb090`, in its order.
const RETAIL_DEFAULT_BUTTONS: [(usize, usize, u32, u32); 32] = [
    (0, 0, 1, 0x11),
    (0, 1, 1, 0x1f),
    (0, 2, 1, 0xc8),
    (0, 3, 1, 0xd0),
    (1, 0, 1, 0x1e),
    (1, 1, 1, 0x20),
    (1, 2, 1, 0xcb),
    (1, 3, 1, 0xcd),
    (2, 0, 2, 0),
    (3, 0, 1, 0x13),
    (3, 2, 1, 0x52),
    (4, 0, 1, 0x39),
    (4, 2, 1, 0x36),
    (5, 0, 1, 0x2e),
    (5, 2, 1, 0x9d),
    (13, 0, 1, 0x12),
    (13, 2, 1, 0x1c),
    (14, 0, 1, 0x02),
    (15, 0, 1, 0x03),
    (16, 0, 1, 0x04),
    (17, 0, 1, 0x05),
    (18, 0, 1, 0x06),
    (19, 0, 1, 0x07),
    (25, 0, 1, 0x22),
    (26, 0, 1, 0x1a),
    (26, 1, 1, 0x1b),
    (28, 0, 1, 0x0f),
    (28, 2, 1, 0x0e),
    (27, 0, 1, 0x19),
    (27, 2, 1, 0xc5),
    (29, 0, 1, 0x10),
    (29, 2, 1, 0x4f),
];

/// `(slot, analog type)` written by `0x4cb090`.
const RETAIL_DEFAULT_ANALOG: [(usize, u32); 3] = [(11, 2), (12, 1), (26, 3)];

/// `(slot, button, type, value)` that `0x4cb090` writes last, after the analog sources.
const RETAIL_DEFAULT_BULLET_TIME: [(usize, usize, u32, u32); 2] = [(31, 0, 2, 1), (31, 2, 1, 0x2a)];

impl Bindings {
    /// The defaults `0x4cb090` writes.
    pub fn retail_defaults() -> Self {
        let mut slots = [Slot::default(); 32];
        let buttons = RETAIL_DEFAULT_BUTTONS
            .iter()
            .chain(&RETAIL_DEFAULT_BULLET_TIME);
        for &(slot, index, kind, value) in buttons {
            slots[slot].buttons[index] = button(kind, value);
        }
        for &(slot, kind) in &RETAIL_DEFAULT_ANALOG {
            slots[slot].analog = analog(kind);
        }
        Self { slots }
    }

    /// `Input Settings` from a Wine prefix's `user.reg`, over the defaults; `None` without that
    /// section. Names are `" %d Button%d type"` and `value`, and `" %d Slider type"` (`0x4cb3d0`).
    pub fn from_wine_registry(user_reg: &str) -> Option<Self> {
        let header = r"[Software\\Remedy Entertainment\\Max Payne\\Input Settings]";
        let start = user_reg.find(header)?;
        let body = user_reg[start + header.len()..].lines().skip(1);
        let mut values: BTreeMap<String, u32> = BTreeMap::new();
        for line in body {
            if line.starts_with('[') {
                break;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let name = name.trim().trim_matches('"').trim();
            let Some(value) = value.trim().strip_prefix("dword:") else {
                continue;
            };
            if let Ok(value) = u32::from_str_radix(value, 16) {
                values.insert(name.to_string(), value);
            }
        }
        let mut bindings = Self::retail_defaults();
        for (index, slot) in bindings.slots.iter_mut().enumerate() {
            for (k, button_slot) in slot.buttons.iter_mut().enumerate() {
                let kind = values.get(&format!("{index} Button{k} type"));
                let value = values.get(&format!("{index} Button{k} value"));
                if let Some(&kind) = kind {
                    *button_slot = button(kind, value.copied().unwrap_or(0));
                }
            }
            if let Some(&kind) = values.get(&format!("{index} Slider type")) {
                slot.analog = analog(kind);
            }
        }
        Some(bindings)
    }
}

/// Names Windows gives scan codes on a US layout (`GetKeyNameTextA`, import `0x7ca478`), for
/// the codes `input.txt` leaves out. OS data: retail shows the player's own layout names.
fn us_layout_name(code: u32) -> Option<String> {
    const ROW: &[&str] = &[
        "",
        "Esc",
        "1",
        "2",
        "3",
        "4",
        "5",
        "6",
        "7",
        "8",
        "9",
        "0",
        "-",
        "=",
        "Backspace",
        "Tab",
        "Q",
        "W",
        "E",
        "R",
        "T",
        "Y",
        "U",
        "I",
        "O",
        "P",
        "[",
        "]",
        "Enter",
        "Ctrl",
        "A",
        "S",
        "D",
        "F",
        "G",
        "H",
        "J",
        "K",
        "L",
        ";",
        "'",
        "`",
        "Shift",
        "\\",
        "Z",
        "X",
        "C",
        "V",
        "B",
        "N",
        "M",
        ",",
        ".",
        "/",
        "Right Shift",
        "Num *",
        "Alt",
        "Space",
        "Caps Lock",
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "Pause",
        "Scroll Lock",
        "Num 7",
        "Num 8",
        "Num 9",
        "Num -",
        "Num 4",
        "Num 5",
        "Num 6",
        "Num +",
        "Num 1",
        "Num 2",
        "Num 3",
        "Num 0",
        "Num Del",
        "",
        "",
        "",
    ];
    match code {
        87 => Some("F11".to_string()),
        88 => Some("F12".to_string()),
        _ => ROW
            .get(code as usize)
            .filter(|name| !name.is_empty())
            .map(|name| name.to_string()),
    }
}

/// The quoted text after `key =` on a `[names]` line.
fn quoted(line: &str) -> Option<String> {
    let (_, rest) = line.split_once('"')?;
    Some(rest.split_once('"')?.0.to_string())
}

/// Key names: `data/input.txt`, then the separator and unassigned text from the menu.
#[derive(Debug, Clone, Default)]
pub struct KeyNames {
    keys: BTreeMap<u32, String>,
    mouse_button: String,
    mouse_wheel: String,
    mouse_left_right: String,
    mouse_up_down: String,
    separator: String,
    unassigned: String,
}

impl KeyNames {
    /// `input` is `data/input.txt`; `stringdefines` the language's `database/stringdefines.h`,
    /// which `menu.txt`'s `ControlSeparatorString` and `UnassignedText` name.
    pub fn parse(input: &str, stringdefines: &str) -> Self {
        let mut names = Self::default();
        for line in input.lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            if let Some(rest) = line.strip_prefix("[key]") {
                let code = rest
                    .split_once("ScanCode")
                    .and_then(|(_, r)| r.trim_start().strip_prefix('='))
                    .and_then(|r| r.split(';').next())
                    .and_then(|r| r.trim().parse().ok());
                if let (Some(code), Some(name)) = (code, quoted(rest)) {
                    names.keys.insert(code, name);
                }
                continue;
            }
            let Some((key, _)) = line.split_once('=') else {
                continue;
            };
            let Some(text) = quoted(line) else {
                continue;
            };
            match key.trim() {
                "MouseButtonName" => names.mouse_button = text,
                "MouseWheelName" => names.mouse_wheel = text,
                "MouseLRName" => names.mouse_left_right = text,
                "MouseUDName" => names.mouse_up_down = text,
                _ => {}
            }
        }
        for line in stringdefines.lines() {
            let Some(define) = line.trim().strip_prefix("#define") else {
                continue;
            };
            let Some((name, _)) = define.trim().split_once(char::is_whitespace) else {
                continue;
            };
            match (name, quoted(define)) {
                ("GCID_SEPARATOR", Some(text)) => names.separator = text,
                ("GCID_UNASSIGNED", Some(text)) => names.unassigned = text,
                _ => {}
            }
        }
        names
    }

    pub fn button(&self, button: Button) -> String {
        match button {
            Button::Mouse(index) => format!("{}{}", self.mouse_button, index + 1),
            Button::Key(code) => self
                .keys
                .get(&code)
                .cloned()
                .or_else(|| us_layout_name(code))
                .unwrap_or_default(),
        }
    }

    fn analog(&self, analog: Analog) -> &str {
        match analog {
            Analog::MouseLeftRight => &self.mouse_left_right,
            Analog::MouseUpDown => &self.mouse_up_down,
            Analog::MouseWheel => &self.mouse_wheel,
        }
    }

    /// The names `0x4cdcf0(direction)` joins: button `direction`, button `direction + 2`, then
    /// the analog source.
    fn slot(&self, slot: &Slot, direction: usize) -> String {
        let buttons = [slot.buttons[direction], slot.buttons[direction + 2]];
        let mut parts: Vec<String> = buttons
            .into_iter()
            .flatten()
            .map(|b| self.button(b))
            .collect();
        parts.extend(slot.analog.map(|a| self.analog(a).to_string()));
        if parts.is_empty() {
            return self.unassigned.clone();
        }
        parts.join(&self.separator)
    }
}

/// Each tip token's slot and direction, in `0x4cce80`'s order.
const TOKENS: [(&str, usize, usize); 37] = [
    ("#RUN#", 0, 0),
    ("#WALKBACK#", 0, 1),
    ("#STRAFELEFT#", 1, 0),
    ("#STAFERIGHT#", 1, 1),
    ("#SHOOT#", 2, 0),
    ("#RELOAD#", 3, 0),
    ("#JUMP#", 4, 0),
    ("#CROUCH#", 5, 0),
    ("#DODGELEFT#", 6, 0),
    ("#DODGERIGHT#", 7, 0),
    ("#DODGEFORWARD#", 8, 0),
    ("#DODGEBACKWARD#", 9, 0),
    ("#DODGEMODIFIER#", 10, 0),
    ("#AIMUP#", 11, 0),
    ("#AIMDOWN#", 11, 1),
    ("#AIMLEFT#", 12, 0),
    ("#AIMRIGHT#", 12, 1),
    ("#USE#", 13, 0),
    ("#BESTWEAPON#", 25, 0),
    ("#NEXTWEAPON#", 26, 0),
    ("#PREVIOUSWEAPON#", 26, 1),
    ("#SELECTWEAPONSLOT0#", 14, 0),
    ("#SELECTWEAPONSLOT1#", 15, 0),
    ("#SELECTWEAPONSLOT2#", 16, 0),
    ("#SELECTWEAPONSLOT3#", 17, 0),
    ("#SELECTWEAPONSLOT4#", 18, 0),
    ("#SELECTWEAPONSLOT5#", 19, 0),
    ("#SELECTWEAPONSLOT6#", 20, 0),
    ("#SELECTWEAPONSLOT7#", 21, 0),
    ("#SELECTWEAPONSLOT8#", 22, 0),
    ("#SELECTWEAPONSLOT9#", 23, 0),
    ("#SELECTWEAPONSLOT10#", 24, 0),
    ("#PAUSE#", 27, 0),
    ("#PAINKILLER#", 28, 0),
    ("#SNIPERZOOM#", 29, 0),
    ("#SLOWMOTION#", 30, 0),
    ("#BULLETTIME#", 31, 0),
];

/// `#TOKEN#`s in `text` replaced by the keys bound to them (`0x4cce80`).
pub fn expand_tokens(text: &str, bindings: &Bindings, names: &KeyNames) -> String {
    TOKENS
        .iter()
        .filter(|(token, _, _)| text.contains(token))
        .fold(text.to_string(), |text, &(token, slot, direction)| {
            text.replace(token, &names.slot(&bindings.slots[slot], direction))
        })
}

fn latin1(bytes: Vec<u8>) -> String {
    bytes.into_iter().map(char::from).collect()
}

/// The player's bindings: retail's own from the Wine prefix the launcher uses (`$WINEPREFIX`, else
/// `<game>/.wine`), else the retail defaults.
pub fn load_bindings(game_dir: &std::path::Path) -> Bindings {
    let prefix = std::env::var_os("WINEPREFIX")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| game_dir.join(".wine"));
    std::fs::read(prefix.join("user.reg"))
        .ok()
        .and_then(|bytes| Bindings::from_wine_registry(&latin1(bytes)))
        .unwrap_or_else(Bindings::retail_defaults)
}

/// `data/input.txt` and the language's `database/stringdefines.h`.
pub fn load_key_names(game_dir: &std::path::Path) -> KeyNames {
    let read = |path: &str| crate::read_game_bytes(game_dir, path).map(latin1);
    KeyNames::parse(
        &read(r"\data\input.txt").unwrap_or_default(),
        &read(r"\data\database\stringdefines.h").unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT: &str = "[key] ScanCode = 200; KeyName = \"UP\";\n\
        [key] ScanCode = 157; KeyName = \"R_CTRL\";\n\
        [key] ScanCode = 208; KeyName = \"DOWN\";\n\
        \n[names]\n\
        MouseButtonName\t= \"MB \";\t\t// leave space after\n\
        MouseWheelName \t= \"MOUSE WHEEL\";\n\
        MouseLRName\t\t= \"Mouse LR\";\n\
        MouseUDName \t= \"Mouse UD\";\n";
    const DEFINES: &str = "#define GCID_UNASSIGNED\t\t\"<non assegnato>\"\n\
        #define GCID_SEPARATOR\t\t\" o \"\n";

    fn names() -> KeyNames {
        KeyNames::parse(INPUT, DEFINES)
    }

    #[test]
    fn defaults_bind_use_to_e_and_enter() {
        let use_slot = Bindings::retail_defaults().slots[13];
        assert_eq!(use_slot.buttons[0], Some(Button::Key(0x12)));
        assert_eq!(use_slot.buttons[2], Some(Button::Key(0x1c)));
        assert_eq!(use_slot.buttons[1], None);
    }

    #[test]
    fn defaults_put_bullet_time_on_the_second_mouse_button_and_shift() {
        let slot = Bindings::retail_defaults().slots[31];
        assert_eq!(slot.buttons[0], Some(Button::Mouse(1)));
        assert_eq!(slot.buttons[2], Some(Button::Key(0x2a)));
    }

    #[test]
    fn defaults_give_weapon_scrolling_the_wheel() {
        let slot = Bindings::retail_defaults().slots[26];
        assert_eq!(slot.analog, Some(Analog::MouseWheel));
        assert_eq!(slot.buttons[1], Some(Button::Key(0x1b)));
    }

    #[test]
    fn names_come_from_input_txt_then_the_us_layout() {
        let names = names();
        assert_eq!(names.button(Button::Key(200)), "UP");
        assert_eq!(names.button(Button::Key(0x2a)), "Shift");
        assert_eq!(names.button(Button::Key(0x12)), "E");
        assert_eq!(names.button(Button::Mouse(1)), "MB 2");
    }

    #[test]
    fn a_token_names_both_buttons_of_its_direction_with_the_separator() {
        let text = expand_tokens(
            "il tasto #BULLETTIME#...",
            &Bindings::retail_defaults(),
            &names(),
        );
        assert_eq!(text, "il tasto MB 2 o Shift...");
    }

    #[test]
    fn a_direction_picks_its_own_buttons() {
        let b = Bindings::retail_defaults();
        assert_eq!(expand_tokens("#WALKBACK#", &b, &names()), "S o DOWN");
        assert_eq!(expand_tokens("#SELECTWEAPONSLOT2#", &b, &names()), "3");
    }

    #[test]
    fn an_analog_source_follows_the_buttons() {
        let b = Bindings::retail_defaults();
        assert_eq!(
            expand_tokens("#NEXTWEAPON#", &b, &names()),
            "[ o MOUSE WHEEL"
        );
    }

    #[test]
    fn a_slot_with_nothing_bound_prints_unassigned() {
        let b = Bindings::retail_defaults();
        assert_eq!(
            expand_tokens("#DODGELEFT#", &b, &names()),
            "<non assegnato>"
        );
    }

    #[test]
    fn unknown_tokens_stay() {
        let b = Bindings::retail_defaults();
        assert_eq!(expand_tokens("#TOTALTIME#", &b, &names()), "#TOTALTIME#");
    }

    #[test]
    fn the_registry_overrides_a_slot() {
        let reg = "[Software\\\\Remedy Entertainment\\\\Max Payne\\\\Input Settings] 1793319803\n\
            #time=1dd6804dfc8787a\n\
            \"13 Button0 type\"=dword:00000001\n\
            \"13 Button0 value\"=dword:00000021\n\
            \"13 Button2 type\"=dword:00000000\n\
            \"13 Button2 value\"=dword:00000000\n\
            \" 2 Button0 type\"=dword:00000002\n\
            \" 2 Button0 value\"=dword:00000000\n\
            \"26 Slider type\"=dword:00000003\n\
            \n[Software\\\\Other]\n\
            \"13 Button1 type\"=dword:00000001\n";
        let b = Bindings::from_wine_registry(reg).unwrap();
        assert_eq!(b.slots[13].buttons[0], Some(Button::Key(0x21)));
        assert_eq!(b.slots[13].buttons[2], None);
        assert_eq!(b.slots[13].buttons[1], None);
        assert_eq!(b.slots[2].buttons[0], Some(Button::Mouse(0)));
        assert_eq!(b.slots[26].analog, Some(Analog::MouseWheel));
    }

    #[test]
    fn no_input_section_means_no_registry_bindings() {
        assert_eq!(Bindings::from_wine_registry("[Software\\\\Other]\n"), None);
    }
}
