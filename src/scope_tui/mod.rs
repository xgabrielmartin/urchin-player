#![allow(clippy::needless_range_loop, clippy::single_match)]

pub mod display;
pub mod music;

pub type Sample = f64;
pub type ChannelTape = Vec<Sample>;
pub type SampleTape = Vec<ChannelTape>;
