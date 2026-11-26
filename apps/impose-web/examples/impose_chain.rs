//! `cargo run -p pdfcraft-impose-web --example impose_chain -- IN.pdf OUT.pdf JOB1.json [JOB2.json ...]`:
//! imposes IN with JOB1, then that result with JOB2, and so on (the "2 buku per lembar" chain of the
//! web app is booklet then step & repeat). Prints the page sizes after every step.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: impose_chain IN.pdf OUT.pdf JOB1.json [JOB2.json ...]");
        std::process::exit(2);
    }
    let mut bytes = std::fs::read(&args[0]).unwrap_or_else(|e| {
        eprintln!("{}: {e}", args[0]);
        std::process::exit(1);
    });
    for path in &args[2..] {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        });
        let job = serde_json::from_str::<pdfcraft_impose_web::Job>(&text).map_err(|e| e.to_string()).unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        });
        bytes = pdfcraft_impose_web::impose(bytes, &job).unwrap_or_else(|e| {
            eprintln!("{path}: impose: {e}");
            std::process::exit(1);
        });
        match pdfcraft_impose_web::page_sizes(bytes.clone()) {
            Ok(s) => println!("{path}: {} page(s) {:?}", s.len(), s),
            Err(e) => {
                eprintln!("{path}: page_sizes: {e}");
                std::process::exit(1);
            }
        }
    }
    std::fs::write(&args[1], bytes).unwrap_or_else(|e| {
        eprintln!("write: {e}");
        std::process::exit(1);
    });
}
