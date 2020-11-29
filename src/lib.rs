#![deny(missing_docs)]

//! A Glutin window back-end for the Piston game engine.

mod self_time;

extern crate gl;
extern crate glutin;
extern crate input;
extern crate shader_version;
extern crate window;

use glutin::platform::desktop::EventLoopExtDesktop;
use std::collections::VecDeque;
use std::error::Error;

// External crates.
use glutin::GlRequest;
use input::{
    keyboard, Button, ButtonArgs, ButtonState, CloseArgs, Event, FileDrag, Input, MouseButton,
    ResizeArgs,
};
use std::thread;
use std::time::Duration;
use window::{
    AdvancedWindow, Api, BuildFromWindowSettings, OpenGLWindow, Position, ProcAddress, Size,
    UnsupportedGraphicsApiError, Window, WindowSettings,
};

use glutin::dpi::{LogicalPosition, LogicalSize, PhysicalSize};
use glutin::event::{MouseScrollDelta, VirtualKeyCode, WindowEvent};
use glutin::window::Fullscreen;
use self_time::SelfTime;
pub use shader_version::OpenGL;
use std::any::Any;
use std::mem::MaybeUninit;

/// Contains stuff for game window.
pub struct GlutinWindow<'a> {
    /// The window.
    pub ctx: glutin::ContextWrapper<glutin::PossiblyCurrent, glutin::window::Window>,
    // The back-end does not remember the title.
    title: String,
    exit_on_esc: bool,
    should_close: bool,
    automatic_close: bool,
    // Used to fake capturing of cursor,
    // to get relative mouse events.
    is_capturing_cursor: bool,
    // Stores the last known cursor position.
    last_cursor_pos: Option<[f64; 2]>,
    // Stores relative coordinates to emit on next poll.
    mouse_relative: Option<(f64, f64)>,
    // Used to emit cursor event after enter/leave.
    cursor_pos: Option<[f64; 2]>,
    // Polls events from window.
    events_loop: glutin::event_loop::EventLoop<()>,
    // Stores list of events ready for processing.
    events: VecDeque<glutin::event::Event<'a, ()>>,

    self_time: SelfTime<'a>,
}

fn window_builder_from_settings(settings: &WindowSettings) -> glutin::window::WindowBuilder {
    let size = settings.get_size();
    let mut builder = glutin::window::WindowBuilder::new()
        .with_inner_size(glutin::dpi::Size::Logical(LogicalSize {
            width: size.width,
            height: size.height,
        }))
        .with_decorations(settings.get_decorated())
        .with_title(settings.get_title())
        .with_resizable(settings.get_resizable());
    if settings.get_fullscreen() {
        let events_loop = glutin::event_loop::EventLoop::new();
        builder =
            builder.with_fullscreen(Some(Fullscreen::Borderless(events_loop.primary_monitor())));
    }
    builder
}

