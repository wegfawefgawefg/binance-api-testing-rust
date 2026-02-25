fn main() {
    println!("public_data_streaming reference demos:");
    println!("  1) Dynamic SUBSCRIBE/UNSUBSCRIBE:");
    println!("     cargo run -p public_data_streaming --bin dynamic_subscriptions");
    println!("  2) Fixed URL stream subscription:");
    println!("     cargo run -p public_data_streaming --bin fixed_url_stream -- --symbol ethusdt");
}
