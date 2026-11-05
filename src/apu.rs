//! NES APU (audio processing unit) emulator.
//!
//! Faithful port of source/Emulation/APU.cpp / APU.hpp.

use crate::state::{impl_save_state, Reader, SaveState, StateError, Writer};

const AUDIO_BUFFER_LENGTH: usize = 4096;

static LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14,
    12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
];

static DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

static TRIANGLE_TABLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

static NOISE_TABLE: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

/// Pulse waveform generator.
struct Pulse {
    enabled: bool,
    channel: u8,
    length_enabled: bool,
    length_value: u8,
    timer_period: u16,
    timer_value: u16,
    duty_mode: u8,
    duty_value: u8,
    sweep_reload: bool,
    sweep_enabled: bool,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_period: u8,
    sweep_value: u8,
    envelope_enabled: bool,
    envelope_loop: bool,
    envelope_start: bool,
    envelope_period: u8,
    envelope_value: u8,
    envelope_volume: u8,
    constant_volume: u8,
}

impl Pulse {
    fn new(channel: u8) -> Pulse {
        Pulse {
            enabled: false,
            channel,
            length_enabled: false,
            length_value: 0,
            timer_period: 0,
            timer_value: 0,
            duty_mode: 0,
            duty_value: 0,
            sweep_reload: false,
            sweep_enabled: false,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_period: 0,
            sweep_value: 0,
            envelope_enabled: false,
            envelope_loop: false,
            envelope_start: false,
            envelope_period: 0,
            envelope_value: 0,
            envelope_volume: 0,
            constant_volume: 0,
        }
    }

    fn write_control(&mut self, value: u8) {
        self.duty_mode = (value >> 6) & 3;
        self.length_enabled = ((value >> 5) & 1) == 0;
        self.envelope_loop = ((value >> 5) & 1) == 1;
        self.envelope_enabled = ((value >> 4) & 1) == 0;
        self.envelope_period = value & 15;
        self.constant_volume = value & 15;
        self.envelope_start = true;
    }

    fn write_sweep(&mut self, value: u8) {
        self.sweep_enabled = ((value >> 7) & 1) == 1;
        self.sweep_period = ((value >> 4) & 7) + 1;
        self.sweep_negate = ((value >> 3) & 1) == 1;
        self.sweep_shift = value & 7;
        self.sweep_reload = true;
    }

    fn write_timer_low(&mut self, value: u8) {
        self.timer_period = (self.timer_period & 0xff00) | (value as u16);
    }

    fn write_timer_high(&mut self, value: u8) {
        self.length_value = LENGTH_TABLE[(value >> 3) as usize];
        self.timer_period = (self.timer_period & 0x00ff) | (((value & 7) as u16) << 8);
        self.envelope_start = true;
        self.duty_value = 0;
    }

    fn step_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            self.duty_value = (self.duty_value + 1) % 8;
        } else {
            self.timer_value = self.timer_value.wrapping_sub(1);
        }
    }

    fn step_envelope(&mut self) {
        if self.envelope_start {
            self.envelope_volume = 15;
            self.envelope_value = self.envelope_period;
            self.envelope_start = false;
        } else if self.envelope_value > 0 {
            self.envelope_value -= 1;
        } else {
            if self.envelope_volume > 0 {
                self.envelope_volume -= 1;
            } else if self.envelope_loop {
                self.envelope_volume = 15;
            }
            self.envelope_value = self.envelope_period;
        }
    }

    fn step_sweep(&mut self) {
        if self.sweep_reload {
            if self.sweep_enabled && self.sweep_value == 0 {
                self.sweep();
            }
            self.sweep_value = self.sweep_period;
            self.sweep_reload = false;
        } else if self.sweep_value > 0 {
            self.sweep_value -= 1;
        } else {
            if self.sweep_enabled {
                self.sweep();
            }
            self.sweep_value = self.sweep_period;
        }
    }

    fn step_length(&mut self) {
        if self.length_enabled && self.length_value > 0 {
            self.length_value -= 1;
        }
    }

    fn sweep(&mut self) {
        let delta = self.timer_period >> self.sweep_shift;
        if self.sweep_negate {
            self.timer_period = self.timer_period.wrapping_sub(delta);
            if self.channel == 1 {
                self.timer_period = self.timer_period.wrapping_sub(1);
            }
        } else {
            self.timer_period = self.timer_period.wrapping_add(delta);
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled {
            return 0;
        }
        if self.length_value == 0 {
            return 0;
        }
        if DUTY_TABLE[self.duty_mode as usize][self.duty_value as usize] == 0 {
            return 0;
        }
        if self.timer_period < 8 || self.timer_period > 0x7ff {
            return 0;
        }
        if self.envelope_enabled {
            self.envelope_volume
        } else {
            self.constant_volume
        }
    }
}