fn context_builder_from_settings(
    settings: &WindowSettings,
) -> Result<glutin::ContextBuilder<glutin::NotCurrent>, Box<dyn Error>> {
    let api = settings
        .get_maybe_graphics_api()
        .unwrap_or(Api::opengl(3, 2));
    if api.api != "OpenGL" {
        return Err(UnsupportedGraphicsApiError {
            found: api.api,
            expected: vec!["OpenGL".into()],
        }
        .into());
    };
    let mut builder = glutin::ContextBuilder::new()
        .with_gl(GlRequest::GlThenGles {
            opengl_version: (api.major as u8, api.minor as u8),
            opengles_version: (api.major as u8, api.minor as u8),
        })
        .with_srgb(settings.get_srgb());
    let samples = settings.get_samples();
    if settings.get_vsync() {
        builder = builder.with_vsync(true);
    }
    if samples != 0 {
        builder = builder.with_multisampling(samples as u16);
    }
    Ok(builder)
}
fn fix_event_lifetime<'a>(
    self_time: &mut SelfTime<'a>,
    mut ev: glutin::event::Event<'_, ()>,
) -> glutin::event::Event<'a, ()> {
    use glutin::event::Event;
    if let Event::WindowEvent { event, .. } = &mut ev {
        if let WindowEvent::ScaleFactorChanged { new_inner_size, .. } = event {
            *new_inner_size = unsafe {
                std::mem::transmute(self_time.push(PhysicalSize {
                    width: new_inner_size.width,
                    height: new_inner_size.height,
                }))
            };
        }
    }

    unsafe { std::mem::transmute(ev) }
}
impl<'a> GlutinWindow<'a> {
    /// Creates a new game window for Glutin.
    pub fn new(settings: &WindowSettings) -> Result<Self, Box<dyn Error>> {
        let events_loop = glutin::event_loop::EventLoop::new();
        let title = settings.get_title();
        let exit_on_esc = settings.get_exit_on_esc();
        let window_builder = window_builder_from_settings(&settings);
        let context_builder = context_builder_from_settings(&settings)?;
        let ctx = context_builder.build_windowed(window_builder, &events_loop);
        let ctx = match ctx {
            Ok(ctx) => ctx,
            Err(_) => {
                let settings = settings.clone().samples(0);
                let window_builder = window_builder_from_settings(&settings);
                let context_builder = context_builder_from_settings(&settings)?;
                let ctx = context_builder.build_windowed(window_builder, &events_loop)?;
                ctx
            }
        };
        let ctx = unsafe { ctx.make_current().map_err(|(_, err)| err)? };

        // Load the OpenGL function pointers.
        gl::load_with(|s| ctx.get_proc_address(s) as *const _);

        Ok(GlutinWindow {
            ctx,
            title,
            exit_on_esc,
            should_close: false,
            automatic_close: settings.get_automatic_close(),
            cursor_pos: None,
            is_capturing_cursor: false,
            last_cursor_pos: None,
            mouse_relative: None,
            events_loop,
            events: VecDeque::new(),
            self_time: Default::default(),
        })
    }

    /// Creates a game window from a pre-existing Glutin event loop and window builder.
    pub fn from_raw(
        settings: &WindowSettings,
        events_loop: glutin::event_loop::EventLoop<()>,
        window_builder: glutin::window::WindowBuilder,
    ) -> Result<Self, Box<dyn Error>> {
        let title = settings.get_title();
        let exit_on_esc = settings.get_exit_on_esc();

        let context_builder = context_builder_from_settings(&settings)?;
        let ctx = context_builder.build_windowed(window_builder, &events_loop)?;
        let ctx = unsafe { ctx.make_current().map_err(|(_, err)| err)? };

        // Load the OpenGL function pointers.
        gl::load_with(|s| ctx.get_proc_address(s) as *const _);

        Ok(GlutinWindow {
            ctx,
            title,
            exit_on_esc,
            should_close: false,
            automatic_close: settings.get_automatic_close(),
            cursor_pos: None,
            is_capturing_cursor: false,
            last_cursor_pos: None,
            mouse_relative: None,
            events_loop,
            events: VecDeque::new(),
            self_time: Default::default(),
        })
    }

    fn wait_event(&mut self) -> Event {
        // First check for and handle any pending events.
        if let Some(event) = self.poll_event() {
            return event;
        }
        loop {
            {
                let ref mut events = self.events;
                let ref mut self_time = self.self_time;
                self.events_loop.run_return(|ev, _, result| {
                    events.push_back(fix_event_lifetime(self_time, ev));

                    *result = glutin::event_loop::ControlFlow::Exit;
                });

                if let Some(event) = self.poll_event() {
                    return event;
                }
            }
        }
    }

    fn wait_event_timeout(&mut self, timeout: Duration) -> Option<Event> {
        // First check for and handle any pending events.
        if let Some(event) = self.poll_event() {
            return Some(event);
        }
        // Schedule wake up when time is out.
        let events_loop_proxy = self.events_loop.create_proxy();
        thread::spawn(move || {
            thread::sleep(timeout);
            // Wakeup can fail only if the event loop went away.
            events_loop_proxy.send_event(()).ok();
        });
        {
            let ref mut events = self.events;
            let ref mut self_time = self.self_time;
            self.events_loop.run_return(|ev, _, ret| {
                events.push_back(fix_event_lifetime(self_time, ev));
                *ret = glutin::event_loop::ControlFlow::Exit
            });
        }

        self.poll_event()
    }

