fn main() {
    let mut s = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).unwrap();
    let kind = std::env::args().nth(1).unwrap_or_default();
    println!("{}", s.len() + kind.len());
}
