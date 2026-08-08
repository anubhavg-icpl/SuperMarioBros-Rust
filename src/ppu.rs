//! Emulates the NES Picture Processing Unit.
//!
//! Faithful 1:1 port of source/Emulation/PPU.cpp. Unlike the C++ version,
//! the PPU owns its CHR data and RGB palette instead of referencing the engine.

const NAMETABLE_MIRROR_LOOKUP: [[u8; 4]; 2] = [
    [0, 0, 1, 1], // Vertical
    [0, 1, 0, 1], // Horizontal
];

/// Default hardcoded palette.
const DEFAULT_PALETTE_RGB: [u32; 64] = [
    0x7c7c7c, 0x0000fc, 0x0000bc, 0x4428bc, 0x940084, 0xa80020, 0xa81000, 0x881400,
    0x503000, 0x007800, 0x006800, 0x005800, 0x004058, 0x000000, 0x000000, 0x000000,
    0xbcbcbc, 0x0078f8, 0x0058f8, 0x6844fc, 0xd800cc, 0xe40058, 0xf83800, 0xe45c10,
    0xac7c00, 0x00b800, 0x00a800, 0x00a844, 0x008888, 0x000000, 0x000000, 0x000000,
    0xf8f8f8, 0x3cbcfc, 0x6888fc, 0x9878f8, 0xf878f8, 0xf85898, 0xf87858, 0xfca044,
    0xf8b800, 0xb8f818, 0x58d854, 0x58f898, 0x00e8d8, 0x787878, 0x000000, 0x000000,
    0xfcfcfc, 0xa4e4fc, 0xb8b8f8, 0xd8b8f8, 0xf8b8f8, 0xf8a4c0, 0xf0d0b0, 0xfce0a8,
    0xf8d878, 0xd8f878, 0xb8f8b8, 0xb8f8d8, 0x00fcfc, 0xf8d8f8, 0x000000, 0x000000,
];

pub struct Ppu {
    ppu_ctrl: u8,   // $2000
    ppu_mask: u8,   // $2001
    #[allow(dead_code)]
    ppu_status: u8, // $2002
    oam_address: u8, // $2003
    ppu_scroll_x: u8, // $2005
    ppu_scroll_y: u8, // $2005

    palette: [u8; 32],      // Palette data.
    nametable: [u8; 2048],  // Background table.
    oam: [u8; 256],         // Sprite memory.

    // PPU Address control
    current_address: u16, // Address that will be accessed on the next PPU read/write.
    write_toggle: bool,   // Toggles low/high byte on the next write to PPUADDR.
    vram_buffer: u8,      // Delays VRAM reads by 1 byte.

    chr: Vec<u8>,
    palette_rgb: [u32; 64],
    status_cycle: u32, // Replaces the C++ function-local `static int cycle` in readRegister.
}

impl Ppu {
    pub fn new(chr: Vec<u8>) -> Ppu {
        Ppu {
            ppu_ctrl: 0,
            ppu_mask: 0,
            ppu_status: 0,
            oam_address: 0,
            ppu_scroll_x: 0,
            ppu_scroll_y: 0,
            palette: [0; 32],
            nametable: [0; 2048],
            oam: [0; 256],
            current_address: 0,
            write_toggle: false,
            vram_buffer: 0,
            chr,
            palette_rgb: DEFAULT_PALETTE_RGB,
            status_cycle: 0,
        }
    }

    pub fn set_palette(&mut self, palette: [u32; 64]) {
        self.palette_rgb = palette;
    }

    fn get_attribute_table_value(&self, nametable_address: u16) -> u8 {
        let nametable_address = self.get_nametable_index(nametable_address);

        // Determine the 32x32 attribute table address
        let row = (((nametable_address as i32) & 0x3e0) >> 5) / 4;
        let col = ((nametable_address as i32) & 0x1f) / 4;

        // Determine the 16x16 metatile for the 8x8 tile addressed
        let shift = (if nametable_address & (1 << 6) != 0 { 4 } else { 0 })
            + (if nametable_address & (1 << 1) != 0 { 2 } else { 0 });

        // Determine the offset into the attribute table
        let offset = ((nametable_address as i32) & 0xc00) + 0x400 - 64 + (row * 8 + col);

        // Determine the attribute table value
        (self.nametable[offset as usize] & (0x3 << shift)) >> shift
    }

    fn get_nametable_index(&self, address: u16) -> u16 {
        let address = (address.wrapping_sub(0x2000)) % 0x1000;
        let table = (address / 0x400) as usize;
        let offset = (address % 0x400) as i32;
        let mode = 1; // Mirroring mode for Super Mario Bros.
        (((NAMETABLE_MIRROR_LOOKUP[mode][table] as i32) * 0x400 + offset) % 2048) as u16
    }

