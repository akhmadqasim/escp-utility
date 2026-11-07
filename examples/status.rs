//! Prints one decoded status reply from the L800 (read-only: the only data
//! sent is the `st 01 00 01` request). Refuses while the CUPS queue has jobs.
//!
//! cargo run --example status [-- QUEUE]

#[path = "../src/status/mod.rs"]
#[allow(dead_code)] // the example uses only the query and the message tables
mod status;

fn main() {
    let queue = std::env::args().nth(1).unwrap_or_else(|| "L800".into());
    let started = std::time::Instant::now();
    match status::query(&queue) {
        Ok(s) => {
            let (title, text) = status::message(&s);
            println!("query took {:?}", started.elapsed());
            println!("ST={:02X?} ER={:02X?} TP={} WR={:02X?} TC={:?}", s.st, s.er, s.tp, s.warnings, s.cleaning_secs);
            for i in &s.inks {
                println!("  {:<13} {}", status::ink_name(i.color), i.percent().map_or("?".into(), |p| format!("{p}%")));
            }
            println!("Status Monitor: {title}: {text}");
            println!("Pre-check (nozzle): {:?}", status::precheck(&s, false));
            println!("Pre-check (cleaning): {:?}", status::precheck(&s, true));
        }
        Err(e) => {
            eprintln!("no status: {e:?}");
            std::process::exit(1);
        }
    }
}