    fn poll_event(&mut self) -> Option<Event> {
        use glutin::event::Event as E;
        use glutin::event::WindowEvent as WE;

        // Loop to skip unknown events.
        loop {
            let event = self.pre_pop_front_event();
            if event.is_some() {
                return event.map(|x| Event::Input(x, None));
            }

            if self.events.len() == 0 {
                let ref mut events = self.events;
                let ref mut self_time = self.self_time;
                self.events_loop.run_return(|ev, _, ret| {
                    if let glutin::event::Event::MainEventsCleared = ev {
                        *ret = glutin::event_loop::ControlFlow::Exit;
                    } else {
                        events.push_back(fix_event_lifetime(self_time, ev));
                    }
                });
            }
            let mut ev = self.events.pop_front();

            if self.is_capturing_cursor && self.last_cursor_pos.is_none() {
                if let Some(E::WindowEvent {
                    event: WE::CursorMoved { position, .. },
                    ..
                }) = ev
                {
                    // Ignore this event since mouse positions
                    // should not be emitted when capturing cursor.
                    self.last_cursor_pos = Some([position.x, position.y]);

                    if self.events.len() == 0 {
                        let ref mut events = self.events;
                        let ref mut self_time = self.self_time;
                        self.events_loop.run_return(|ev, _, ret| {
                            if let glutin::event::Event::MainEventsCleared = ev {
                                *ret = glutin::event_loop::ControlFlow::Exit;
                            } else {
                                events.push_back(fix_event_lifetime(self_time, ev));
                            }
                        });
                    }
                    ev = self.events.pop_front();
                }
            }

            let mut unknown = false;
            let event = self.handle_event(ev, &mut unknown);
            if unknown {
                continue;
            };
            return event.map(|x| Event::Input(x, None));
        }
    }

    // These events are emitted before popping a new event from the queue.
    // This is because Piston handles some events separately.
    fn pre_pop_front_event(&mut self) -> Option<Input> {
        use input::Motion;

        // Check for a pending mouse cursor move event.
        if let Some(pos) = self.cursor_pos {
            self.cursor_pos = None;
            return Some(Input::Move(Motion::MouseCursor(pos)));
        }

        // Check for a pending relative mouse move event.
        if let Some((x, y)) = self.mouse_relative {
            self.mouse_relative = None;
            return Some(Input::Move(Motion::MouseRelative([x, y])));
        }

        None
    }

