use std::fmt::Display;
use std::io::Write;

use crate::CardError;
use crate::parser_utils::OriginalBytes;

pub trait Card: TryFrom<OriginalBytes, Error = CardError> + Display {
    fn original_bytes(&self) -> &[u8];
    fn updated_bytes(&self) -> Vec<u8>;
    /// Write the updated card bytes directly into `writer`, avoiding an intermediate `Vec` allocation.
    fn write_into(&self, writer: &mut dyn Write) -> std::io::Result<()> {
        writer.write_all(&self.updated_bytes())
    }
    /// Returns the original card text. MCNP files are ASCII so the conversion
    /// from bytes is infallible in practice; panics on non-UTF-8 input.
    fn original_text(&self) -> &str {
        std::str::from_utf8(self.original_bytes()).expect("card bytes are not valid UTF-8")
    }
    fn updated_text(&self) -> String {
        String::from_utf8_lossy(&self.updated_bytes()).into_owned()
    }
}
