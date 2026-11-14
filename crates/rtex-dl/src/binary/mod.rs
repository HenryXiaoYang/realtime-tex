//! Binary display-list encoding, revision 1 (docs/display-list.md): decoder and encoder.

mod decode;
mod encode;
#[cfg(test)]
mod tests;

pub use decode::decode;
pub use encode::encode;

use thiserror::Error;

pub const MAGIC: &[u8; 4] = b"RTDL";

pub const VERSION: u16 = 1;

#[derive(Debug, Error)]
pub enum BinError {
    #[error("truncated at offset {0}")]
    Truncated(usize),
    #[error("bad magic")]
    BadMagic,
    #[error("unsupported version {0}")]
    Version(u16),
    #[error("malformed record {tag:#x} at offset {offset}")]
    Malformed { tag: u8, offset: usize },
    #[error("invalid utf-8 in string")]
    Utf8,
}