/// Triangle waveform generator.
struct Triangle {
    enabled: bool,
    length_enabled: bool,
    length_value: u8,
    timer_period: u16,
    timer_value: u16,
    duty_value: u8,
    counter_period: u8,
    counter_value: u8,
    counter_reload: bool,
}

impl Triangle {
    fn new() -> Triangle {
        Triangle {
            enabled: false,
            length_enabled: false,
            length_value: 0,
            timer_period: 0,
            timer_value: 0,
            duty_value: 0,
            counter_period: 0,
            counter_value: 0,
            counter_reload: false,
        }
    }

    fn write_control(&mut self, value: u8) {
        self.length_enabled = ((value >> 7) & 1) == 0;
        self.counter_period = value & 0x7f;
    }

    fn write_timer_low(&mut self, value: u8) {
        self.timer_period = (self.timer_period & 0xff00) | (value as u16);
    }

    fn write_timer_high(&mut self, value: u8) {
        self.length_value = LENGTH_TABLE[(value >> 3) as usize];
        self.timer_period = (self.timer_period & 0x00ff) | (((value & 7) as u16) << 8);
        self.timer_value = self.timer_period;
        self.counter_reload = true;
    }

    fn step_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            if self.length_value > 0 && self.counter_value > 0 {
                self.duty_value = (self.duty_value + 1) % 32;
            }
        } else {
            self.timer_value = self.timer_value.wrapping_sub(1);
        }
    }

    fn step_length(&mut self) {
        if self.length_enabled && self.length_value > 0 {
            self.length_value -= 1;
        }
    }

    fn step_counter(&mut self) {
        if self.counter_reload {
            self.counter_value = self.counter_period;
        } else if self.counter_value > 0 {
            self.counter_value -= 1;
        }
        if self.length_enabled {
            self.counter_reload = false;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled {
            return 0;
        }
        if self.length_value == 0 {
            return 0;
        }
        if self.counter_value == 0 {
            return 0;
        }
        TRIANGLE_TABLE[self.duty_value as usize]
    }
}

/// Noise generator.
struct Noise {
    enabled: bool,
    mode: bool,
    shift_register: u16,
    length_enabled: bool,
    length_value: u8,
    timer_period: u16,
    timer_value: u16,
    envelope_enabled: bool,
    envelope_loop: bool,
    envelope_start: bool,
    envelope_period: u8,
    envelope_value: u8,
    envelope_volume: u8,
    constant_volume: u8,
}

impl Noise {
    fn new() -> Noise {
        Noise {
            enabled: false,
            mode: false,
            shift_register: 1,
            length_enabled: false,
            length_value: 0,
            timer_period: 0,
            timer_value: 0,
            envelope_enabled: false,
            envelope_loop: false,
            envelope_start: false,
            envelope_period: 0,
            envelope_value: 0,
            envelope_volume: 0,
            constant_volume: 0,
        }
    }

    fn write_control(&mut self, value: u8) {
        self.length_enabled = ((value >> 5) & 1) == 0;
        self.envelope_loop = ((value >> 5) & 1) == 1;
        self.envelope_enabled = ((value >> 4) & 1) == 0;
        self.envelope_period = value & 15;
        self.constant_volume = value & 15;
        self.envelope_start = true;
    }

    fn write_period(&mut self, value: u8) {
        self.mode = (value & 0x80) == 0x80;
        self.timer_period = NOISE_TABLE[(value & 0x0f) as usize];
    }

    fn write_length(&mut self, value: u8) {
        self.length_value = LENGTH_TABLE[(value >> 3) as usize];
        self.envelope_start = true;
    }

