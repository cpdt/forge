fn main() {
    windres::Build::new()
        .compile("manifest/Resource.rc")
        .expect("failed to include resources in the dll");
}
