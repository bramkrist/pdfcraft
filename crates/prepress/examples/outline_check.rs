//! `cargo run -p pdfcraft-prepress --example outline_check -- IN.pdf OUT.pdf [max_pages]`: outline
//! the text of IN, write OUT, and compare every page rendered before and after.
use std::sync::Arc;

use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RequestKind};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(input), Some(output)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: outline_check IN.pdf OUT.pdf [max_pages]");
        std::process::exit(2);
    };
    let max: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    let src = std::fs::read(input).unwrap_or_default();
    let t0 = std::time::Instant::now();
    let (out, report) = match pdfcraft_prepress::outline::outline_text(src.clone()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    println!("outlined {} blocks, {} glyphs, kept {} in {:?}", report.outlined, report.glyphs, report.kept.len(), t0.elapsed());
    let mut reasons = std::collections::BTreeMap::new();
    for k in &report.kept {
        *reasons.entry(k.reason.clone()).or_insert(0) += 1;
    }
    println!("kept reasons: {reasons:?}");
    let _ = std::fs::write(output, &out);
    let mut a = PageRenderer::new(Arc::new(src), RenderConfig::default());
    let mut b = PageRenderer::new(Arc::new(out), RenderConfig::default());
    let mut worst = (0.0_f64, 0);
    let mut compared = 0;
    for page in 0..max {
        let req = RenderRequest { page, kind: RequestKind::Pixels, tile: None, scale: 1.0, tag: 0, preview: false };
        let (ra, rb) = (a.render(req), b.render(req));
        if ra.error.is_some() || ra.rgba.is_empty() {
            println!("stop at page {}: {:?}", page + 1, ra.error);
            break;
        }
        compared += 1;
        let pa: Vec<&[u8]> = ra.rgba.chunks(4).collect();
        let pb: Vec<&[u8]> = rb.rgba.chunks(4).collect();
        let differ = pa.iter().zip(&pb).filter(|(x, y)| x.iter().zip(y.iter()).take(3).any(|(m, n)| m.abs_diff(*n) > 48)).count();
        let d = differ as f64 / pa.len().max(1) as f64;
        if d > worst.0 {
            worst = (d, page + 1);
        }
        if d > 0.002 {
            println!("page {} differs {:.3}%", page + 1, d * 100.0);
        }
    }
    println!("compared {compared} pages, worst page {} {:.4}%", worst.1, worst.0 * 100.0);
}
