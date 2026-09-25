#[path = "aom/lib.rs"]
mod aom_decode;
mod decode;
#[allow(nonstandard_style)]
mod ffi;
pub use decode::{Decoder, Image};
