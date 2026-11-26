//! Translate saved containers: `trans <file.cdxil>..` (prints the outcome).
//! `VICTORY_HOST_WAVE=32` translates as for a 32-lane host.
fn main() {
    if let Some(n) = std::env::var("VICTORY_HOST_WAVE").ok().and_then(|v| v.parse().ok()) {
        victory_shader::emit::set_host_max_wave(n);
    }
    for a in std::env::args().skip(1) {
        let bytes = std::fs::read(&a).expect("read");
        match victory_shader::emit::translate(&bytes) {
            Ok(h) => println!("{a}: ok, {} bytes of DXIL", h.dxil.len()),
            Err(e) => println!("{a}: does not translate: {e}"),
        }
    }
}