    /// Convert an incoming Glutin event to Piston input.
    /// Update cursor state if necessary.
    ///
    /// The `unknown` flag is set to `true` when the event is not recognized.
    /// This is used to poll another event to make the event loop logic sound.
    /// When `unknown` is `true`, the return value is `None`.
    fn handle_event(
        &mut self,
        ev: Option<glutin::event::Event<impl Any>>,
        unknown: &mut bool,
    ) -> Option<Input> {
        use glutin::event::Event as E;
        use glutin::event::WindowEvent as WE;
        // use glutin::MouseScrollDelta;
        use input::{Key, Motion};

        match ev {
            None => {
                if self.is_capturing_cursor {
                    self.fake_capture();
                }
                None
            }
            Some(E::WindowEvent {
                event: WE::Resized(size),
                ..
            }) => {
                let draw_size = self.draw_size();
                Some(Input::Resize(ResizeArgs {
                    window_size: [size.width.into(), size.height.into()],
                    draw_size: draw_size.into(),
                }))
            }
            Some(E::WindowEvent {
                event: WE::ReceivedCharacter(ch),
                ..
            }) => {
                let string = match ch {
                    // Ignore control characters and return ascii for Text event (like sdl2).
                    '\u{7f}' | // Delete
                    '\u{1b}' | // Escape
                    '\u{8}'  | // Backspace
                    '\r' | '\n' | '\t' => "".to_string(),
                    _ => ch.to_string()
                };
                Some(Input::Text(string))
            }
            Some(E::WindowEvent {
                event: WE::Focused(focused),
                ..
            }) => Some(Input::Focus(focused)),
            Some(E::WindowEvent {
                event:
                    WE::KeyboardInput {
                        input:
                            glutin::event::KeyboardInput {
                                state: glutin::event::ElementState::Pressed,
                                virtual_keycode: Some(key),
                                scancode,
                                ..
                            },
                        ..
                    },
                ..
            }) => {
                let piston_key = map_key(key);
                if let (true, Key::Escape) = (self.exit_on_esc, piston_key) {
                    self.should_close = true;
                }
                Some(Input::Button(ButtonArgs {
                    state: ButtonState::Press,
                    button: Button::Keyboard(piston_key),
                    scancode: Some(scancode as i32),
                }))
            }
            Some(E::WindowEvent {
                event:
                    WE::KeyboardInput {
                        input:
                            glutin::event::KeyboardInput {
                                state: glutin::event::ElementState::Released,
                                virtual_keycode: Some(key),
                                scancode,
                                ..
                            },
                        ..
                    },
                ..
            }) => Some(Input::Button(ButtonArgs {
                state: ButtonState::Release,
                button: Button::Keyboard(map_key(key)),
                scancode: Some(scancode as i32),
            })),
            Some(E::WindowEvent {
                event:
                    WE::Touch(glutin::event::Touch {
                        phase,
                        location,
                        id,
                        ..
                    }),
                ..
            }) => {
                use glutin::event::TouchPhase;
                use input::{Touch, TouchArgs};

                Some(Input::Move(Motion::Touch(TouchArgs::new(
                    0,
                    id as i64,
                    [location.x, location.y],
                    1.0,
                    match phase {
                        TouchPhase::Started => Touch::Start,
                        TouchPhase::Moved => Touch::Move,
                        TouchPhase::Ended => Touch::End,
                        TouchPhase::Cancelled => Touch::Cancel,
                    },
                ))))
            }
            Some(E::WindowEvent {
                event: WE::CursorMoved { position, .. },
                ..
            }) => {
                let x = position.x;
                let y = position.y;

                if let Some(pos) = self.last_cursor_pos {
                    let dx = x - pos[0];
                    let dy = y - pos[1];
                    if self.is_capturing_cursor {
                        self.last_cursor_pos = Some([x, y]);
                        self.fake_capture();
                        // Skip normal mouse movement and emit relative motion only.
                        return Some(Input::Move(Motion::MouseRelative([dx as f64, dy as f64])));
                    }
                    // Send relative mouse movement next time.
                    self.mouse_relative = Some((dx as f64, dy as f64));
                }

                self.last_cursor_pos = Some([x, y]);
                Some(Input::Move(Motion::MouseCursor([x, y])))
            }
            Some(E::WindowEvent {
                event: WE::CursorEntered { .. },
                ..
            }) => Some(Input::Cursor(true)),
            Some(E::WindowEvent {
                event: WE::CursorLeft { .. },
                ..
            }) => Some(Input::Cursor(false)),
            Some(E::WindowEvent {
                event:
                    WE::MouseWheel {
                        delta: MouseScrollDelta::PixelDelta(pos),
                        ..
                    },
                ..
            }) => Some(Input::Move(Motion::MouseScroll([
                pos.x as f64,
                pos.y as f64,
            ]))),
            Some(E::WindowEvent {
                event:
                    WE::MouseWheel {
                        delta: MouseScrollDelta::LineDelta(x, y),
                        ..
                    },
                ..
            }) => Some(Input::Move(Motion::MouseScroll([x as f64, y as f64]))),
            Some(E::WindowEvent {
                event:
                    WE::MouseInput {
                        state: glutin::event::ElementState::Pressed,
                        button,
                        ..
                    },
                ..
            }) => Some(Input::Button(ButtonArgs {
                state: ButtonState::Press,
                button: Button::Mouse(map_mouse(button)),
                scancode: None,
            })),
            Some(E::WindowEvent {
                event:
                    WE::MouseInput {
                        state: glutin::event::ElementState::Released,
                        button,
                        ..
                    },
                ..
            }) => Some(Input::Button(ButtonArgs {
                state: ButtonState::Release,
                button: Button::Mouse(map_mouse(button)),
                scancode: None,
            })),
            Some(E::WindowEvent {
                event: WE::HoveredFile(path),
                ..
            }) => Some(Input::FileDrag(FileDrag::Hover(path))),
            Some(E::WindowEvent {
                event: WE::DroppedFile(path),
                ..
            }) => Some(Input::FileDrag(FileDrag::Drop(path))),
            Some(E::WindowEvent {
                event: WE::HoveredFileCancelled,
                ..
            }) => Some(Input::FileDrag(FileDrag::Cancel)),
            Some(E::WindowEvent {
                event: WE::CloseRequested,
                ..
            }) => {
                if self.automatic_close {
                    self.should_close = true;
                }
                Some(Input::Close(CloseArgs))
            }
            _ => {
                *unknown = true;
                None
            }
        }
    }

