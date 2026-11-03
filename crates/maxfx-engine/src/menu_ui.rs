//! Keyboard main menu drawn after the intro. Retail actions come from `menu.txt`.

use crate::frontend::{Frontend, Screen};
use crate::{novel_input, novel_turn, MenuEntry, NovelShownPage};
use minifb::{Key, KeyRepeat, MouseButton};
use std::io::Write;
use std::process::{Child, Command, Stdio};

const W: usize = 960;
const H: usize = 600;

/// `Some(level id)` from `gm_init`, or `None` when the player quits.
pub fn run_main_menu(screen: &mut Screen, root: &[MenuEntry]) -> Option<String> {
    screen.set_title("Open Max Payne");
    let mut stack: Vec<(Vec<MenuEntry>, usize)> = vec![(root.to_vec(), 0)];
    let mut buf = vec![0u32; W * H];
    while screen.is_open() {
        screen.pump();
        let (entries, sel) = stack.last_mut().unwrap();
        let n = entries.len().max(1);
        if screen.is_key_pressed(Key::Down, KeyRepeat::No) && !entries.is_empty() {
            *sel = (*sel + 1) % n;
        }
        if screen.is_key_pressed(Key::Up, KeyRepeat::No) && !entries.is_empty() {
            *sel = (*sel + n - 1) % n;
        }
        if screen.is_key_pressed(Key::Enter, KeyRepeat::No) && !entries.is_empty() {
            let entry = entries[*sel].clone();
            if !entry.children.is_empty() {
                stack.push((entry.children, 0));
            } else if entry.quits || entry.id == "GCID_QUIT" {
                return None;
            } else if let Some(id) = entry.level_id {
                return Some(id);
            } else if (entry.id == "GCID_QUIT_NO" || entry.id == "GCID_BACK") && stack.len() > 1 {
                stack.pop();
            }
        }
        if screen.is_key_pressed(Key::Escape, KeyRepeat::No)
            || screen.is_key_pressed(Key::Backspace, KeyRepeat::No)
        {
            if stack.len() > 1 {
                stack.pop();
            } else {
                return None;
            }
        }
        paint(&mut buf, stack.last().unwrap());
        if screen.show(&buf).is_err() {
            return None;
        }
    }
    None
}

/// Collected pages, then the exit-to-game indicator. A bad jpeg draws the page id.
pub fn show_novel_session(screen: &mut Screen, chrome: &[u8], pages: &[NovelShownPage]) {
    if pages.is_empty() {
        return;
    }
    screen.set_title("Open Max Payne — graphic novel");
    let mut index = 0usize;
    let mut buf = vec![0u32; W * H];
    let mut voice: Option<Child> = None;
    let mut shown = usize::MAX;
    let mut mouse_down = false;
    while screen.is_open() {
        screen.pump();
        if shown != index {
            stop_voice(&mut voice);
            voice = start_voice(&pages[index].wav);
            shown = index;
        }
        paint_novel_page(&mut buf, &pages[index].jpeg, &pages[index].id, chrome);
        let click = screen.get_mouse_down(MouseButton::Left);
        let click_edge = click && !mouse_down;
        mouse_down = click;
        let advance = screen.is_key_pressed(Key::Right, KeyRepeat::No)
            || screen.is_key_pressed(Key::Space, KeyRepeat::No)
            || screen.is_key_pressed(Key::Enter, KeyRepeat::No)
            || click_edge;
        let back = screen.is_key_pressed(Key::Left, KeyRepeat::No);
        let escape = screen.is_key_pressed(Key::Escape, KeyRepeat::No);
        if let Some(input) = novel_input(escape, advance, back) {
            match novel_turn(index, pages.len(), input) {
                Some(next) => index = next,
                None => break,
            }
        }
        if screen.show(&buf).is_err() {
            break;
        }
    }
    stop_voice(&mut voice);
}

/// The mode music (`a_playmusic` in `menu.txt` `[OnInit]`), looping until another cue replaces it.
/// Dropping it stops the music.
#[derive(Default)]
pub struct Music {
    cue: Option<String>,
    child: Option<Child>,
}

impl Music {
    /// Play `mode`'s music (`menumode`, `graphicnovelmode`); a mode without music stops it,
    /// as `LevelOnInitMessages` does when a level starts (`a_playmusic("")`).
    pub fn play_mode(&mut self, game_dir: &std::path::Path, mode: &str) {
        let cue = crate::read_game_bytes(game_dir, r"\data\menu\menu.txt")
            .and_then(|bytes| crate::mode_music_cue(&String::from_utf8_lossy(&bytes), mode));
        self.play(game_dir, cue.as_deref());
    }

