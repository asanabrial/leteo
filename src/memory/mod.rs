//! What a memory is, with no database in sight.
//!
//! These three answer the questions that have nothing to do with storage: what
//! a valid memory looks like, what its fields mean once tidied, which kinds go
//! stale, and what one memory may claim about another.
//!
//! They are grouped because they share a property worth protecting — measured,
//! not asserted: nothing here imports `store`, `mcp`, `cli`, `tui`, `cloud` or
//! `sync`, and nothing here opens a connection. That is what makes them
//! testable without a database, and what makes them the one place a rule can
//! live so that the several paths which merely persist a memory cannot each
//! decide differently. They had already drifted once, on six of eight
//! invariants, when the rules lived inside the SQLite adapter.
//!
//! Keeping the boundary is a matter of not importing upward from here. There is
//! no ceremony of ports enforcing it: Leteo has one storage engine and is not
//! looking for a second.

pub mod model;
pub mod normalize;
pub mod rules;
