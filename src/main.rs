#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

mod apu;
mod configuration;
mod constants;
mod controller;
mod engine;
mod generated;
mod ppu;
mod state;
mod video;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sdl2::audio::{AudioCallback, AudioSpecDesired};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::Scancode;
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::BlendMode;

use constants::{APP_TITLE, CONFIG_FILE_NAME, RENDER_HEIGHT, RENDER_WIDTH};
use controller::Button;
use engine::Engine;

const RENDER_PIXELS: usize = (RENDER_WIDTH * RENDER_HEIGHT) as usize;

/// SDL audio callback: pulls signed 8-bit samples from the engine's APU.
struct EngineAudio {
    engine: Arc<Mutex<Engine>>,
}

impl AudioCallback for EngineAudio {
    type Channel = i8;

    fn callback(&mut self, out: &mut [i8]) {
        for s in out.iter_mut() {
            *s = 0;
        }
        if let Ok(mut engine) = self.engine.lock() {
            engine.audio_callback(out);
        }
    }
}

fn load_rom_image() -> Option<Vec<u8>> {
    let path = configuration::rom_file();
    match std::fs::read(path) {
        Ok(data) => Some(data),
        Err(_) => {
            println!("Failed to open the file \"{}\". Exiting.", path);
            None
        }
    }
}

