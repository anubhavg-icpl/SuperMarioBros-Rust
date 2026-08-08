//! Video utilities (ported from Video.cpp).
//!
//! Only the pieces the game actually uses are ported: custom palette loading.
//! (The C++ drawBox/drawText/drawCHRTile helpers were dead code.)

/// Load a 64-entry RGB palette from a `.pal` file (192 or 1536 bytes).
pub fn load_palette(file_name: &str) -> Option<[u32; 64]> {
    let buffer = match std::fs::read(file_name) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("Unable to open palette file \"{}\"", file_name);
            return None;
        }
    };

    if buffer.len() != 192 && buffer.len() != 1536 {
        eprintln!("Unsupported palette file \"{}\"", file_name);
        return None;
    }

    let mut palette = [0u32; 64];
    for (entry, slot) in palette.iter_mut().enumerate() {
        *slot = ((buffer[entry * 3] as u32) << 16)
            | ((buffer[entry * 3 + 1] as u32) << 8)
            | (buffer[entry * 3 + 2] as u32);
    }
    Some(palette)
}
