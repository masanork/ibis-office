// SPDX-License-Identifier: AGPL-3.0-only

//! Timings for synchronous report refresh benchmarks.

use std::{
    cell::Cell,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default)]
pub struct Timings {
    pub schema: Duration,
    /// Includes schema inference.
    pub stage: Duration,
    pub sql: Duration,
    pub write: Duration,
}

thread_local! {
    static SINGLE_ROW_INSERTS: Cell<bool> = const { Cell::new(false) };
    static TIMINGS: Cell<Timings> = const { Cell::new(Timings {
        schema: Duration::ZERO,
        stage: Duration::ZERO,
        sql: Duration::ZERO,
        write: Duration::ZERO,
    }) };
}

/// Select the comparison path for a report staging benchmark on this thread.
pub fn force_single_row_inserts() {
    SINGLE_ROW_INSERTS.set(true);
}

pub(super) fn single_row_inserts_forced() -> bool {
    SINGLE_ROW_INSERTS.get()
}

pub fn take() -> Timings {
    TIMINGS.take()
}

pub enum Phase {
    Schema,
    Stage,
    Sql,
    Write,
}

pub struct Timer(Phase, Instant);

impl Timer {
    pub fn start(phase: Phase) -> Self {
        Self(phase, Instant::now())
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        TIMINGS.with(|timings| {
            let mut value = timings.get();
            let elapsed = self.1.elapsed();
            match self.0 {
                Phase::Schema => value.schema += elapsed,
                Phase::Stage => value.stage += elapsed,
                Phase::Sql => value.sql += elapsed,
                Phase::Write => value.write += elapsed,
            }
            timings.set(value);
        });
    }
}