    /// Play a `music.txt` cue from a level script; `""` stops the music.
    pub fn play_cue(&mut self, game_dir: &std::path::Path, cue: &str) {
        self.play(game_dir, Some(cue).filter(|cue| !cue.is_empty()));
    }

    fn play(&mut self, game_dir: &std::path::Path, cue: Option<&str>) {
        if self.cue.as_deref() == cue {
            return;
        }
        stop_voice(&mut self.child);
        self.cue = cue.map(str::to_string);
        let Some(cue) = cue else {
            return;
        };
        let music_txt = crate::read_game_bytes(game_dir, r"\data\database\music\music.txt")
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let wav = crate::music_win_path(&music_txt, cue)
            .and_then(|win| crate::read_game_bytes(game_dir, &win));
        let Some(wav) = wav else {
            eprintln!("music {cue}: not found");
            return;
        };
        let path = std::env::temp_dir().join(format!("omp-music-{}-{cue}.wav", std::process::id()));
        if std::fs::write(&path, wav).is_err() {
            return;
        }
        self.child = Command::new("ffplay")
            .args(crate::render3d::sound_player_args(
                &path,
                crate::sound_cue_looping(&music_txt, cue),
                100,
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();
    }
}

impl Drop for Music {
    fn drop(&mut self) {
        stop_voice(&mut self.child);
    }
}

pub(crate) fn start_voice(wav: &[u8]) -> Option<Child> {
    if wav.len() < 12 || &wav[..4] != b"RIFF" {
        return None;
    }
    let path = std::env::temp_dir().join(format!("omp-story-{}.wav", std::process::id()));
    let mut file = std::fs::File::create(&path).ok()?;
    file.write_all(wav).ok()?;
    drop(file);
    for (bin, quiet) in [("paplay", false), ("aplay", true)] {
        let mut cmd = Command::new(bin);
        if quiet {
            cmd.arg("-q");
        }
        cmd.arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(child) = cmd.spawn() {
            return Some(child);
        }
    }
    None
}

pub(crate) fn stop_voice(voice: &mut Option<Child>) {
    if let Some(child) = voice.take() {
        let mut child = child;
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn paint_novel_page(buf: &mut [u32], jpeg: &[u8], page: &str, chrome: &[u8]) {
    buf.fill(0x0012100e);
    let band = decode_jpeg_rgb(chrome)
        .map(|(w, h, _)| chrome_band_height(w, h))
        .unwrap_or(28);
    let view_h = H.saturating_sub(band);
    if let Some((w, h, rgb)) = decode_jpeg_rgb(jpeg) {
        blit_rgb_letterbox(buf, &rgb, w, h, view_h);
    } else {
        blit(buf, 48, 48, &page.to_ascii_uppercase(), 0x00f0e6c8, 3);
    }
    if let Some((w, h, rgb)) = decode_jpeg_rgb(chrome) {
        blit_rgb_stretch(buf, &rgb, w, h, (0, view_h, W, band));
    } else {
        hbar(buf, 0, view_h, W, band, 0x00181010);
        blit(buf, 48, view_h + 6, "ENTER", 0x00887870, 2);
    }
}

fn chrome_band_height(src_w: usize, src_h: usize) -> usize {
    if src_w == 0 || src_h == 0 {
        return 28;
    }
    let h = ((src_h as f32) * (W as f32 / src_w as f32)).round() as usize;
    h.clamp(1, H / 3)
}

fn decode_jpeg_rgb(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    if bytes.is_empty() {
        return None;
    }
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let w = info.width as usize;
    let h = info.height as usize;
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels,
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|p| [*p, *p, *p]).collect(),
        _ => return None,
    };
    if w == 0 || h == 0 || rgb.len() < w * h * 3 {
        None
    } else {
        Some((w, h, rgb))
    }
}

fn blit_rgb_stretch(
    buf: &mut [u32],
    rgb: &[u8],
    src_w: usize,
    src_h: usize,
    dest: (usize, usize, usize, usize),
) {
    let (ox, oy, dw, dh) = dest;
    if src_w == 0 || src_h == 0 || dw == 0 || dh == 0 {
        return;
    }
    for y in 0..dh {
        let py = oy + y;
        if py >= H {
            break;
        }
        let sy = y * src_h / dh;
        for x in 0..dw {
            let px = ox + x;
            if px >= W {
                break;
            }
            let sx = x * src_w / dw;
            let i = (sy * src_w + sx) * 3;
            if i + 2 >= rgb.len() {
                continue;
            }
            buf[py * W + px] =
                ((rgb[i] as u32) << 16) | ((rgb[i + 1] as u32) << 8) | rgb[i + 2] as u32;
        }
    }
}

fn blit_rgb_letterbox(buf: &mut [u32], rgb: &[u8], src_w: usize, src_h: usize, view_h: usize) {
    if src_w == 0 || src_h == 0 || view_h == 0 {
        return;
    }
    let scale = (W as f32 / src_w as f32).min(view_h as f32 / src_h as f32);
    let dw = ((src_w as f32 * scale) as usize).clamp(1, W);
    let dh = ((src_h as f32 * scale) as usize).clamp(1, view_h);
    let ox = (W - dw) / 2;
    let oy = (view_h - dh) / 2;
    for y in 0..dh {
        let sy = y * src_h / dh;
        for x in 0..dw {
            let sx = x * src_w / dw;
            let i = (sy * src_w + sx) * 3;
            if i + 2 >= rgb.len() {
                continue;
            }
            let px = ((rgb[i] as u32) << 16) | ((rgb[i + 1] as u32) << 8) | rgb[i + 2] as u32;
            buf[(oy + y) * W + ox + x] = px;
        }
    }
}

fn paint(buf: &mut [u32], (entries, sel): &(Vec<MenuEntry>, usize)) {
    buf.fill(0x0012100e);
    blit(buf, 48, 48, "MAX PAYNE", 0x00f0e6c8, 3);
    blit(buf, 48, 88, "UP DOWN  ENTER  ESC", 0x00887870, 2);
    for (i, entry) in entries.iter().enumerate() {
        let y = 150 + i * 36;
        let on = i == *sel;
        if on {
            hbar(buf, 40, y - 6, 520, 32, 0x003a2418);
        }
        let mut label = entry.label.clone();
        if !entry.children.is_empty() {
            label.push_str("  >");
        }
        blit(
            buf,
            56,
            y,
            &label.to_ascii_uppercase(),
            if on { 0x00f0e6c8 } else { 0x00b0a898 },
            2,
        );
    }
}

fn hbar(buf: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
    for row in y..y + h {
        if row >= H {
            break;
        }
        for col in x..x + w {
            if col >= W {
                break;
            }
            buf[row * W + col] = color;
        }
    }
}

fn blit(buf: &mut [u32], x: usize, y: usize, text: &str, color: u32, scale: usize) {
    let mut cx = x;
    for ch in text.chars() {
        let g = glyph(ch);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) == 0 {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px = cx + col * scale + sx;
                        let py = y + row * scale + sy;
                        if px < W && py < H {
                            buf[py * W + px] = color;
                        }
                    }
                }
            }
        }
        cx += 6 * scale;
    }
}

fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b11110, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'H' => [
            0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b11100, 0b10010, 0b10001, 0b10001, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10001, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10010, 0b10001, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b00001, 0b00001, 0b00001,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b01110, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b01110, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        '>' => [
            0b10000, 0b01000, 0b00100, 0b00010, 0b00100, 0b01000, 0b10000,
        ],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_page_draws_the_page_id() {
        let mut buf = vec![0u32; W * H];
        paint_novel_page(&mut buf, b"not-a-jpeg", "P1L0A_001", b"");
        assert_eq!(buf[48 * W + 48], 0x00f0e6c8);
    }

    #[test]
    fn letterbox_centers_a_red_pixel() {
        let mut buf = vec![0u32; W * H];
        blit_rgb_letterbox(&mut buf, &[255, 0, 0], 1, 1, H - 28);
        assert_eq!(buf[(H / 2) * W + W / 2], 0x00ff0000);
    }

    #[test]
    fn chrome_band_sits_under_the_page() {
        let mut buf = vec![0u32; W * H];
        let band = chrome_band_height(640, 100);
        assert_eq!(band, 150);
        blit_rgb_stretch(&mut buf, &[0, 0, 255], 1, 1, (0, H - band, W, band));
        assert_eq!(buf[(H - 1) * W + W / 2], 0x000000ff);
        assert_eq!(buf[(H - band - 1) * W + W / 2], 0);
    }
}
