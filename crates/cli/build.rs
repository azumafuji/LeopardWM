fn main() {
    let res = winresource::WindowsResource::new();
    res.compile().expect("Failed to compile Windows resources");
}
