use rand::Rng;
use refault::SimBuilder;

fn main() {
    let runtime = SimBuilder::default().with_seed(42);
    // Retrieve random numbers from different sources
    runtime.run(|| async {
        let mut rng = rand::rng();
        println!("rand: {}", rng.random_range(0..100));

        println!("getrandom: {}", getrandom::u32().unwrap());
    });
}
