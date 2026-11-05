//! Save states: a small, dependency-free binary serializer for the engine.
//!
//! A state file is `MAGIC`, a format `VERSION` byte, then every field listed in
//! the `impl_save_state!` invocations, in order, little-endian. Constant data
//! (the CHR ROM, the game's constant tables, the RGB palette) is not stored:
//! it is identical for every state and comes from the ROM/config at startup.

use std::fmt;

const MAGIC: &[u8; 8] = b"SMBCSAVE";
const VERSION: u8 = 1;

#[derive(Debug)]
pub enum StateError {
    Io(std::io::Error),
    BadHeader,
    UnsupportedVersion(u8),
    Truncated,
    TrailingData,
    SizeMismatch,
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::Io(e) => write!(f, "{}", e),
            StateError::BadHeader => write!(f, "not a save state file"),
            StateError::UnsupportedVersion(v) => write!(f, "unsupported save state version {}", v),
            StateError::Truncated => write!(f, "save state file is truncated"),
            StateError::TrailingData => write!(f, "save state file has unexpected trailing data"),
            StateError::SizeMismatch => write!(f, "save state does not match this build"),
        }
    }
}

impl From<std::io::Error> for StateError {
    fn from(e: std::io::Error) -> Self {
        StateError::Io(e)
    }
}

pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Writer {
        let mut buf = Vec::with_capacity(8 * 1024);
        buf.extend_from_slice(MAGIC);
        buf.push(VERSION);
        Writer { buf }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
}

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Result<Reader<'a>, StateError> {
        if data.len() < MAGIC.len() + 1 || &data[..MAGIC.len()] != MAGIC {
            return Err(StateError::BadHeader);
        }
        let version = data[MAGIC.len()];
        if version != VERSION {
            return Err(StateError::UnsupportedVersion(version));
        }
        Ok(Reader { data, pos: MAGIC.len() + 1 })
    }

    pub fn finish(&self) -> Result<(), StateError> {
        if self.pos == self.data.len() {
            Ok(())
        } else {
            Err(StateError::TrailingData)
        }
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], StateError> {
        let end = self.pos.checked_add(n).ok_or(StateError::Truncated)?;
        if end > self.data.len() {
            return Err(StateError::Truncated);
        }
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }
}

/// Something that can be written to / restored from a save state.
pub trait SaveState {
    fn save(&self, w: &mut Writer);
    fn load(&mut self, r: &mut Reader) -> Result<(), StateError>;
}

macro_rules! impl_int {
    ($($t:ty),*) => {$(
        impl SaveState for $t {
            fn save(&self, w: &mut Writer) {
                w.bytes(&self.to_le_bytes());
            }
            fn load(&mut self, r: &mut Reader) -> Result<(), StateError> {
                let b = r.bytes(std::mem::size_of::<$t>())?;
                *self = <$t>::from_le_bytes(b.try_into().unwrap());
                Ok(())
            }
        }
    )*};
}
impl_int!(u8, u16, u32, i32);

impl SaveState for bool {
    fn save(&self, w: &mut Writer) {
        w.bytes(&[*self as u8]);
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), StateError> {
        *self = r.bytes(1)?[0] != 0;
        Ok(())
    }
}

impl<T: SaveState, const N: usize> SaveState for [T; N] {
    fn save(&self, w: &mut Writer) {
        for item in self.iter() {
            item.save(w);
        }
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), StateError> {
        for item in self.iter_mut() {
            item.load(r)?;
        }
        Ok(())
    }
}

/// Fixed-size buffers stored in a `Vec` (e.g. RAM). The length is recorded and
/// must match on load, so a state can never resize engine memory.
impl SaveState for Vec<u8> {
    fn save(&self, w: &mut Writer) {
        (self.len() as u32).save(w);
        w.bytes(self);
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), StateError> {
        let mut len = 0u32;
        len.load(r)?;
        if len as usize != self.len() {
            return Err(StateError::SizeMismatch);
        }
        let b = r.bytes(self.len())?;
        self.copy_from_slice(b);
        Ok(())
    }
}

impl<T: SaveState> SaveState for std::cell::RefCell<T> {
    fn save(&self, w: &mut Writer) {
        self.borrow().save(w);
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), StateError> {
        self.get_mut().load(r)
    }
}

/// Implement `SaveState` for a struct by listing the fields to store, in order.
/// Changing the list changes the file format: bump `VERSION` when you do.
macro_rules! impl_save_state {
    ($t:ty { $($field:ident),* $(,)? }) => {
        impl $crate::state::SaveState for $t {
            fn save(&self, w: &mut $crate::state::Writer) {
                $( $crate::state::SaveState::save(&self.$field, w); )*
            }
            fn load(
                &mut self,
                r: &mut $crate::state::Reader,
            ) -> Result<(), $crate::state::StateError> {
                $( $crate::state::SaveState::load(&mut self.$field, r)?; )*
                Ok(())
            }
        }
    };
}
pub(crate) use impl_save_state;