    fn fake_capture(&mut self) {
        if let Some(pos) = self.last_cursor_pos {
            // Fake capturing of cursor.
            let size = self.size();
            let cx = size.width / 2.0;
            let cy = size.height / 2.0;
            let dx = cx - pos[0];
            let dy = cy - pos[1];
            if dx != 0.0 || dy != 0.0 {
                if let Ok(_) =
                    self.ctx
                        .window()
                        .set_cursor_position(glutin::dpi::Position::Logical(LogicalPosition {
                            x: size.width,
                            y: size.height,
                        }))
                {
                    self.last_cursor_pos = Some([cx, cy]);
                }
            }
        }
    }
}

impl Window for GlutinWindow<'_> {
    fn size(&self) -> Size {
        let size = self.ctx.window().inner_size();
        (size.width, size.height).into()
    }
    fn draw_size(&self) -> Size {
        let size = self.ctx.window().inner_size();
        (size.width, size.height).into()
    }
    fn should_close(&self) -> bool {
        self.should_close
    }
    fn set_should_close(&mut self, value: bool) {
        self.should_close = value;
    }
    fn swap_buffers(&mut self) {
        let _ = self.ctx.swap_buffers();
    }
    fn wait_event(&mut self) -> Event {
        self.wait_event()
    }
    fn wait_event_timeout(&mut self, timeout: Duration) -> Option<Event> {
        self.wait_event_timeout(timeout)
    }
    fn poll_event(&mut self) -> Option<Event> {
        self.poll_event()
    }
}

impl BuildFromWindowSettings for GlutinWindow<'_> {
    fn build_from_window_settings(settings: &WindowSettings) -> Result<Self, Box<dyn Error>> {
        GlutinWindow::new(settings)
    }
}

impl AdvancedWindow for GlutinWindow<'_> {
    fn get_title(&self) -> String {
        self.title.clone()
    }
    fn set_title(&mut self, value: String) {
        self.title = value;
        self.ctx.window().set_title(&self.title);
    }
    fn get_exit_on_esc(&self) -> bool {
        self.exit_on_esc
    }
    fn set_exit_on_esc(&mut self, value: bool) {
        self.exit_on_esc = value;
    }
    fn get_automatic_close(&self) -> bool {
        self.automatic_close
    }
    fn set_automatic_close(&mut self, value: bool) {
        self.automatic_close = value;
    }
    fn set_capture_cursor(&mut self, value: bool) {
        // Normally we would call `.grab_cursor(true)`
        // but since relative mouse events does not work,
        // the capturing of cursor is faked by hiding the cursor
        // and setting the position to the center of window.
        self.is_capturing_cursor = value;
        self.ctx.window().set_cursor_visible(false);
        if value {
            self.fake_capture();
        }
    }
    fn show(&mut self) {
        self.ctx.window().set_visible(true);
    }
    fn hide(&mut self) {
        self.ctx.window().set_visible(false);
    }
    fn get_position(&self) -> Option<Position> {
        let pos = self.ctx.window().outer_position().unwrap();
        return Some(Position {
            x: pos.x as i32,
            y: pos.y as i32,
        });
    }
    fn set_position<P: Into<Position>>(&mut self, pos: P) {
        let pos: Position = pos.into();
        self.ctx
            .window()
            .set_outer_position(glutin::dpi::LogicalPosition::new(pos.x, pos.y));
    }
    fn set_size<S: Into<Size>>(&mut self, size: S) {
        let size: Size = size.into();
        self.ctx
            .window()
            .set_inner_size(glutin::dpi::LogicalSize::new(size.width, size.height));
    }
}

impl OpenGLWindow for GlutinWindow<'_> {
    fn get_proc_address(&mut self, proc_name: &str) -> ProcAddress {
        self.ctx.get_proc_address(proc_name) as *const _
    }

    fn is_current(&self) -> bool {
        self.ctx.is_current()
    }

    #[allow(invalid_value)]
    fn make_current(&mut self) {
        use std::mem::{forget, replace};

        let ctx = replace(&mut self.ctx, unsafe {
            MaybeUninit::<glutin::ContextWrapper<glutin::PossiblyCurrent, glutin::window::Window>>::zeroed().assume_init()
        });
        forget(replace(&mut self.ctx, unsafe {
            ctx.make_current().unwrap()
        }));
    }
}

