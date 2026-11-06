//! `cargo run -p pdfcraft-impose-web --example preflight -- FILE.pdf`: the preflight as JSON.
fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: preflight FILE.pdf");
        std::process::exit(2);
    };
    match std::fs::read(&path).map_err(|e| e.to_string()).and_then(pdfcraft_impose_web::preflight) {
        Ok(p) => println!("{}", serde_json::to_string_pretty(&p).unwrap_or_default()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
