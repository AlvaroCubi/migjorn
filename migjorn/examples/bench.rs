///! Use with: `cargo run -r --example bench`
use std::time;

use migjorn::Model;

fn main() {
    let timer = time::Instant::now();
    let model = Model::from_file("resources/untracked/big.mcnp").unwrap();
    println!("Model loaded in: {:?}", timer.elapsed());

    let timer = time::Instant::now();
    model.validation_checks().unwrap();
    println!("Model validated in: {:?}", timer.elapsed());
}
