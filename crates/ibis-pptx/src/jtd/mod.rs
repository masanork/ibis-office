// SPDX-License-Identifier: AGPL-3.0-only
//! Bounded decoding of a JTD DocumentText stream.
//!
//! This is an experimental building block, not a file importer. It neither
//! opens compound containers nor preserves all layout and embedded objects.
//! Format observations: <https://github.com/OpenJTD/rjtd/tree/cfb44c9ac53126e00808c49709fb4dade8a4daae/openjtd-spec/rfc>.

pub mod decode;