    fn read_byte(&self, address: u16) -> u8 {
        // Mirror all addresses above $3fff
        let address = address & 0x3fff;

        if address < 0x2000 {
            // CHR
            self.chr.get(address as usize).copied().unwrap_or(0)
        } else if address < 0x3f00 {
            // Nametable
            self.nametable[self.get_nametable_index(address) as usize]
        } else {
            0
        }
    }

    fn read_chr(&self, index: i32) -> u8 {
        if index < 0x2000 {
            self.chr.get(index as usize).copied().unwrap_or(0)
        } else {
            0
        }
    }

    fn read_data_register(&mut self) -> u8 {
        let value = self.vram_buffer;
        self.vram_buffer = self.read_byte(self.current_address);

        if self.ppu_ctrl & (1 << 2) == 0 {
            self.current_address = self.current_address.wrapping_add(1);
        } else {
            self.current_address = self.current_address.wrapping_add(32);
        }

        value
    }

    pub fn read_register(&mut self, address: u16) -> u8 {
        match address {
            // PPUSTATUS
            0x2002 => {
                self.write_toggle = false;
                let value = if self.status_cycle % 2 == 0 { 0xc0 } else { 0 };
                self.status_cycle = self.status_cycle.wrapping_add(1);
                value
            }
            // OAMDATA
            0x2004 => self.oam[self.oam_address as usize],
            // PPUDATA
            0x2007 => self.read_data_register(),
            _ => 0,
        }
    }

    fn render_tile(&self, buffer: &mut [u32], index: i32, x_offset: i32, y_offset: i32) {
        // Lookup the pattern table entry
        let tile = (self.read_byte(index as u16) as u16)
            + (if self.ppu_ctrl & (1 << 4) != 0 { 256 } else { 0 });
        let attribute = self.get_attribute_table_value(index as u16);

        // Read the pixels of the tile
        for row in 0..8i32 {
            let plane1 = self.read_chr((tile as i32) * 16 + row);
            let plane2 = self.read_chr((tile as i32) * 16 + row + 8);

            for column in 0..8i32 {
                let palette_index = (if plane1 & (1 << column) != 0 { 1u8 } else { 0 })
                    + (if plane2 & (1 << column) != 0 { 2 } else { 0 });
                let color_index = self.palette[(attribute * 4 + palette_index) as usize];
                if palette_index == 0 {
                    // skip transparent pixels
                    continue;
                }
                let pixel = 0xff000000u32 | self.palette_rgb[color_index as usize];

                let x = x_offset + (7 - column);
                let y = y_offset + row;
                if x < 0 || x >= 256 || y < 0 || y >= 240 {
                    continue;
                }
                buffer[(y * 256 + x) as usize] = pixel;
            }
        }
    }