    fn step_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            let shift: u8 = if self.mode { 6 } else { 1 };
            let b1 = self.shift_register & 1;
            let b2 = (self.shift_register >> shift) & 1;
            self.shift_register >>= 1;
            self.shift_register |= (b1 ^ b2) << 14;
        } else {
            self.timer_value = self.timer_value.wrapping_sub(1);
        }
    }

    fn step_envelope(&mut self) {
        if self.envelope_start {
            self.envelope_volume = 15;
            self.envelope_value = self.envelope_period;
            self.envelope_start = false;
        } else if self.envelope_value > 0 {
            self.envelope_value -= 1;
        } else {
            if self.envelope_volume > 0 {
                self.envelope_volume -= 1;
            } else if self.envelope_loop {
                self.envelope_volume = 15;
            }
            self.envelope_value = self.envelope_period;
        }
    }

    fn step_length(&mut self) {
        if self.length_enabled && self.length_value > 0 {
            self.length_value -= 1;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled {
            return 0;
        }
        if self.length_value == 0 {
            return 0;
        }
        if (self.shift_register & 1) == 1 {
            return 0;
        }
        if self.envelope_enabled {
            self.envelope_volume
        } else {
            self.constant_volume
        }
    }
}

/// Audio processing unit emulator.
pub struct Apu {
    audio_buffer: [u8; AUDIO_BUFFER_LENGTH],
    audio_buffer_length: usize,
    frame_value: i32,
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,
}

impl Apu {
    pub fn new() -> Apu {
        Apu {
            audio_buffer: [0; AUDIO_BUFFER_LENGTH],
            audio_buffer_length: 0,
            frame_value: 0,
            pulse1: Pulse::new(1),
            pulse2: Pulse::new(2),
            triangle: Triangle::new(),
            noise: Noise::new(),
        }
    }

    fn get_output(&self) -> u8 {
        let pulse_out = 0.00752 * (self.pulse1.output() as f64 + self.pulse2.output() as f64);
        let tnd_out =
            0.00851 * self.triangle.output() as f64 + 0.00494 * self.noise.output() as f64;

        (255.0 * (pulse_out + tnd_out)).floor() as u8
    }

    /// Copy up to `buffer.len()` queued samples into `buffer` (as i8 bit-casts
    /// of the internal u8 samples) and shift the remainder down.
    pub fn output(&mut self, buffer: &mut [i8]) {
        let len = buffer.len().min(self.audio_buffer_length);
        for (dst, &src) in buffer[..len].iter_mut().zip(self.audio_buffer[..len].iter()) {
            *dst = src as i8;
        }
        self.audio_buffer_length -= len;
        self.audio_buffer.copy_within(len..len + self.audio_buffer_length, 0);
    }

    /// Step the APU by one frame.
    pub fn step_frame(&mut self) {
        // Step the frame counter 4 times per frame, for 240Hz
        for i in 0..4 {
            self.frame_value = (self.frame_value + 1) % 5;
            match self.frame_value {
                1 | 3 => {
                    self.step_envelope();
                }
                0 | 2 => {
                    self.step_envelope();
                    self.step_sweep();
                    self.step_length();
                }
                _ => {}
            }

            // Calculate the number of samples needed per 1/4 frame
            let frequency = crate::configuration::audio_frequency();

            // Example: we need 735 samples per frame for 44.1KHz sound sampling
            let frame_rate = crate::configuration::frame_rate();
            let mut samples_to_write = frequency / (frame_rate * 4);
            if i == 3 {
                // Handle the remainder on the final tick of the frame counter
                samples_to_write = (frequency / frame_rate) - 3 * (frequency / (frame_rate * 4));
            }

            // Step the timer ~3729 times per quarter frame for most channels
            let samples_to_write = samples_to_write as usize;
            let mut j = 0usize;
            for step_index in 0..3729 {
                if j < samples_to_write
                    && (step_index as f64 / 3729.0) > (j as f64 / samples_to_write as f64)
                {
                    let sample = self.get_output();
                    let idx = self.audio_buffer_length + j;
                    // Guard against a full buffer (consumer not draining fast
                    // enough): drop the sample rather than overrun. The C++
                    // silently wrote out of bounds here.
                    if idx < self.audio_buffer.len() {
                        self.audio_buffer[idx] = sample;
                    }
                    j += 1;
                }

                self.pulse1.step_timer();
                self.pulse2.step_timer();
                self.noise.step_timer();
                self.triangle.step_timer();
                self.triangle.step_timer();
            }
            self.audio_buffer_length =
                (self.audio_buffer_length + samples_to_write).min(self.audio_buffer.len());
        }
    }

