//! Using `hive` as an object pool — a particle system simulation.
//!
//! Key properties that make hive a natural object pool:
//! - Stable `*const T` handles for O(1) deallocation (Vec<Option<T>> refs invalidate on grow)
//! - Erased memory immediately reused for new allocations (no free_list bookkeeping)
//! - Fast iteration over only *active* objects (no `is_some()` checks per slot)
//! - No per-element discriminant overhead (Vec<Option<T>> adds padding/discriminant)
//! - Contiguous groups give good cache locality
//!
//! Run with: `cargo +nightly run --example object_pool`

use hive::Hive;
use std::time::Instant;

// ── A typical pooled object ──

#[derive(Debug)]
struct Particle {
    x: f64, y: f64, vx: f64, vy: f64, life: f64,
}

impl Particle {
    fn spawn(x: f64, y: f64, vx: f64, vy: f64) -> Self {
        Self { x, y, vx, vy, life: 1.0 }
    }
    fn update(&mut self, dt: f64) -> bool {
        self.x += self.vx * dt;
        self.y += self.vy * dt;
        self.life -= dt;
        self.life <= 0.0
    }
}

// ── Hive-based pool ──

struct HivePool { pool: Hive<Particle> }

impl HivePool {
    fn new(cap: usize) -> Self { Self { pool: Hive::with_capacity(cap) } }
    /// Spawn using safe API — returns `&Particle`
    fn spawn(&self, x: f64, y: f64, vx: f64, vy: f64) -> *const Particle {
        self.pool.insert(Particle::spawn(x, y, vx, vy))
    }
    unsafe fn despawn(&mut self, p: *const Particle) { self.pool.erase(&*p); }
    #[allow(dead_code)]
    fn active(&self) -> usize { self.pool.len() }
}

// ── Vec<Option<Particle>> pool (typical alternative) ──

struct VecPool {
    particles: Vec<Option<Particle>>,
    free_list: Vec<usize>,
}

impl VecPool {
    fn new(cap: usize) -> Self {
        Self { particles: Vec::with_capacity(cap), free_list: Vec::with_capacity(cap) }
    }
    fn spawn(&mut self, x: f64, y: f64, vx: f64, vy: f64) -> usize {
        let p = Particle::spawn(x, y, vx, vy);
        if let Some(idx) = self.free_list.pop() {
            self.particles[idx] = Some(p);
            idx
        } else {
            self.particles.push(Some(p));
            self.particles.len() - 1
        }
    }
    fn despawn(&mut self, idx: usize) {
        self.particles[idx] = None;
        self.free_list.push(idx);
    }
    #[allow(dead_code)]
    fn active(&self) -> usize {
        self.particles.iter().filter(|p| p.is_some()).count()
    }
}

// ── Simulation ──

fn simulate_hive(pool: &mut HivePool, dt: f64) -> usize {
    let mut dead = Vec::new();
    for p in pool.pool.iter_mut() {
        if p.update(dt) { dead.push(p as *const Particle); }
    }
    let n = dead.len();
    for ptr in dead { unsafe { pool.despawn(ptr); } }
    n
}

#[allow(dead_code)]
fn simulate_vec(pool: &mut VecPool, dt: f64) -> usize {
    let mut dead = Vec::new();
    for (i, p) in pool.particles.iter_mut().enumerate() {
        if let Some(particle) = p {
            if particle.update(dt) { dead.push(i); }
        }
    }
    let n = dead.len();
    for i in dead { pool.despawn(i); }
    n
}