    /// Render to a frame buffer (256*240 u32 values, indexed `buffer[y*256+x]`).
    pub fn render(&self, buffer: &mut [u32]) {
        // Clear the buffer with the background color
        for index in 0..256 * 240 {
            buffer[index] = self.palette_rgb[self.palette[0] as usize];
        }

        // Draw sprites behind the background
        if self.ppu_mask & (1 << 4) != 0 {
            // Are sprites enabled?
            // Sprites with the lowest index in OAM take priority.
            // Therefore, render the array of sprites in reverse order.
            for i in (0..=63i32).rev() {
                // Read OAM for the sprite
                let y = self.oam[(i * 4) as usize];
                let index = self.oam[(i * 4 + 1) as usize];
                let attributes = self.oam[(i * 4 + 2) as usize];
                let x = self.oam[(i * 4 + 3) as usize];

                // Check if the sprite has the correct priority
                if attributes & (1 << 5) == 0 {
                    continue;
                }

                // Check if the sprite is visible
                if y >= 0xef || x >= 0xf9 {
                    continue;
                }

                // Increment y by one since sprite data is delayed by one scanline
                let y = y.wrapping_add(1);

                // Determine the tile to use
                let tile = (index as u16)
                    + (if self.ppu_ctrl & (1 << 3) != 0 { 256 } else { 0 });
                let flip_x = attributes & (1 << 6) != 0;
                let flip_y = attributes & (1 << 7) != 0;

                // Copy pixels to the framebuffer
                for row in 0..8i32 {
                    let plane1 = self.read_chr((tile as i32) * 16 + row);
                    let plane2 = self.read_chr((tile as i32) * 16 + row + 8);

                    for column in 0..8i32 {
                        let palette_index = (if plane1 & (1 << column) != 0 { 1u8 } else { 0 })
                            + (if plane2 & (1 << column) != 0 { 2 } else { 0 });
                        let color_index =
                            self.palette[(0x10 + (attributes & 0x03) * 4 + palette_index) as usize];
                        if palette_index == 0 {
                            // Skip transparent pixels
                            continue;
                        }
                        let pixel = 0xff000000u32 | self.palette_rgb[color_index as usize];

                        let mut x_offset = 7 - column;
                        if flip_x {
                            x_offset = column;
                        }
                        let mut y_offset = row;
                        if flip_y {
                            y_offset = 7 - row;
                        }

                        let x_pixel = (x as i32) + x_offset;
                        let y_pixel = (y as i32) + y_offset;
                        if x_pixel < 0 || x_pixel >= 256 || y_pixel < 0 || y_pixel >= 240 {
                            continue;
                        }

                        buffer[(y_pixel * 256 + x_pixel) as usize] = pixel;
                    }
                }
            }
        }

        // Draw the background (nametable)
        if self.ppu_mask & (1 << 3) != 0 {
            // Is the background enabled?
            let scroll_x = (self.ppu_scroll_x as i32)
                + (if self.ppu_ctrl & (1 << 0) != 0 { 256 } else { 0 });
            let x_min = scroll_x / 8;
            let x_max = (scroll_x + 256) / 8;
            for x in 0..32i32 {
                for y in 0..4i32 {
                    // Render the status bar in the same position (it doesn't scroll)
                    self.render_tile(buffer, 0x2000 + 32 * y + x, x * 8, y * 8);
                }
            }
            for x in x_min..=x_max {
                for y in 4..30i32 {
                    // Determine the index of the tile to render
                    let index = if x < 32 {
                        0x2000 + 32 * y + x
                    } else if x < 64 {
                        0x2400 + 32 * y + (x - 32)
                    } else {
                        0x2800 + 32 * y + (x - 64)
                    };

                    // Render the tile
                    self.render_tile(buffer, index, (x * 8) - scroll_x, y * 8);
                }
            }
        }

        // Draw sprites in front of the background
        if self.ppu_mask & (1 << 4) != 0 {
            // Sprites with the lowest index in OAM take priority.
            // Therefore, render the array of sprites in reverse order.
            //
            // We render sprite 0 first as a special case (coin indicator).
            for j in (1..=64i32).rev() {
                // Start at 0, then 63, 62, 61, ..., 1
                let i = j % 64;

                // Read OAM for the sprite
                let y = self.oam[(i * 4) as usize];
                let index = self.oam[(i * 4 + 1) as usize];
                let attributes = self.oam[(i * 4 + 2) as usize];
                let x = self.oam[(i * 4 + 3) as usize];

                // Check if the sprite has the correct priority
                //
                // Special case for sprite 0, tile 0xff in Super Mario Bros.
                // (part of the pixels for the coin indicator)
                if attributes & (1 << 5) != 0 && !(i == 0 && index == 0xff) {
                    continue;
                }

                // Check if the sprite is visible
                if y >= 0xef || x >= 0xf9 {
                    continue;
                }

                // Increment y by one since sprite data is delayed by one scanline
                let y = y.wrapping_add(1);

                // Determine the tile to use
                let tile = (index as u16)
                    + (if self.ppu_ctrl & (1 << 3) != 0 { 256 } else { 0 });
                let flip_x = attributes & (1 << 6) != 0;
                let flip_y = attributes & (1 << 7) != 0;

                // Copy pixels to the framebuffer
                for row in 0..8i32 {
                    let plane1 = self.read_chr((tile as i32) * 16 + row);
                    let plane2 = self.read_chr((tile as i32) * 16 + row + 8);

                    for column in 0..8i32 {
                        let palette_index = (if plane1 & (1 << column) != 0 { 1u8 } else { 0 })
                            + (if plane2 & (1 << column) != 0 { 2 } else { 0 });
                        let color_index =
                            self.palette[(0x10 + (attributes & 0x03) * 4 + palette_index) as usize];
                        if palette_index == 0 {
                            // Skip transparent pixels
                            continue;
                        }
                        let pixel = 0xff000000u32 | self.palette_rgb[color_index as usize];

                        let mut x_offset = 7 - column;
                        if flip_x {
                            x_offset = column;
                        }
                        let mut y_offset = row;
                        if flip_y {
                            y_offset = 7 - row;
                        }

                        let x_pixel = (x as i32) + x_offset;
                        let y_pixel = (y as i32) + y_offset;
                        if x_pixel < 0 || x_pixel >= 256 || y_pixel < 0 || y_pixel >= 240 {
                            continue;
                        }

                        // Special case for sprite 0, tile 0xff in Super Mario Bros.
                        // (part of the pixels for the coin indicator)
                        if i == 0 && index == 0xff && row == 5 && column > 3 && column < 6 {
                            continue;
                        }

                        buffer[(y_pixel * 256 + x_pixel) as usize] = pixel;
                    }
                }
            }
        }
    }

