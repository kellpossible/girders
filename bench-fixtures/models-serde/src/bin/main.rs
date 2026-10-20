fn main() {
    let mut s = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).unwrap();
    println!("{}", models_serde::run(&std::env::args().nth(1).unwrap_or_default(), &s));
}
