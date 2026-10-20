fn main() {
    use std::mem::size_of;
    println!(
        "girders Value<Ordered>: {}",
        size_of::<girders_json::Value<girders_json::Ordered>>()
    );
    println!(
        "girders Node<Ordered>:  {}",
        size_of::<girders_json::Node<girders_json::Ordered>>()
    );
    println!("girders Number:         {}", size_of::<girders::Number>());
    println!("girders Span:           {}", size_of::<girders::Span>());
    println!(
        "Option<Span>:           {}",
        size_of::<Option<girders::Span>>()
    );
    println!(
        "ordered Entry:          {}",
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
