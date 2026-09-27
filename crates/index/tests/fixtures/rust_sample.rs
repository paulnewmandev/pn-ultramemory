// SPDX-License-Identifier: Apache-2.0
//! Fixture for the extraction tests: a small inventory module. It is never compiled.

use std::collections::{BTreeMap, HashSet};
use std::fmt::{self, Display};
use crate::store::Store as Backend;

/// Maximum number of items an inventory holds.
pub const MAX_ITEMS: usize = 1024;
const SECRET_SEED: u64 = 42;
static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A stock keeping unit.
///
/// Identifies one product.
#[derive(Debug, Clone, PartialEq)]
pub struct Sku {
    code: String,
    pub weight: f32,
}

/// Anything that can be priced.
pub trait Priced: Display + Send {
    /// The price in cents.
    fn cents(&self) -> u64;

    /// The price with tax applied.
    fn with_tax(&self, rate: f64) -> u64 {
        (self.cents() as f64 * (1.0 + rate)).round() as u64
    }

    fn hidden_in_trait(&self);
}

pub enum Kind {
    Tool,
    Toy(u8),
}

pub(crate) struct Inventory<T: Priced> {
    items: BTreeMap<String, T>,
    seen: HashSet<String>,
}

impl<T: Priced> Inventory<T> {
    /// Creates an empty inventory.
    pub fn new() -> Self {
        Inventory { items: BTreeMap::new(), seen: HashSet::new() }
    }

    /// Adds an item unless the inventory is full.
    #[must_use]
    pub fn add(
        &mut self,
        key: &str,
        item: T,
    ) -> Result<(), Error> {
        if self.items.len() >= MAX_ITEMS {
            return Err(Error::Full);
        }
        self.seen.insert(key.to_owned());
        self.items.insert(key.to_owned(), item);
        log_change(key);
        Ok(())
    }

    fn total(&self) -> u64 {
        self.items.values().map(|i| i.cents()).sum()
    }
}

impl Display for Sku {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code)
    }
}

/// Logs a change.
pub fn log_change(key: &str) {
    println!("changed {key}");
    Backend::global().record(key);
}

pub mod report {
    //! Reporting helpers.
    use super::Inventory;

    /// Renders an inventory.
    pub fn render<T: super::Priced>(inv: &Inventory<T>) -> String {
        format!("{} items", inv.len())
    }

    fn private_helper() {}
}

macro_rules! sku {
    ($code:expr) => { Sku { code: $code.to_owned(), weight: 0.0 } };
}

type Table = BTreeMap<String, Sku>;

fn main() {
    let inv: Inventory<Sku> = Inventory::new();
    inv.add("a", sku!("A1")).ok();
}
