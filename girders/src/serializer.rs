//! The format-side contract for encoding.
//!
//! Containers use per-element framing so the contract isn't shaped
//! around any one format: `begin_*` receive the element count when known
//! (length-prefixed formats need it), `array_element`/`object_key` run
//! before each element or entry, `end_*` close.

pub trait Serializer {
    type Error;

    fn write_bool(&mut self, v: bool) -> Result<(), Self::Error>;
    fn write_null(&mut self) -> Result<(), Self::Error>;
    fn write_i64(&mut self, v: i64) -> Result<(), Self::Error>;
    fn write_u64(&mut self, v: u64) -> Result<(), Self::Error>;
    fn write_f64(&mut self, v: f64) -> Result<(), Self::Error>;
    fn write_f32(&mut self, v: f32) -> Result<(), Self::Error>;
    /// A number given as text (`i128`, big integers, exact decimals).
    /// The caller guarantees it is a valid number for the format.
    fn write_number_text(&mut self, v: &str) -> Result<(), Self::Error>;
    fn write_str(&mut self, v: &str) -> Result<(), Self::Error>;

    fn begin_array(&mut self, len: Option<usize>) -> Result<(), Self::Error>;
    fn array_element(&mut self) -> Result<(), Self::Error>;
    fn end_array(&mut self) -> Result<(), Self::Error>;

    fn begin_object(&mut self, len: Option<usize>) -> Result<(), Self::Error>;
    fn object_key(&mut self, key: &str) -> Result<(), Self::Error>;
    fn end_object(&mut self) -> Result<(), Self::Error>;
}
