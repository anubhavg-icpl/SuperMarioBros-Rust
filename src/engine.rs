//! Core NES CPU/memory emulation for Super Mario Bros.
//!
//! Mirrors the semantics of the original C++ `SMBEngine` + `MemoryAccess`.
//! All arithmetic is done in `i32` to emulate C++ integer promotion; values are
//! truncated to 8 bits on store and status flags are set exactly as the 6502.

use std::cell::{RefCell, RefMut};

use crate::apu::Apu;
use crate::configuration;
use crate::controller::Controller;
use crate::ppu::Ppu;

const DATA_STORAGE_OFFSET: i32 = 0x8000;

pub struct Engine {
    // CPU registers
    pub(crate) ra: u8,
    pub(crate) rx: u8,
    pub(crate) ry: u8,
    pub(crate) rs: u8,
    // Status flags
    pub(crate) c: bool,
    pub(crate) z: bool,
    pub(crate) n: bool,

    data_storage: Vec<u8>, // 0x8000 bytes of constant data
    ram: Vec<u8>,          // 0x800 bytes of RAM

    return_stack: [i32; 128],
    return_top: i32,

    // Wrapped in RefCell so `read`/`read_word` can take `&self` (reads of PPU
    // status / controller shift registers mutate those subsystems).
    ppu: RefCell<Ppu>,
    apu: Apu,
    controller1: RefCell<Controller>,
    controller2: RefCell<Controller>,
}

impl Engine {
    pub fn new(rom: &[u8]) -> Engine {
        // CHR location in ROM: 16-byte header + 2 PRG pages (16k each)
        let chr_start = 16 + 16384 * 2;
        let chr = rom[chr_start..].to_vec();

        Engine {
            ra: 0,
            rx: 0,
            ry: 0,
            rs: 0,
            c: false,
            z: false,
            n: false,
            data_storage: vec![0u8; 0x8000],
            ram: vec![0u8; 0x800],
            return_stack: [0; 128],
            return_top: 0,
            ppu: RefCell::new(Ppu::new(chr)),
            apu: Apu::new(),
            controller1: RefCell::new(Controller::new()),
            controller2: RefCell::new(Controller::new()),
        }
    }

    //-----------------------------------------------------------------
    // Public interface
    //-----------------------------------------------------------------

    pub fn reset(&mut self) {
        self.code(0);
    }

    pub fn update(&mut self) {
        self.code(1);
        if configuration::audio_enabled() {
            self.apu.step_frame();
        }
    }

    pub fn render(&self, buffer: &mut [u32]) {
        self.ppu.borrow().render(buffer);
    }

    pub fn audio_callback(&mut self, stream: &mut [i8]) {
        self.apu.output(stream);
    }

