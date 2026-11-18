use ats2_rs::sweep::demo::{POWER_ASSUMPTIONS, simulate_demo};

fn main() {
    let result = match simulate_demo(Some(8.0), true) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("simulation failed: {error}");
            std::process::exit(1);
        }
    };

    println!("{}", result.provenance);
    println!("Load resistance: {:?} ohms", result.load_resistance_ohms);
    if result.power_assumptions == Some(POWER_ASSUMPTIONS) {
        println!("Power assumption: {}", POWER_ASSUMPTIONS);
    }
    println!("generator_Vrms,measured_Vrms,THD+N_percent,THD_percent,power_W");
    for point in &result.points {
        println!(
            "{:.6},{:.6},{:.6},{},{}",
            point.generator_level_vrms,
            point.measured_vrms,
            point.thd_n_percent,
            point
                .thd_percent
                .map_or_else(|| "n/a".to_owned(), |value| format!("{value:.6}")),
            result
                .measured_power_w(point.record_index)
                .map_or_else(|| "n/a".to_owned(), |value| format!("{value:.9}")),
        );
    }
}