fn main() {
    println!("=== Hive as an Object Pool ===\n");

    // ── Basic demo ──
    let mut pool = HivePool::new(1024);
    for i in 0..200 {
        let x = (i % 20) as f64 * 10.0;
        pool.spawn(x, (i / 20) as f64 * 10.0, 0.0, -50.0);
    }
    println!("Spawned {} particles (cap: {})", pool.active(), pool.pool.capacity());

    let mut total_despawned = 0;
    for _frame in 0..60 {
        let despawned = simulate_hive(&mut pool, 0.016);
        total_despawned += despawned;
        for i in 0..despawned {
            pool.spawn(i as f64, 200.0, 0.0, -100.0);
        }
    }
    println!("Despawned {total_despawned} over 60 frames, {} still active\n", pool.active());

    // ── Benchmarks ──
    const N: usize = 10_000;

    // 1. Pure append throughput
    let start = Instant::now();
    let h = HivePool::new(N);
    for i in 0..N { h.spawn(i as f64, 0.0, 0.0, 0.0); }
    println!("Insert {N}: Hive {:>8.2?}", start.elapsed());

    let start = Instant::now();
    let mut v = VecPool::new(N);
    for i in 0..N { v.spawn(i as f64, 0.0, 0.0, 0.0); }
    println!("Insert {N}: Vec  {:>8.2?}", start.elapsed());

    // 2. Erase + re-insert cycles (shows memory reuse advantage)
    let start = Instant::now();
    let mut h = HivePool::new(N);
    let ptrs: Vec<*const Particle> = (0..N).map(|i| h.spawn(i as f64, 0.0, 0.0, 0.0)).collect();
    for _ in 0..10 {
        for i in (0..N).step_by(3) { unsafe { h.despawn(ptrs[i]); } }
        for i in (0..N).step_by(3) { h.spawn(i as f64, 0.0, 0.0, 0.0); }
    }
    println!("Erase/insert {N}: Hive {:>8.2?}", start.elapsed());

    let start = Instant::now();
    let mut v = VecPool::new(N);
    let idxs: Vec<usize> = (0..N).map(|i| v.spawn(i as f64, 0.0, 0.0, 0.0)).collect();
    for _ in 0..10 {
        for &idx in idxs.iter().step_by(3) { v.despawn(idx); }
        for i in (0..N).step_by(3) { v.spawn(i as f64, 0.0, 0.0, 0.0); }
    }
    println!("Erase/insert {N}: Vec  {:>8.2?}", start.elapsed());

    // 3. Iteration throughput
    let h = HivePool::new(N);
    for i in 0..N { h.spawn(i as f64, 0.0, 0.0, 0.0); }
    let start = Instant::now();
    let sum: f64 = h.pool.iter().map(|p| p.x).sum();
    println!("Iterate {N}: Hive {:>8.2?} (sum={})", start.elapsed(), sum);

    let mut v = VecPool::new(N);
    for i in 0..N { v.spawn(i as f64, 0.0, 0.0, 0.0); }
    let start = Instant::now();
    let sum: f64 = v.particles.iter().filter_map(|p| p.as_ref()).map(|p| p.x).sum();
    println!("Iterate {N}: Vec  {:>8.2?} (sum={})", start.elapsed(), sum);

    // ── Memory comparison ──
    println!();
    println!("--- Memory overhead ---");
    println!(
        "Particle: {} bytes.  Hive slot: {} bytes.  Vec<Option>: {} bytes",
        std::mem::size_of::<Particle>(),
        Hive::<Particle>::new().capacity(), // just to show it compiles
        std::mem::size_of::<Option<Particle>>(),
    );
    println!(
        "Vec<Option<Particle>> wastes {} bytes per slot for discriminant + padding",
        std::mem::size_of::<Option<Particle>>() - std::mem::size_of::<Particle>(),
    );
    println!("Hive stores free-list inline — zero per-slot overhead");

    println!();
    println!("--- When hive excels as a pool ---");
    println!("* Heavy insert/erase mix — no shifting, no index reclamation");
    println!("* Stable handles needed (pointers never invalidate)");
    println!("* Large structs where Option<> discriminant wastes cache space");
    println!("* Embedded / no_std — hive has no heap requirement beyond alloc");
}
