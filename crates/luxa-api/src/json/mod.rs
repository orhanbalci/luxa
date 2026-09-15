//! The JSON format.

mod parse;
mod write;

pub use parse::{MAX_NESTING, ParseError, StateRequest, parse_state};
pub use write::{
    Info, Measure, Names, Wifi, Window, write_effect_data, write_effect_names, write_error,
    write_everything, write_info, write_palette_names, write_state, write_state_and_info,
    write_success,
};
