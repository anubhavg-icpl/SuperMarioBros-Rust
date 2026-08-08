//! NES game controller emulation (ported from Controller.cpp).

#[derive(Copy, Clone)]
pub enum Button {
    A = 0,
    B = 1,
    Select = 2,
    Start = 3,
    Up = 4,
    Down = 5,
    Left = 6,
    Right = 7,
}

pub struct Controller {
    button_states: [bool; 8],
    button_index: u8,
    strobe: u8,
}

impl Controller {
    pub fn new() -> Controller {
        Controller {
            button_states: [false; 8],
            button_index: 0,
            strobe: 1,
        }
    }

    pub fn read_byte(&mut self) -> u8 {
        let mut value = 1;
        if self.button_index < 8 {
            value = if self.button_states[self.button_index as usize] {
                0x41
            } else {
                0x40
            };
        }
        if (self.strobe & 1) == 0 {
            self.button_index += 1;
        }
        value
    }

    pub fn set_button_state(&mut self, button: Button, state: bool) {
        self.button_states[button as usize] = state;
    }

    pub fn write_byte(&mut self, value: u8) {
        if (value & 1) == 0 && (self.strobe & 1) == 1 {
            self.button_index = 0;
        }
        self.strobe = value;
    }
}
