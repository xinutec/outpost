//! The parts of `outpost` that are worth testing without a session to talk to.
//!
//! The binary next door is argument parsing, printing and a poll loop. What
//! sits here is everything that reads a machine's answer and decides what it
//! means: the transports in [`remote`], where to point from [`config`], and the
//! transcript rules in [`transcript`].
//!
//! **The split exists so the tests can link this as an outside user.** Tests in
//! `tests/` reach only the public surface, so what they pin is behaviour rather
//! than the shape of a private helper — which matters most for
//! [`transcript::endings`], whose whole job is to tell a killed background task
//! from a completed one, on rows nothing here controls the format of.
pub mod args;
pub mod config;
pub mod remote;
pub mod transcript;