fn main() {
    configuration::initialize(CONFIG_FILE_NAME);

    let rom = match load_rom_image() {
        Some(r) => r,
        None => {
            println!("Failed to initialize. The program will now exit.");
            std::process::exit(-1);
        }
    };

    let sdl_context = sdl2::init().expect("SDL_Init failed");
    let video_subsystem = sdl_context.video().expect("SDL video init failed");
    let audio_subsystem = sdl_context.audio().expect("SDL audio init failed");

    let scale = configuration::render_scale();
    let window = video_subsystem
        .window(APP_TITLE, RENDER_WIDTH * scale, RENDER_HEIGHT * scale)
        .position_centered()
        .build()
        .expect("SDL_CreateWindow failed");

    let mut canvas_builder = window.into_canvas().accelerated();
    if configuration::vsync_enabled() {
        canvas_builder = canvas_builder.present_vsync();
    }
    let mut canvas = canvas_builder.build().expect("SDL_CreateRenderer failed");
    canvas
        .set_logical_size(RENDER_WIDTH, RENDER_HEIGHT)
        .expect("SDL_RenderSetLogicalSize failed");

    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(PixelFormatEnum::ARGB8888, RENDER_WIDTH, RENDER_HEIGHT)
        .expect("SDL_CreateTexture failed");

    // Optional scanline overlay texture (rendered at 3x).
    let scanlines_enabled = configuration::scanlines_enabled();
    let mut scanline_texture = if scanlines_enabled {
        Some(generate_scanline_texture(&texture_creator))
    } else {
        None
    };

    // Engine, shared with the audio callback thread.
    let engine = Arc::new(Mutex::new(Engine::new(&rom)));

    // Apply a custom palette if configured.
    if !configuration::palette_file().is_empty() {
        if let Some(palette) = video::load_palette(configuration::palette_file()) {
            engine.lock().unwrap().set_palette(palette);
        }
    }

    // Start audio.
    let _audio_device = if configuration::audio_enabled() {
        let desired = AudioSpecDesired {
            freq: Some(configuration::audio_frequency()),
            channels: Some(1),
            samples: Some(2048),
        };
        let device = audio_subsystem
            .open_playback(None, &desired, |_spec| EngineAudio {
                engine: Arc::clone(&engine),
            })
            .expect("SDL_OpenAudio failed");
        device.resume();
        Some(device)
    } else {
        None
    };

    engine.lock().unwrap().reset();

    let mut render_buffer = vec![0u32; RENDER_PIXELS];
    let mut event_pump = sdl_context.event_pump().expect("event pump failed");

    let mut running = true;
    let mut fullscreen = false;
    let frame_duration = Duration::from_secs_f64(1.0 / configuration::frame_rate().max(1) as f64);
    let mut next_frame_time = Instant::now();

    let mut paused = false;
    let mut slot: u32 = 1;
    let mut status: Option<(String, Instant)> = None;
    let mut shown_title = String::new();

    while running {
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => running = false,
                Event::Window {
                    win_event: WindowEvent::Close,
                    ..
                } => running = false,
                Event::Window {
                    win_event: WindowEvent::FocusLost,
                    ..
                } => paused = true,
                Event::KeyDown {
                    scancode: Some(code),
                    repeat: false,
                    ..
                } => {
                    let message = match code {
                        Scancode::P => {
                            paused = !paused;
                            None
                        }
                        Scancode::F5 => Some(save_to_slot(&engine.lock().unwrap(), slot)),
                        Scancode::F9 => Some(load_from_slot(&mut engine.lock().unwrap(), slot)),
                        _ => slot_for_key(code).map(|n| {
                            slot = n;
                            format!("Slot {} selected", n)
                        }),
                    };
                    if let Some(message) = message {
                        println!("{}", message);
                        status = Some((message, Instant::now() + Duration::from_secs(2)));
                    }
                }
                _ => {}
            }
        }

        // Window title doubles as the status line (paused, slot, last action).
        if status.as_ref().is_some_and(|(_, until)| Instant::now() >= *until) {
            status = None;
        }
        let mut title = format!("{}  [slot {}]", APP_TITLE, slot);
        if paused {
            title.push_str("  PAUSED");
        }
        if let Some((message, _)) = &status {
            title.push_str("  - ");
            title.push_str(message);
        }
        if title != shown_title {
            let _ = canvas.window_mut().set_title(&title);
            shown_title = title;
        }

        let keys = event_pump.keyboard_state();
        {
            let mut engine = engine.lock().unwrap();
            {
                let mut c1 = engine.controller1();
                c1.set_button_state(Button::A, keys.is_scancode_pressed(Scancode::X));
                c1.set_button_state(Button::B, keys.is_scancode_pressed(Scancode::Z));
                c1.set_button_state(Button::Select, keys.is_scancode_pressed(Scancode::Backspace));
                c1.set_button_state(Button::Start, keys.is_scancode_pressed(Scancode::Return));
                c1.set_button_state(Button::Up, keys.is_scancode_pressed(Scancode::Up));
                c1.set_button_state(Button::Down, keys.is_scancode_pressed(Scancode::Down));
                c1.set_button_state(Button::Left, keys.is_scancode_pressed(Scancode::Left));
                c1.set_button_state(Button::Right, keys.is_scancode_pressed(Scancode::Right));
            }

            if keys.is_scancode_pressed(Scancode::R) {
                engine.reset();
            }
            if keys.is_scancode_pressed(Scancode::Escape) {
                running = false;
            }
            if keys.is_scancode_pressed(Scancode::F) && !fullscreen {
                let _ = canvas
                    .window_mut()
                    .set_fullscreen(sdl2::video::FullscreenType::Desktop);
                fullscreen = true;
            }

            // While paused the game does not advance; the last frame stays on
            // screen and the audio queue drains to silence.
            if !paused {
                engine.update();
            }
            engine.render(&mut render_buffer);
        }

        // Upload the frame buffer to the texture.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                render_buffer.as_ptr() as *const u8,
                render_buffer.len() * 4,
            )
        };
        texture
            .update(None, bytes, (RENDER_WIDTH * 4) as usize)
            .expect("SDL_UpdateTexture failed");

        canvas.clear();
        canvas.set_logical_size(RENDER_WIDTH, RENDER_HEIGHT).ok();
        canvas.copy(&texture, None, None).ok();

        if let Some(scanline) = scanline_texture.as_mut() {
            canvas
                .set_logical_size(RENDER_WIDTH * 3, RENDER_HEIGHT * 3)
                .ok();
            canvas.copy(scanline, None, None).ok();
        }

        canvas.present();

        // Frame pacing: keep as close to the desired FPS as possible.
        // The game logic and the APU advance exactly once per loop iteration,
        // so the loop must never run faster than `frame_rate`, whatever the
        // display's refresh rate is.
        next_frame_time += frame_duration;
        let now = Instant::now();
        if next_frame_time > now {
            std::thread::sleep(next_frame_time - now);
        } else if now - next_frame_time > Duration::from_millis(100) {
            // Fell far behind (window drag, suspend, ...): re-anchor instead
            // of fast-forwarding to catch up.
            next_frame_time = now;
        }
    }
}