/// Maps Glutin's key to Piston's key.
pub fn map_key(keycode: glutin::event::VirtualKeyCode) -> keyboard::Key {
    use glutin::event::VirtualKeyCode as K;
    use input::keyboard::Key;

    match keycode {
        K::Key0 => Key::D0,
        K::Key1 => Key::D1,
        K::Key2 => Key::D2,
        K::Key3 => Key::D3,
        K::Key4 => Key::D4,
        K::Key5 => Key::D5,
        K::Key6 => Key::D6,
        K::Key7 => Key::D7,
        K::Key8 => Key::D8,
        K::Key9 => Key::D9,
        K::A => Key::A,
        K::B => Key::B,
        K::C => Key::C,
        K::D => Key::D,
        K::E => Key::E,
        K::F => Key::F,
        K::G => Key::G,
        K::H => Key::H,
        K::I => Key::I,
        K::J => Key::J,
        K::K => Key::K,
        K::L => Key::L,
        K::M => Key::M,
        K::N => Key::N,
        K::O => Key::O,
        K::P => Key::P,
        K::Q => Key::Q,
        K::R => Key::R,
        K::S => Key::S,
        K::T => Key::T,
        K::U => Key::U,
        K::V => Key::V,
        K::W => Key::W,
        K::X => Key::X,
        K::Y => Key::Y,
        K::Z => Key::Z,
        K::Apostrophe => Key::Unknown,
        K::Backslash => Key::Backslash,
        K::Back => Key::Backspace,
        // K::CapsLock => Key::CapsLock,
        K::Delete => Key::Delete,
        K::Comma => Key::Comma,
        K::Down => Key::Down,
        K::End => Key::End,
        K::Return => Key::Return,
        K::Equals => Key::Equals,
        K::Escape => Key::Escape,
        K::F1 => Key::F1,
        K::F2 => Key::F2,
        K::F3 => Key::F3,
        K::F4 => Key::F4,
        K::F5 => Key::F5,
        K::F6 => Key::F6,
        K::F7 => Key::F7,
        K::F8 => Key::F8,
        K::F9 => Key::F9,
        K::F10 => Key::F10,
        K::F11 => Key::F11,
        K::F12 => Key::F12,
        K::F13 => Key::F13,
        K::F14 => Key::F14,
        K::F15 => Key::F15,
        K::F16 => Key::F16,
        K::F17 => Key::F17,
        K::F18 => Key::F18,
        K::F19 => Key::F19,
        K::F20 => Key::F20,
        K::F21 => Key::F21,
        K::F22 => Key::F22,
        K::F23 => Key::F23,
        K::F24 => Key::F24,
        // Possibly next code.
        // K::F25 => Key::Unknown,
        K::Numpad0 => Key::NumPad0,
        K::Numpad1 => Key::NumPad1,
        K::Numpad2 => Key::NumPad2,
        K::Numpad3 => Key::NumPad3,
        K::Numpad4 => Key::NumPad4,
        K::Numpad5 => Key::NumPad5,
        K::Numpad6 => Key::NumPad6,
        K::Numpad7 => Key::NumPad7,
        K::Numpad8 => Key::NumPad8,
        K::Numpad9 => Key::NumPad9,
        K::NumpadComma => Key::NumPadDecimal,
        K::NumpadEnter => Key::NumPadEnter,
        K::NumpadEquals => Key::NumPadEquals,
        K::LShift => Key::LShift,
        K::LControl => Key::LCtrl,
        K::LAlt => Key::LAlt,
        K::RShift => Key::RShift,
        K::RControl => Key::RCtrl,
        K::RAlt => Key::RAlt,
        // Map to backslash?
        // K::GraveAccent => Key::Unknown,
        K::Home => Key::Home,
        K::Insert => Key::Insert,
        K::Left => Key::Left,
        K::LBracket => Key::LeftBracket,
        // K::Menu => Key::Menu,
        K::Minus => Key::Minus,
        K::Numlock => Key::NumLockClear,
        K::PageDown => Key::PageDown,
        K::PageUp => Key::PageUp,
        K::Pause => Key::Pause,
        K::Period => Key::Period,
        K::Snapshot => Key::PrintScreen,
        K::Right => Key::Right,
        K::RBracket => Key::RightBracket,
        K::Scroll => Key::ScrollLock,
        K::Semicolon => Key::Semicolon,
        K::Slash => Key::Slash,
        K::Space => Key::Space,
        K::Tab => Key::Tab,
        K::Up => Key::Up,
        // K::World1 => Key::Unknown,
        // K::World2 => Key::Unknown,
        //_ => Key::Unknown,
        VirtualKeyCode::Compose => Key::Unknown,
        VirtualKeyCode::Caret => Key::Caret,
        VirtualKeyCode::NumpadAdd => Key::NumPadPlus,
        VirtualKeyCode::NumpadDivide => Key::NumPadDivide,
        VirtualKeyCode::NumpadDecimal => Key::NumPadDecimal,
        VirtualKeyCode::NumpadMultiply => Key::NumPadMultiply,
        VirtualKeyCode::NumpadSubtract => Key::NumPadMinus,
        VirtualKeyCode::AbntC1 => Key::Unknown,
        VirtualKeyCode::AbntC2 => Key::Unknown,
        VirtualKeyCode::Apps => Key::Unknown,
        VirtualKeyCode::Asterisk => Key::Asterisk,
        VirtualKeyCode::At => Key::At,
        VirtualKeyCode::Ax => Key::Unknown,
        VirtualKeyCode::Calculator => Key::Calculator,
        VirtualKeyCode::Capital => Key::Unknown,
        VirtualKeyCode::Colon => Key::Colon,
        VirtualKeyCode::Convert => Key::Unknown,
        VirtualKeyCode::Grave => Key::Unknown,
        VirtualKeyCode::Kana => Key::Unknown,
        VirtualKeyCode::Kanji => Key::Unknown,
        VirtualKeyCode::LWin => Key::Unknown,
        VirtualKeyCode::Mail => Key::Mail,
        VirtualKeyCode::MediaSelect => Key::MediaSelect,
        VirtualKeyCode::MediaStop => Key::Stop,
        VirtualKeyCode::Mute => Key::Mute,
        VirtualKeyCode::MyComputer => Key::Computer,
        VirtualKeyCode::NavigateForward => Key::AcForward,
        VirtualKeyCode::NavigateBackward => Key::AcBack,
        VirtualKeyCode::NextTrack => Key::AudioNext,
        VirtualKeyCode::NoConvert => Key::Unknown,
        VirtualKeyCode::OEM102 => Key::Unknown,
        VirtualKeyCode::PlayPause => Key::Unknown,
        VirtualKeyCode::Plus => Key::Plus,
        VirtualKeyCode::Power => Key::Power,
        VirtualKeyCode::PrevTrack => Key::AudioPrev,
        VirtualKeyCode::RWin => Key::Unknown,
        VirtualKeyCode::Sleep => Key::Sleep,
        VirtualKeyCode::Stop => Key::Stop,
        VirtualKeyCode::Sysrq => Key::Sysreq,
        VirtualKeyCode::Underline => Key::Unknown,
        VirtualKeyCode::Unlabeled => Key::Unknown,
        VirtualKeyCode::VolumeDown => Key::VolumeDown,
        VirtualKeyCode::VolumeUp => Key::VolumeUp,
        VirtualKeyCode::Wake => Key::Unknown,
        VirtualKeyCode::WebBack => Key::Unknown,
        VirtualKeyCode::WebFavorites => Key::Unknown,
        VirtualKeyCode::WebForward => Key::Unknown,
        VirtualKeyCode::WebHome => Key::Unknown,
        VirtualKeyCode::WebRefresh => Key::Unknown,
        VirtualKeyCode::WebSearch => Key::Unknown,
        VirtualKeyCode::WebStop => Key::Unknown,
        VirtualKeyCode::Yen => Key::Unknown,
        VirtualKeyCode::Copy => Key::Copy,
        VirtualKeyCode::Paste => Key::Paste,
        VirtualKeyCode::Cut => Key::Cut,
    }
}

/// Maps Glutin's mouse button to Piston's mouse button.
pub fn map_mouse(mouse_button: glutin::event::MouseButton) -> MouseButton {
    use glutin::event::MouseButton as M;

    match mouse_button {
        M::Left => MouseButton::Left,
        M::Right => MouseButton::Right,
        M::Middle => MouseButton::Middle,
        M::Other(0) => MouseButton::X1,
        M::Other(1) => MouseButton::X2,
        M::Other(2) => MouseButton::Button6,
        M::Other(3) => MouseButton::Button7,
        M::Other(4) => MouseButton::Button8,
        _ => MouseButton::Unknown,
    }
}
