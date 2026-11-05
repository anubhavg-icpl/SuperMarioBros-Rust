#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

mod apu;
mod configuration;
mod constants;
mod controller;
mod engine;
mod generated;
mod ppu;
mod video;

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

    while running {
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => running = false,
                Event::Window {
                    win_event: WindowEvent::Close,
                    ..
                } => running = false,
                _ => {}
            }
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

            engine.update();
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