/// Number keys 1-9 (top row or keypad) select a save slot.
fn slot_for_key(code: Scancode) -> Option<u32> {
    use Scancode::*;
    let n = match code {
        Num1 | Kp1 => 1,
        Num2 | Kp2 => 2,
        Num3 | Kp3 => 3,
        Num4 | Kp4 => 4,
        Num5 | Kp5 => 5,
        Num6 | Kp6 => 6,
        Num7 | Kp7 => 7,
        Num8 | Kp8 => 8,
        Num9 | Kp9 => 9,
        _ => return None,
    };
    Some(n)
}

/// Save states live next to the ROM file: `smbc-slot1.state` ... `smbc-slot9.state`.
fn slot_path(slot: u32) -> PathBuf {
    let rom = Path::new(configuration::rom_file());
    let dir = rom.parent().unwrap_or(Path::new(""));
    dir.join(format!("smbc-slot{}.state", slot))
}

fn save_to_slot(engine: &Engine, slot: u32) -> String {
    let path = slot_path(slot);
    let data = engine.save_state();
    // Write to a temporary file and rename, so a crash mid-write can never
    // destroy the previous save in this slot.
    let tmp = path.with_extension("state.tmp");
    let result = std::fs::write(&tmp, &data).and_then(|_| std::fs::rename(&tmp, &path));
    match result {
        Ok(()) => format!("Saved to slot {}", slot),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            format!("Could not save slot {} ({}): {}", slot, path.display(), e)
        }
    }
}

fn load_from_slot(engine: &mut Engine, slot: u32) -> String {
    let path = slot_path(slot);
    let data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return format!("Slot {} is empty", slot);
        }
        Err(e) => return format!("Could not read slot {}: {}", slot, e),
    };
    match engine.load_state(&data) {
        Ok(()) => format!("Loaded slot {}", slot),
        Err(e) => format!("Could not load slot {}: {}", slot, e),
    }
}

/// Build the 3x scanline overlay texture (multiplicative blend).
fn generate_scanline_texture(
    texture_creator: &sdl2::render::TextureCreator<sdl2::video::WindowContext>,
) -> sdl2::render::Texture<'_> {
    let w = RENDER_WIDTH * 3;
    let h = RENDER_HEIGHT * 3;
    let mut buffer = vec![0u32; (w * h) as usize];
    for y in 0..RENDER_HEIGHT {
        for x in 0..RENDER_WIDTH {
            for i in 0..3 {
                for j in 0..3u32 {
                    let color = 0xff000000
                        | match j {
                            0 => 0x00fdd6c7,
                            1 => 0x00bef5e1,
                            _ => 0x00cfe2ff,
                        };
                    let idx = ((y * 3 + i) * w + (x * 3 + j)) as usize;
                    buffer[idx] = color;
                }
            }
        }
    }

    let mut texture = texture_creator
        .create_texture_static(PixelFormatEnum::ARGB8888, w, h)
        .expect("scanline texture create failed");
    texture.set_blend_mode(BlendMode::Mod);
    let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const u8, buffer.len() * 4) };
    texture.update(None, bytes, (w * 4) as usize).ok();
    texture
}
