//! Window and input for the walkthrough: the minifb window (software renderer) or a winit
//! window (hardware renderer), both answering in minifb's key vocabulary.

use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::platform::pump_events::EventLoopExtPumpEvents;
use winit::window::WindowId;

/// What the walkthrough loop asks of its window.
pub trait Frontend {
    fn is_open(&self) -> bool;
    fn is_key_down(&self, key: Key) -> bool;
    fn is_key_pressed(&self, key: Key, repeat: KeyRepeat) -> bool;
    fn get_mouse_down(&self, button: MouseButton) -> bool;
    /// Cursor in the 960 × 600 frame's coordinates.
    fn get_mouse_pos(&self, mode: MouseMode) -> Option<(f32, f32)>;
}

impl Frontend for Window {
    fn is_open(&self) -> bool {
        Window::is_open(self)
    }
    fn is_key_down(&self, key: Key) -> bool {
        Window::is_key_down(self, key)
    }
    fn is_key_pressed(&self, key: Key, repeat: KeyRepeat) -> bool {
        Window::is_key_pressed(self, key, repeat)
    }
    fn get_mouse_down(&self, button: MouseButton) -> bool {
        Window::get_mouse_down(self, button)
    }
    fn get_mouse_pos(&self, mode: MouseMode) -> Option<(f32, f32)> {
        Window::get_mouse_pos(self, mode)
    }
}

thread_local! {
    /// winit allows one event loop per process; the walkthrough runs more than once.
    static EVENT_LOOP: RefCell<Option<EventLoop<()>>> = const { RefCell::new(None) };
}

/// Keyboard and mouse state collected from winit events.
#[derive(Default)]
struct InputState {
    open: bool,
    down: BTreeSet<u32>,
    pressed: BTreeSet<u32>,
    mouse_left: bool,
    mouse_right: bool,
    cursor: Option<(f64, f64)>,
    size: (u32, u32),
}

impl ApplicationHandler for InputState {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.open = false,
            WindowEvent::Resized(size) => self.size = (size.width, size.height),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let Some(key) = minifb_key(code) else {
                    return;
                };
                let id = key as u32;
                if event.state == ElementState::Pressed {
                    if !event.repeat {
                        self.pressed.insert(id);
                    }
                    self.down.insert(id);
                } else {
                    self.down.remove(&id);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                match button {
                    winit::event::MouseButton::Left => self.mouse_left = down,
                    winit::event::MouseButton::Right => self.mouse_right = down,
                    _ => {}
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some((position.x, position.y));
            }
            _ => {}
        }
    }
}

/// A resizable winit window for the hardware renderer.
pub struct WinitFrontend {
    pub window: Arc<winit::window::Window>,
    input: InputState,
}

impl WinitFrontend {
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self, String> {
        let window = EVENT_LOOP.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                *slot = Some(EventLoop::new().map_err(|e| e.to_string())?);
            }
            let event_loop = slot.as_ref().ok_or("no event loop")?;
            let attributes = winit::window::Window::default_attributes()
                .with_title(title)
                .with_inner_size(winit::dpi::LogicalSize::new(width, height));
            #[allow(deprecated)]
            event_loop
                .create_window(attributes)
                .map_err(|e| e.to_string())
        })?;
        let size = window.inner_size();
        Ok(Self {
            window: Arc::new(window),
            input: InputState {
                open: true,
                size: (size.width, size.height),
                ..Default::default()
            },
        })
    }

    /// The window's display handle, for creating the GPU instance.
    pub fn display_handle(&self) -> Option<winit::event_loop::OwnedDisplayHandle> {
        EVENT_LOOP.with(|cell| cell.borrow().as_ref().map(EventLoop::owned_display_handle))
    }

    /// Drawable size in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.input.size
    }

    /// Collect the events that arrived since the last frame.
    pub fn pump(&mut self) {
        self.input.pressed.clear();
        EVENT_LOOP.with(|cell| {
            if let Some(event_loop) = cell.borrow_mut().as_mut() {
                event_loop.pump_app_events(Some(Duration::ZERO), &mut self.input);
            }
        });
    }
}

impl Frontend for WinitFrontend {
    fn is_open(&self) -> bool {
        self.input.open
    }
    fn is_key_down(&self, key: Key) -> bool {
        self.input.down.contains(&(key as u32))
    }
    fn is_key_pressed(&self, key: Key, _repeat: KeyRepeat) -> bool {
        self.input.pressed.contains(&(key as u32))
    }
    fn get_mouse_down(&self, button: MouseButton) -> bool {
        match button {
            MouseButton::Left => self.input.mouse_left,
            MouseButton::Right => self.input.mouse_right,
            MouseButton::Middle => false,
        }
    }
    fn get_mouse_pos(&self, _mode: MouseMode) -> Option<(f32, f32)> {
        let (x, y) = self.input.cursor?;
        let (w, h) = self.input.size;
        let sx = crate::render3d::FRAME_W as f64 / f64::from(w.max(1));
        let sy = crate::render3d::FRAME_H as f64 / f64::from(h.max(1));
        Some(((x * sx) as f32, (y * sy) as f32))
    }
}

/// winit physical key → the minifb key the walkthrough checks.
pub(crate) fn minifb_key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::KeyA => Key::A,
        KeyCode::KeyB => Key::B,
        KeyCode::KeyC => Key::C,
        KeyCode::KeyD => Key::D,
        KeyCode::KeyE => Key::E,
        KeyCode::KeyF => Key::F,
        KeyCode::KeyG => Key::G,
        KeyCode::KeyL => Key::L,
        KeyCode::KeyS => Key::S,
        KeyCode::KeyV => Key::V,
        KeyCode::KeyW => Key::W,
        KeyCode::ArrowUp => Key::Up,
        KeyCode::ArrowDown => Key::Down,
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::Space => Key::Space,
        KeyCode::Tab => Key::Tab,
        KeyCode::Escape => Key::Escape,
        KeyCode::F1 => Key::F1,
        KeyCode::ShiftLeft => Key::LeftShift,
        KeyCode::ShiftRight => Key::RightShift,
        KeyCode::ControlLeft => Key::LeftCtrl,
        KeyCode::ControlRight => Key::RightCtrl,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_walkthrough_key_has_a_winit_code() {
        // The keys render3d's loop reads (WASD, arrows, actions, toggles).
        let codes = [
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
            KeyCode::KeyE,
            KeyCode::KeyF,
            KeyCode::KeyV,
            KeyCode::KeyG,
            KeyCode::KeyL,
            KeyCode::KeyB,
            KeyCode::KeyC,
            KeyCode::Tab,
            KeyCode::Space,
            KeyCode::Escape,
            KeyCode::F1,
            KeyCode::ShiftLeft,
            KeyCode::ControlLeft,
            KeyCode::ArrowUp,
        ];
        let keys: Vec<_> = codes.into_iter().map(minifb_key).collect();
        assert!(keys.iter().all(Option::is_some));
        assert_eq!(minifb_key(KeyCode::KeyW), Some(Key::W));
        assert_eq!(minifb_key(KeyCode::ControlRight), Some(Key::RightCtrl));
        assert_eq!(minifb_key(KeyCode::KeyZ), None);
    }
}