    fn step_envelope(&mut self) {
        self.pulse1.step_envelope();
        self.pulse2.step_envelope();
        self.triangle.step_counter();
        self.noise.step_envelope();
    }

    fn step_sweep(&mut self) {
        self.pulse1.step_sweep();
        self.pulse2.step_sweep();
    }

    fn step_length(&mut self) {
        self.pulse1.step_length();
        self.pulse2.step_length();
        self.triangle.step_length();
        self.noise.step_length();
    }

    fn write_control(&mut self, value: u8) {
        self.pulse1.enabled = (value & 1) == 1;
        self.pulse2.enabled = (value & 2) == 2;
        self.triangle.enabled = (value & 4) == 4;
        self.noise.enabled = (value & 8) == 8;
        if !self.pulse1.enabled {
            self.pulse1.length_value = 0;
        }
        if !self.pulse2.enabled {
            self.pulse2.length_value = 0;
        }
        if !self.triangle.enabled {
            self.triangle.length_value = 0;
        }
        if !self.noise.enabled {
            self.noise.length_value = 0;
        }
    }

    pub fn write_register(&mut self, address: u16, value: u8) {
        match address {
            0x4000 => self.pulse1.write_control(value),
            0x4001 => self.pulse1.write_sweep(value),
            0x4002 => self.pulse1.write_timer_low(value),
            0x4003 => self.pulse1.write_timer_high(value),
            0x4004 => self.pulse2.write_control(value),
            0x4005 => self.pulse2.write_sweep(value),
            0x4006 => self.pulse2.write_timer_low(value),
            0x4007 => self.pulse2.write_timer_high(value),
            0x4008 => self.triangle.write_control(value),
            0x400a => self.triangle.write_timer_low(value),
            0x400b => self.triangle.write_timer_high(value),
            0x400c => self.noise.write_control(value),
            0x400d | 0x400e => self.noise.write_period(value),
            0x400f => self.noise.write_length(value),
            0x4015 => self.write_control(value),
            0x4017 => {
                // C++ falls through into the (empty) default case
                self.step_envelope();
                self.step_sweep();
                self.step_length();
            }
            _ => {}
        }
    }
}

impl_save_state!(Pulse {
    enabled, channel, length_enabled, length_value, timer_period, timer_value,
    duty_mode, duty_value, sweep_reload, sweep_enabled, sweep_negate,
    sweep_shift, sweep_period, sweep_value, envelope_enabled, envelope_loop,
    envelope_start, envelope_period, envelope_value, envelope_volume,
    constant_volume,
});

impl_save_state!(Triangle {
    enabled, length_enabled, length_value, timer_period, timer_value,
    duty_value, counter_period, counter_value, counter_reload,
});

impl_save_state!(Noise {
    enabled, mode, shift_register, length_enabled, length_value, timer_period,
    timer_value, envelope_enabled, envelope_loop, envelope_start,
    envelope_period, envelope_value, envelope_volume, constant_volume,
});

// The queued audio samples are not saved: they belong to the moment the state
// was taken, so loading drops them instead of replaying stale sound.
impl SaveState for Apu {
    fn save(&self, w: &mut Writer) {
        self.frame_value.save(w);
        self.pulse1.save(w);
        self.pulse2.save(w);
        self.triangle.save(w);
        self.noise.save(w);
    }

    fn load(&mut self, r: &mut Reader) -> Result<(), StateError> {
        self.frame_value.load(r)?;
        self.pulse1.load(r)?;
        self.pulse2.load(r)?;
        self.triangle.load(r)?;
        self.noise.load(r)?;
        self.audio_buffer_length = 0;
        Ok(())
    }
}
