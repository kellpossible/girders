fn main() {
    use std::mem::size_of;
    println!(
        "girders Value:          {}",
        size_of::<girders_json::Value>()
    );
    println!(
        "girders Node:           {}",
        size_of::<girders_json::Node>()
    );
    println!("girders Number:         {}", size_of::<girders::Number>());
    println!("girders Span:           {}", size_of::<girders::Span>());
    println!(
        "Option<Span>:           {}",
        size_of::<Option<girders::Span>>()
    );
    println!(
        "object Entry:           {}",
        size_of::<girders_json::value::Entry>()
    );
    println!("serde_json Value:       {}", size_of::<serde_json::Value>());
    println!(
        "DecodeError<Span>:      {}",
        size_of::<girders::DecodeError<girders::Span>>()
    );
    println!(
        "Result<u64, DecodeError<Span>>: {}",
        size_of::<Result<u64, girders::DecodeError<girders::Span>>>()
    );
}
