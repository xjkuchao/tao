//! tao-probe 入口.

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    std::process::exit(tao_probe::run(argv));
}
