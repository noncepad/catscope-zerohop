fn main() {
    #[cfg(feature = "capacity")]
    tonic_build::compile_protos("proto/relay.proto")
        .expect("failed to compile proto/relay.proto");
}
