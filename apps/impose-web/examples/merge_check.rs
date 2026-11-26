//! `cargo run -p pdfcraft-impose-web --example merge_check -- OUT.pdf A.pdf B.pdf ...`: every page of every
//! input, in order, through `assemble` (the engine call behind the "Gabung" panel and the page editor).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: merge_check OUT.pdf A.pdf B.pdf ...");
        std::process::exit(2);
    }
    let mut files = Vec::new();
    let mut plan = Vec::new();
    for (i, p) in args[1..].iter().enumerate() {
        let bytes = std::fs::read(p).unwrap_or_else(|e| {
            eprintln!("{p}: {e}");
            std::process::exit(1);
        });
        let sizes = pdfcraft_impose_web::page_sizes(bytes.clone()).unwrap_or_else(|e| {
            eprintln!("{p}: page_sizes: {e}");
            std::process::exit(1);
        });
        println!("{p}: {} page(s) {:?}", sizes.len(), sizes);
        for page in 0..sizes.len() {
            plan.push(serde_json::json!({ "src": i, "page": page, "rotate": 0 }));
        }
        files.push(bytes);
    }
    let plan: Vec<pdfcraft_impose_web::PlanPage> = serde_json::from_value(serde_json::Value::Array(plan)).unwrap_or_else(|e| {
        eprintln!("plan: {e}");
        std::process::exit(1);
    });
    let first = files.remove(0);
    match pdfcraft_impose_web::assemble(first, files, &plan) {
        Ok(out) => {
            std::fs::write(&args[0], out).unwrap_or_else(|e| {
                eprintln!("write: {e}");
                std::process::exit(1);
            });
            println!("ok: {} page(s) -> {}", plan.len(), args[0]);
        }
        Err(e) => {
            eprintln!("assemble: {e}");
            std::process::exit(1);
        }
    }
}
