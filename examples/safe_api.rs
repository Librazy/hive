//! Demonstrates the safe reference-based API (`insert_ref`) vs raw pointers.
//!
//! `insert_ref(&self) -> &T` lets you hold multiple live `&T` references
//! from separate insertions, since it takes `&self` (not `&mut self`).
//! Erasing still requires `&mut self` to uphold Rust's aliasing rules.
//!
//! Run with: `cargo +nightly run --example safe_api`

use hive::Hive;

fn main() {
    println!("=== Safe Reference API ===\n");

    // ── Multiple live references ──
    println!("--- Multiple live references from insert_ref ---");
    let hive = Hive::new();
    let a = hive.insert_ref(String::from("alpha"));
    let b = hive.insert_ref(String::from("beta"));
    let c = hive.insert_ref(String::from("gamma"));
    // a, b, c all live simultaneously — no borrow conflicts
    println!("  a = {a}, b = {b}, c = {c}");
    println!("  len = {}", hive.len());
    println!();

    // ── Stable under growth ──
    println!("--- Stable references under growth ---");
    let hive = Hive::new();
    let first = hive.insert_ref(0u32);
    let second = hive.insert_ref(1u32);
    assert_eq!(*first, 0);
    assert_eq!(*second, 1);

    // Insert 10000 more — original refs stay valid
    for i in 2..10_002 {
        hive.insert_ref(i);
    }
    assert_eq!(*first, 0);
    assert_eq!(*second, 1);
    println!(
        "  After 10000 inserts, first={}, second={}",
        *first, *second
    );
    println!("  len = {}", hive.len());
    println!();

    // ── Borrow-checker enforced erase safety ──
    println!("--- Erase requires borrowed refs to be dropped first ---");
    let mut hive = Hive::new();
    {
        let a = hive.insert_ref(1);
        let _b = hive.insert_ref(2);
        // hive.erase(a); // Would not compile — a borrows &self, erase needs &mut self
        assert_eq!(*a, 1);
    }
    // Manual erase via raw pointer still works
    let p = hive.insert(99);
    unsafe {
        hive.erase(&*p);
    }
    println!("  After manual erase with raw ptr, len = {}", hive.len());
    println!();

    // ── Mutable reference ──
    println!("--- insert_ref_mut (takes &mut self) ---");
    let mut hive = Hive::new();
    {
        let r = hive.insert_ref_mut(42);
        *r = 99;
        // hive.insert(1); // Would not compile
    }
    // Now it's fine
    hive.insert(1);
    let vals: Vec<i32> = hive.iter().copied().collect();
    println!("  Values after mutation: {:?}", vals);
    println!();

    // ── Comparison with raw pointer API ──
    println!("--- API comparison ---");
    println!("  insert(value)      -> *const T   | &mut self | raw pointer, zero overhead");
    println!("  insert_ref(value)  -> &T         | &self     | safe, multiple live refs");
    println!("  insert_ref_mut(v)  -> &mut T     | &mut self | safe, single mutable ref");
    println!("  erase(&T)          -> ()         | &mut self | raw or ref from insert_ref");
    println!();
    println!("  Use insert_ref for: multi-ref spawn phases, shareable handles");
    println!("  Use insert for:    zero-overhead, storing in external hash maps");
}
