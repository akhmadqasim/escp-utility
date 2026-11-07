//! Prints one decoded status reply from the queue's printer (read-only: the
//! only data sent is the `st 01 00 01` request). Refuses while the CUPS
//! queue has jobs.
//!
//! cargo run --example status [-- QUEUE]

#[path = "../src/status/mod.rs"]
#[allow(dead_code)] // the example uses only the query and the message tables
mod status;

#[path = "../src/printer.rs"]
#[allow(dead_code)]
mod printer;

fn main() {
    let queue = std::env::args().nth(1).unwrap_or_else(|| "L800".into());
    let p = printer::Identity::of(&queue);
    print!("{}", p.summary());
    let started = std::time::Instant::now();
    match status::query(&queue, p.status_node().as_deref()) {
        Ok(s) => {
            let (title, text) = status::message(&s);
            println!("query took {:?}", started.elapsed());
            println!("ST={:02X?} ER={:02X?} TP={} WR={:02X?} TC={:?}", s.st, s.er, s.tp, s.warnings, s.cleaning_secs);
            for i in &s.inks {
                println!("  {:<13} {}", status::ink_name(i), i.percent().map_or("?".into(), |p| format!("{p}%")));
            }
            println!("Status Monitor (L800 tables): {title}: {text}");
            let (title, text) = status::message_generic(&s);
            println!("Status Monitor (generic): {title}: {text}");
            println!("Pre-check (nozzle): {:?}", status::precheck(&s, false));
            println!("Pre-check (cleaning): {:?}", status::precheck(&s, true));
            println!("Generic pre-check (nozzle, cleaning): {:?}, {:?}", status::precheck_generic(&s, false), status::precheck_generic(&s, true));
        }
        Err(e) => {
            eprintln!("no status: {e:?}");
            std::process::exit(1);
        }
    }
}
