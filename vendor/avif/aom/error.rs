use core::num::NonZeroU32;
use quick_error::quick_error;

quick_error! {
    #[derive(Debug)]
    #[non_exhaustive]
    pub enum Error {
        AOM(code: NonZeroU32, msg: Option<String>) {
            display("{} ({})", msg.as_deref().unwrap_or("libaom error"), code)
        }
        Unsupported(msg: &'static str) {
            display("{}", msg)
        }
    }
}
