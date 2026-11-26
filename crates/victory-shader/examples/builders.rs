//! Lists the saved containers that are programs of the ray-tracing
//! structure builder (a spilled user-data table), and how each translates:
//! `builders <dir of .cdxil>`. `VICTORY_HOST_WAVE=32` as for a 32-lane host.
use victory_shader::{Container, FourCC};
fn main() {
    if let Some(n) = std::env::var("VICTORY_HOST_WAVE").ok().and_then(|v| v.parse().ok()) {
        victory_shader::emit::set_host_max_wave(n);
    }
    let dir = std::env::args().nth(1).expect("dir");
    let mut names: Vec<_> = std::fs::read_dir(&dir).expect("dir").filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    for path in names {
        if path.extension().is_none_or(|e| e != "cdxil") {
            continue;
        }
        let bytes = std::fs::read(&path).expect("read");
        let Ok(c) = Container::parse(&bytes) else { continue };
        let Some(programs) = c.part(FourCC::DXIL).and_then(|p| victory_shader::native_programs(p.data).ok()) else {
            continue;
        };
        if !programs.iter().any(victory_shader::spill::is_spilled) {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let code: usize = programs.iter().map(|p| p.code.len()).sum();
        match victory_shader::emit::translate(&bytes) {
            Ok(h) => {
                let others: Vec<String> = h
                    .user_data
                    .iter()
                    .filter_map(|u| match u.root {
                        victory_shader::RootType::Other(k) => Some(format!("{k:#x}/{}", u.slot)),
                        _ => None,
                    })
                    .collect();
                println!("{name}	code {code}	ok	{} bytes	{}", h.dxil.len(), others.join(" "))
            }
            Err(e) => println!("{name}\tcode {code}\tFAILS\t{e}"),
        }
    }
}
