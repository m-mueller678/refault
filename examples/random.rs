use deterministic_simulation_core::runtime::Runtime;
use rand::Rng;

fn main() {
    let runtime = Runtime::default().with_seed(42);
    // Retrieve random numbers from different sources
    runtime.run(|| async {
        let mut rng = rand::rng();
        println!("rand: {}", rng.random_range(0..100));

        println!("getrandom: {}", getrandom::u32().unwrap());
    });
}
