#![no_std]
//! Throwaway spike for girders' codec shape. no_std + alloc check.
extern crate alloc;

pub mod core;
pub mod dispatch;
pub mod prim;
pub mod record;
pub mod tok;

pub mod prelude {
    pub use crate::core::*;
    pub use crate::dispatch::*;
    pub use crate::prim::*;
    pub use crate::record::*;
    pub use crate::tok::{Tok, decode, encode};
}