    pub fn controller1(&self) -> RefMut<'_, Controller> {
        self.controller1.borrow_mut()
    }

    #[allow(dead_code)]
    pub fn controller2(&self) -> RefMut<'_, Controller> {
        self.controller2.borrow_mut()
    }

    pub fn set_palette(&mut self, palette: [u32; 64]) {
        self.ppu.borrow_mut().set_palette(palette);
    }

    //-----------------------------------------------------------------
    // Register accessors (as i32, for use in generated expressions)
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn a(&self) -> i32 {
        self.ra as i32
    }
    #[inline]
    pub(crate) fn x(&self) -> i32 {
        self.rx as i32
    }
    #[inline]
    pub(crate) fn y(&self) -> i32 {
        self.ry as i32
    }
    #[inline]
    #[allow(dead_code)] // TSX (x = s) is never emitted by the SMB disassembly
    pub(crate) fn s(&self) -> i32 {
        self.rs as i32
    }

    //-----------------------------------------------------------------
    // Flag helpers
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn set_zn(&mut self, value: i32) {
        self.z = (value & 0xff) == 0;
        self.n = (value & 0x80) != 0;
    }

    #[inline]
    pub(crate) fn clc(&mut self) {
        self.c = false;
    }
    #[inline]
    pub(crate) fn sec(&mut self) {
        self.c = true;
    }

    //-----------------------------------------------------------------
    // Register loads (LDA/LDX/LDY/transfers) — set Z/N
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn set_a(&mut self, value: i32) {
        self.ra = (value & 0xff) as u8;
        self.set_zn(value);
    }
    #[inline]
    pub(crate) fn set_x(&mut self, value: i32) {
        self.rx = (value & 0xff) as u8;
        self.set_zn(value);
    }
    #[inline]
    pub(crate) fn set_y(&mut self, value: i32) {
        self.ry = (value & 0xff) as u8;
        self.set_zn(value);
    }
    #[inline]
    pub(crate) fn set_s(&mut self, value: i32) {
        self.rs = (value & 0xff) as u8;
        self.set_zn(value);
    }

    //-----------------------------------------------------------------
    // Logical ops on A
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn and(&mut self, value: i32) {
        self.ra &= (value & 0xff) as u8;
        let a = self.ra as i32;
        self.set_zn(a);
    }
    #[inline]
    pub(crate) fn eor(&mut self, value: i32) {
        self.ra ^= (value & 0xff) as u8;
        let a = self.ra as i32;
        self.set_zn(a);
    }
    #[inline]
    pub(crate) fn ora(&mut self, value: i32) {
        self.ra |= (value & 0xff) as u8;
        let a = self.ra as i32;
        self.set_zn(a);
    }

    //-----------------------------------------------------------------
    // Arithmetic on A (ADC/SBC) — carry in/out
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn adc(&mut self, value: i32) {
        let temp = self.ra as i32 + (value & 0xff) + if self.c { 1 } else { 0 };
        self.ra = (temp & 0xff) as u8;
        let a = self.ra as i32;
        self.set_zn(a);
        self.c = temp > 0xff;
    }
    #[inline]
    pub(crate) fn sbc(&mut self, value: i32) {
        let temp = self.ra as i32 - (value & 0xff) - if self.c { 0 } else { 1 };
        self.ra = (temp & 0xff) as u8;
        let a = self.ra as i32;
        self.set_zn(a);
        self.c = (temp & 0xffff) < 0x100;
    }

    //-----------------------------------------------------------------
    // Compare / BIT
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn compare(&mut self, value1: i32, value2: i32) {
        let v1 = value1 & 0xff;
        let v2 = value2 & 0xff;
        let result = (v1 - v2) & 0xff;
        self.c = v1 >= v2;
        self.set_zn(result);
    }
    #[inline]
    pub(crate) fn bit(&mut self, value: i32) {
        self.n = (value & 0x80) != 0;
        self.z = (self.ra as i32 & value) & 0xff == 0;
    }

    //-----------------------------------------------------------------
    // Register increment / decrement
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn inx(&mut self) {
        self.rx = self.rx.wrapping_add(1);
        let v = self.rx as i32;
        self.set_zn(v);
    }
    #[inline]
    pub(crate) fn iny(&mut self) {
        self.ry = self.ry.wrapping_add(1);
        let v = self.ry as i32;
        self.set_zn(v);
    }
    #[inline]
    pub(crate) fn dex(&mut self) {
        self.rx = self.rx.wrapping_sub(1);
        let v = self.rx as i32;
        self.set_zn(v);
    }
    #[inline]
    pub(crate) fn dey(&mut self) {
        self.ry = self.ry.wrapping_sub(1);
        let v = self.ry as i32;
        self.set_zn(v);
    }

    //-----------------------------------------------------------------
    // Memory increment / decrement
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn inc(&mut self, address: i32) {
        let v = (self.read(address) + 1) & 0xff;
        self.write(address, v);
        self.set_zn(v);
    }
    #[inline]
    pub(crate) fn dec(&mut self, address: i32) {
        let v = (self.read(address) - 1) & 0xff;
        self.write(address, v);
        self.set_zn(v);
    }

    //-----------------------------------------------------------------
    // Shifts / rotates on A
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn asl_a(&mut self) {
        self.c = (self.ra & 0x80) != 0;
        self.ra = (self.ra << 1) & 0xfe;
        let v = self.ra as i32;
        self.set_zn(v);
    }
    #[inline]
    pub(crate) fn lsr_a(&mut self) {
        self.c = (self.ra & 0x01) != 0;
        self.ra = (self.ra >> 1) & 0x7f;
        let v = self.ra as i32;
        self.set_zn(v);
    }
    #[inline]
    pub(crate) fn rol_a(&mut self) {
        let bit7 = (self.ra & 0x80) != 0;
        self.ra <<= 1;
        if self.c {
            self.ra |= 0x01;
        }
        self.c = bit7;
        let v = self.ra as i32;
        self.set_zn(v);
    }
    #[inline]
    pub(crate) fn ror_a(&mut self) {
        let bit0 = (self.ra & 0x01) != 0;
        self.ra >>= 1;
        if self.c {
            self.ra |= 0x80;
        }
        self.c = bit0;
        let v = self.ra as i32;
        self.set_zn(v);
    }

    //-----------------------------------------------------------------
    // Shifts / rotates on memory
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn asl_mem(&mut self, address: i32) {
        let mut v = self.read(address);
        self.c = (v & 0x80) != 0;
        v = (v << 1) & 0xfe;
        self.write(address, v);
        self.set_zn(v);
    }
    #[inline]
    pub(crate) fn lsr_mem(&mut self, address: i32) {
        let mut v = self.read(address);
        self.c = (v & 0x01) != 0;
        v = (v >> 1) & 0x7f;
        self.write(address, v);
        self.set_zn(v);
    }
    #[inline]
    pub(crate) fn rol_mem(&mut self, address: i32) {
        let mut v = self.read(address);
        let bit7 = (v & 0x80) != 0;
        v = (v << 1) & 0xff;
        if self.c {
            v |= 0x01;
        }
        self.c = bit7;
        self.write(address, v);
        self.set_zn(v);
    }
    #[inline]
    pub(crate) fn ror_mem(&mut self, address: i32) {
        let mut v = self.read(address);
        let bit0 = (v & 0x01) != 0;
        v >>= 1;
        if self.c {
            v |= 0x80;
        }
        self.c = bit0;
        self.write(address, v);
        self.set_zn(v);
    }

    //-----------------------------------------------------------------
    // Stack (PHA/PLA)
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn pha(&mut self) {
        let addr = 0x100 | self.rs as i32;
        let a = self.ra as i32;
        self.write(addr, a);
        self.rs = self.rs.wrapping_sub(1);
    }
    #[inline]
    pub(crate) fn pla(&mut self) {
        self.rs = self.rs.wrapping_add(1);
        let addr = 0x100 | self.rs as i32;
        let v = self.read(addr);
        self.set_a(v);
    }

    //-----------------------------------------------------------------
    // Return-index stack (emulates JSR/RTS call stack)
    //-----------------------------------------------------------------

    #[inline]
    pub(crate) fn push_return_index(&mut self, index: i32) {
        self.return_top += 1;
        self.return_stack[self.return_top as usize] = index;
    }
    #[inline]
    pub(crate) fn pop_return_index(&mut self) -> i32 {
        let v = self.return_stack[self.return_top as usize];
        self.return_top -= 1;
        v
    }

    //-----------------------------------------------------------------
    // Memory access
    //-----------------------------------------------------------------

    pub(crate) fn read(&self, address: i32) -> i32 {
        let address = (address & 0xffff) as u16;
        if address >= 0x8000 {
            self.data_storage[(address as i32 - DATA_STORAGE_OFFSET) as usize] as i32
        } else if address < 0x2000 {
            self.ram[(address & 0x7ff) as usize] as i32
        } else if address < 0x4000 {
            self.ppu.borrow_mut().read_register(0x2000 + (address & 0x7)) as i32
        } else if address < 0x4020 {
            match address {
                0x4016 => self.controller1.borrow_mut().read_byte() as i32,
                0x4017 => self.controller2.borrow_mut().read_byte() as i32,
                _ => 0,
            }
        } else {
            0
        }
    }

    pub(crate) fn read_word(&self, address: i32) -> i32 {
        self.read(address) + (self.read(address + 1) << 8)
    }

    pub(crate) fn write(&mut self, address: i32, value: i32) {
        let address = (address & 0xffff) as u16;
        let value = (value & 0xff) as u8;
        if address < 0x2000 {
            self.ram[(address & 0x7ff) as usize] = value;
        } else if address < 0x4000 {
            self.ppu.borrow_mut().write_register(0x2000 + (address & 0x7), value);
        } else if address < 0x4020 {
            match address {
                0x4014 => {
                    // OAM DMA: copy 256 bytes from CPU address space to PPU OAM
                    let base = (value as i32) << 8;
                    let mut buf = [0u8; 256];
                    for (i, b) in buf.iter_mut().enumerate() {
                        *b = self.read(base + i as i32) as u8;
                    }
                    self.ppu.borrow_mut().write_dma(&buf);
                }
                0x4016 => {
                    self.controller1.borrow_mut().write_byte(value);
                    self.controller2.borrow_mut().write_byte(value);
                }
                _ => {
                    self.apu.write_register(address, value);
                }
            }
        }
        // address >= 0x4020 (including constant-data region): no-op at runtime
    }

    #[cfg(test)]
    pub fn debug_ram(&self, address: usize) -> u8 {
        self.ram[address & 0x7ff]
    }

    /// Map constant data into the address space. Address must be >= 0x8000.
    pub(crate) fn write_range(&mut self, address: i32, data: &[i32]) {
        let start = (address - DATA_STORAGE_OFFSET) as usize;
        for (i, &b) in data.iter().enumerate() {
            self.data_storage[start + i] = (b & 0xff) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercise the full generated 6502 state machine: power-on reset plus
    /// many NMI frames, using a zero-filled ROM (the game's constant data
    /// comes from the disassembly via load_constant_data, not the ROM image).
    #[test]
    fn runs_many_frames_without_panic() {
        let rom = vec![0u8; 16 + 16384 * 2 + 8192];
        let mut engine = Engine::new(&rom);
        engine.reset();

        let mut buffer = vec![0u32; 256 * 240];
        for _ in 0..300 {
            engine.update();
            engine.render(&mut buffer);
        }

        // OperMode ($0770) should be a valid mode (0..=3) after running.
        assert!(engine.debug_ram(0x0770) <= 3, "OperMode out of range");
    }
}