    fn write_address_register(&mut self, value: u8) {
        if !self.write_toggle {
            // Upper byte
            self.current_address =
                (self.current_address & 0xff) | (((value as u16) << 8) & 0xff00);
        } else {
            // Lower byte
            self.current_address = (self.current_address & 0xff00) | (value as u16);
        }
        self.write_toggle = !self.write_toggle;
    }

    fn write_byte(&mut self, address: u16, value: u8) {
        // Mirror all addresses above $3fff
        let address = address & 0x3fff;

        if address < 0x2000 {
            // CHR (no-op)
        } else if address < 0x3f00 {
            let index = self.get_nametable_index(address);
            self.nametable[index as usize] = value;
        } else if address < 0x3f20 {
            // Palette data
            self.palette[(address - 0x3f00) as usize] = value;

            // Mirroring
            if address == 0x3f10 || address == 0x3f14 || address == 0x3f18 || address == 0x3f1c {
                self.palette[(address - 0x3f10) as usize] = value;
            }
        }
    }

    fn write_data_register(&mut self, value: u8) {
        self.write_byte(self.current_address, value);
        if self.ppu_ctrl & (1 << 2) == 0 {
            self.current_address = self.current_address.wrapping_add(1);
        } else {
            self.current_address = self.current_address.wrapping_add(32);
        }
    }

    /// OAM DMA: copies 256 bytes into OAM starting at `oam_address`,
    /// wrapping the OAM address for each byte (matches the C++ writeDMA loop).
    pub fn write_dma(&mut self, data: &[u8; 256]) {
        for &byte in data.iter() {
            self.oam[self.oam_address as usize] = byte;
            self.oam_address = self.oam_address.wrapping_add(1);
        }
    }

    pub fn write_register(&mut self, address: u16, value: u8) {
        match address {
            // PPUCTRL
            0x2000 => self.ppu_ctrl = value,
            // PPUMASK
            0x2001 => self.ppu_mask = value,
            // OAMADDR
            0x2003 => self.oam_address = value,
            // OAMDATA
            0x2004 => {
                self.oam[self.oam_address as usize] = value;
                self.oam_address = self.oam_address.wrapping_add(1);
            }
            // PPUSCROLL
            0x2005 => {
                if !self.write_toggle {
                    self.ppu_scroll_x = value;
                } else {
                    self.ppu_scroll_y = value;
                }
                self.write_toggle = !self.write_toggle;
            }
            // PPUADDR
            0x2006 => self.write_address_register(value),
            // PPUDATA
            0x2007 => self.write_data_register(value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ponytail: minimal self-check — fails if register plumbing or render clear breaks.
    #[test]
    fn basic_register_and_render_behavior() {
        let mut ppu = Ppu::new(vec![0u8; 0x2000]);

        // PPUSTATUS alternates 0xc0 / 0
        assert_eq!(ppu.read_register(0x2002), 0xc0);
        assert_eq!(ppu.read_register(0x2002), 0);
        assert_eq!(ppu.read_register(0x2002), 0xc0);

        // PPUADDR high/low, then palette write via PPUDATA with $3f10 mirroring
        ppu.write_register(0x2006, 0x3f);
        ppu.write_register(0x2006, 0x10);
        ppu.write_register(0x2007, 0x21);
        assert_eq!(ppu.palette[0x10], 0x21);
        assert_eq!(ppu.palette[0x00], 0x21); // mirrored to $3f00

        // OAM address wraps
        ppu.write_register(0x2003, 0xff);
        ppu.write_register(0x2004, 0xab);
        assert_eq!(ppu.oam[0xff], 0xab);
        ppu.write_register(0x2004, 0xcd);
        assert_eq!(ppu.oam[0x00], 0xcd);

        // DMA fills OAM starting at current oam_address (now 1)
        ppu.write_dma(&[7u8; 256]);
        assert_eq!(ppu.oam[1], 7);
        assert_eq!(ppu.oam[0], 7); // wrapped around

        // Render clears buffer to background color (palette[0] = 0x21 from above)
        let mut buffer = vec![0u32; 256 * 240];
        ppu.render(&mut buffer);
        assert_eq!(buffer[0], DEFAULT_PALETTE_RGB[0x21]);
    }
}
