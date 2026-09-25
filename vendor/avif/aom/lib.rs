pub type Result<T, E = Error> = std::result::Result<T, E>;
pub use yuv::color;

mod error;
pub use error::Error;

mod aom;
pub use aom::Config;
pub use aom::Decoder;
pub use aom::FrameMeta;
pub use aom::FrameTempRef;
pub use aom::RowsIter;
pub use aom::RowsIters;

/// Helper functions for undoing chroma subsampling
pub mod chroma;
