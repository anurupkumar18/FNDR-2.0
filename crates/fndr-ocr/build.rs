fn main() {
    // The wrapper resolves VN* classes at runtime via objc2 class!, which
    // only works when Vision is loaded into the process. Linking it here
    // makes every consumer of this crate correct by construction (the v1 app
    // only worked because the shell happened to load it).
    println!("cargo:rustc-link-lib=framework=Vision");
}
