//! Misuse 12: Expensify's `setRequired` is a scalar for dependent tag levels
//! and a per-level array for independent ones. The two constructors take the
//! two shapes, so neither pairing can be written the wrong way round.

use expensify::TagCsvConfig;

fn main() {
    // Dependent levels take one boolean, not one per level.
    let _ = TagCsvConfig::dependent([true, false]);

    // Independent levels take one boolean per level, not a single scalar.
    let _ = TagCsvConfig::independent(true);
}
