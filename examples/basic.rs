//! Demonstrates the key features of the `hive` container.
//!
//! Run with: `cargo +nightly run --example basic`

use hive::Hive;

fn main() {
    println!("=== Hive Container Demo ===\n");

    stable_references();
    insert_and_erase();
    bulk_insertion();
    retain_and_sort();
    iteration();
}

/// Stable references: element pointers persist after insertions/erasures.
fn stable_references() {
    println!("--- Stable References ---");

    let mut hive: Hive<&str> = Hive::new();
    let hello = hive.insert("hello");
    let world = hive.insert("world");
    let rust = hive.insert("rust");

    println!(
        "Inserted: hello at {:p}, world at {:p}, rust at {:p}",
        hello, world, rust
    );

    // Insert many more elements, pointers to originals remain valid
    for _i in 0..10_000 {
        hive.insert("filler");
    }

    unsafe {
        println!(
            "After 10000 inserts, hello still at {:p}: {}",
            hello, *hello
        );
        println!(
            "After 10000 inserts, world still at {:p}: {}",
            world, *world
        );
        println!("After 10000 inserts, rust  still at {:p}: {}", rust, *rust);
    }
    println!();
}

/// Insert, erase, and re-use erased memory slots.
fn insert_and_erase() {
    println!("--- Insert and Erase ---");

    let mut hive = Hive::new();
    let _p1 = hive.insert(1);
    let p2 = hive.insert(2);
    let _p3 = hive.insert(3);

    println!("Before erase: {:?}", hive);
    assert_eq!(hive.len(), 3);

    // Erase the middle element
    unsafe {
        hive.erase(p2);
    }
    println!("After erasing 2: {:?}", hive);
    assert_eq!(hive.len(), 2);

    // New insert reuses the freed slot
    let p4 = hive.insert(99);
    println!("Inserted 99 into freed slot at {:p}", p4);
    println!("After re-insert:   {:?}", hive);
    println!();
}

/// Bulk insertion and iteration.
fn bulk_insertion() {
    println!("--- Bulk Insertion ---");

    let mut hive = Hive::with_capacity(100_000);
    for i in 0..100_000 {
        hive.insert(i);
    }
    println!(
        "Inserted {} elements, capacity: {}",
        hive.len(),
        hive.capacity()
    );

    let sum: u64 = hive.iter().map(|&x| x as u64).sum();
    println!("Sum of all elements: {sum}");
    println!();
}

/// Retain and sort operations.
fn retain_and_sort() {
    println!("--- Retain and Sort ---");

    let mut hive: Hive<i32> = (0..20).collect();
    println!("Initial: {:?}", hive);

    // Remove odd numbers
    hive.retain(|&x| x % 2 == 0);
    println!("After retain(even): {:?}", hive);

    // Sort descending
    hive.sort_by(|a, b| b.cmp(a));
    println!("Sorted descending: {:?}", hive);
    println!();
}

/// Bidirectional iteration.
fn iteration() {
    println!("--- Bidirectional Iteration ---");

    let mut hive = Hive::new();
    for c in 'a'..='j' {
        hive.insert(c);
    }

    print!("Forward:  ");
    for &c in hive.iter() {
        print!("{c} ");
    }
    println!();

    print!("Backward: ");
    for &c in hive.iter().rev() {
        print!("{c} ");
    }
    println!();

    print!("Mixed:    ");
    let mut it = hive.iter();
    for _ in 0..5 {
        print!("{} ", it.next().unwrap());
        print!("{} ", it.next_back().unwrap());
    }
    println!();
}
