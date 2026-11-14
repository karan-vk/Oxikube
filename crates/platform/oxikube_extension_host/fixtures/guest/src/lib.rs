//! Test guest for `oxikube_extension_host::wasm_host` (see `wit/fixture.wit`).

wit_bindgen::generate!({ path: "wit", world: "fixture" });

struct Fixture;

impl Guest for Fixture {
    fn answer() -> u32 {
        42
    }

    fn spin() {
        loop {
            std::hint::black_box(());
        }
    }

    fn read_file(path: String) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|err| err.to_string())
    }

    fn write_file(path: String, contents: String) -> Result<(), String> {
        std::fs::write(path, contents).map_err(|err| err.to_string())
    }

    fn env_var(name: String) -> Option<String> {
        std::env::var(name).ok()
    }

    fn grow_memory(pages: u32) -> bool {
        core::arch::wasm32::memory_grow(0, pages as usize) != usize::MAX
    }

    fn print(line: String) {
        println!("{line}");
    }
}

export!(Fixture);
