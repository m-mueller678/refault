use rand::Rng;
use refault::SimBuilder;

fn main() {
    // Retrieve random numbers from different sources
    SimBuilder::default()
        .with_seed(42)
        .run(|| async {
            let mut rng = rand::rng();
            println!("rand: {}", rng.random_range(0..100));
            println!("getrandom: {}", getrandom::u32().unwrap());
        })
        .unwrap();
}
