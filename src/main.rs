mod commands;

fn main() {
    println!("{}", commands::hexdump(&commands::nozzle_check_job(commands::Timestamp::now())));
}
