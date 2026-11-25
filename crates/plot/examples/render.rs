//! Renders a figure spec to SVG: `cargo run --release --example render -- spec.json [out.svg]`.
//!
//! With `--bench N` it renders the figure N times and prints the mean time per figure.

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (bench, args): (Option<usize>, Vec<String>) = match args.iter().position(|a| a == "--bench")
    {
        Some(i) => {
            let n = args.get(i + 1).and_then(|n| n.parse().ok()).unwrap_or(200);
            let rest = args
                .iter()
                .enumerate()
                .filter(|(k, _)| *k != i && *k != i + 1)
                .map(|(_, a)| a.clone())
                .collect();
            (Some(n), rest)
        }
        None => (None, args),
    };
    let Some(path) = args.first() else {
        eprintln!("usage: render SPEC.json [OUT.svg] [--bench N]");
        std::process::exit(2);
    };
    let json = std::fs::read_to_string(path).expect("read the spec");
    let figure = sbmlsim_plot::parse(&json).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    let svg = sbmlsim_plot::render_svg(&figure);
    if let Some(n) = bench {
        let start = Instant::now();
        for _ in 0..n {
            std::hint::black_box(sbmlsim_plot::render_svg(std::hint::black_box(&figure)));
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!(
            "{path}: {ms:.3} ms per figure over {n}, {} bytes",
            svg.len()
        );
    }
    match args.get(1) {
        Some(out) => std::fs::write(out, &svg).expect("write the svg"),
        None if bench.is_none() => println!("{svg}"),
        None => {}
    }
}
