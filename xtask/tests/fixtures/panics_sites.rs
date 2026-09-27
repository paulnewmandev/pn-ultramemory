// SPDX-License-Identifier: Apache-2.0
//! Fixture for the panic ratchet: one construct of each kind it collects.
//!
//! The guard must report four findings, keyed by the expression text and the enclosing function and
//! by no line number at all.

/// A table of rows, indexed without bounds checking.
pub struct Table {
    /// The rows held by the table.
    rows: Vec<String>,
}

impl Table {
    /// Returns one cell by index, which panics when the index is out of range.
    pub fn cell(&self, index: usize) -> &str {
        &self.rows[index]
    }

    /// Returns the row count narrowed to 32 bits, which truncates above four billion rows.
    pub fn count(&self) -> u32 {
        let total = self.rows.len();
        total as u32
    }

    /// Returns the first row, panicking when the table is empty.
    pub fn first(&self) -> &str {
        self.rows.first().unwrap()
    }

    /// Refuses to answer at all.
    pub fn summary(&self) -> String {
        unimplemented!("the summary is not written yet")
    }
}
