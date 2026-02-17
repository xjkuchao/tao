
mod tables;
use tables::{MPA_HUFF_LENS, MPA_HUFF_OFFSET};

fn main() {
    println!("MPA_HUFF_LENS length: {}", MPA_HUFF_LENS.len());
    println!("Last offset (Table 33): {}", MPA_HUFF_OFFSET[33]);
    println!("Expected length: {}", MPA_HUFF_OFFSET[33] + 16);
}
