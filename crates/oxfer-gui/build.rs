use std::path::PathBuf;

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>oxfer</title></head>
<body><p>The oxfer frontend has not been built. Run <code>npm ci &amp;&amp; npm run build</code>
in <code>ui/</code>, then rebuild <code>oxfer-gui</code>.</p></body></html>
"#;

/// `tauri::generate_context!` embeds `ui/dist` and fails to compile when it
/// is missing. Put a placeholder page there so that `cargo build/test` work
/// in a fresh checkout; `npm run build` replaces it.
fn ensure_frontend_dist() {
    let dist = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"))
        .join("../../ui/dist");
    println!("cargo:rerun-if-changed=../../ui/dist");
    if dist.join("index.html").exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dist)
        .and_then(|()| std::fs::write(dist.join("index.html"), PLACEHOLDER))
    {
        println!("cargo:warning=cannot create placeholder frontend: {e}");
        return;
    }
    println!(
        "cargo:warning=ui/dist was missing; embedded a placeholder page (run `npm run build` in ui/)"
    );
}

fn main() {
    ensure_frontend_dist();
    tauri_build::build();
}
