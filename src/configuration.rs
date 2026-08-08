//! Global configuration (ported from Configuration.cpp).
//!
//! Reads an optional INI file; falls back to defaults. Stored once in a
//! process-global `OnceLock` so the audio callback and game loop can read it.

use std::sync::OnceLock;

pub struct Config {
    pub audio_enabled: bool,
    pub audio_frequency: i32,
    pub frame_rate: i32,
    pub palette_file: String,
    pub render_scale: u32,
    pub rom_file: String,
    pub scanlines_enabled: bool,
    pub vsync_enabled: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            audio_enabled: true,
            audio_frequency: 48000,
            frame_rate: 60,
            palette_file: String::new(),
            render_scale: 3,
            rom_file: "Super Mario Bros. (JU) (PRG0) [!].nes".to_string(),
            scanlines_enabled: true,
            vsync_enabled: true,
        }
    }
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Parse the config file (if present) and install the global configuration.
pub fn initialize(file_name: &str) {
    let mut config = Config::default();

    if let Ok(contents) = std::fs::read_to_string(file_name) {
        let mut section = String::new();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = format!("{}.{}", section, key.trim());
            let value = value.trim();
            match key.as_str() {
                "audio.enabled" => config.audio_enabled = parse_bool(value),
                "audio.frequency" => set_int(&mut config.audio_frequency, value),
                "game.frame_rate" => set_int(&mut config.frame_rate, value),
                "video.palette_file" => config.palette_file = value.to_string(),
                "video.scale" => {
                    if let Ok(v) = value.parse::<u32>() {
                        config.render_scale = v;
                    }
                }
                "game.rom_file" => config.rom_file = value.to_string(),
                "video.scanlines" => config.scanlines_enabled = parse_bool(value),
                "video.vsync" => config.vsync_enabled = parse_bool(value),
                _ => {}
            }
            println!("Configuration option \"{}\" set to \"{}\"", key, value);
        }
    }

    let _ = CONFIG.set(config);
}

fn parse_bool(value: &str) -> bool {
    value != "0" && !value.eq_ignore_ascii_case("false")
}

fn set_int(target: &mut i32, value: &str) {
    if let Ok(v) = value.parse::<i32>() {
        *target = v;
    }
}

fn get() -> &'static Config {
    CONFIG.get_or_init(Config::default)
}

pub fn audio_enabled() -> bool {
    get().audio_enabled
}
pub fn audio_frequency() -> i32 {
    get().audio_frequency
}
pub fn frame_rate() -> i32 {
    get().frame_rate
}
pub fn palette_file() -> &'static str {
    &get().palette_file
}
pub fn render_scale() -> u32 {
    get().render_scale
}
pub fn rom_file() -> &'static str {
    &get().rom_file
}
pub fn scanlines_enabled() -> bool {
    get().scanlines_enabled
}
pub fn vsync_enabled() -> bool {
    get().vsync_enabled
}
