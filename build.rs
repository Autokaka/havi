fn main() {
    #[cfg(feature = "napi-binding")]
    napi_build::setup();
}
