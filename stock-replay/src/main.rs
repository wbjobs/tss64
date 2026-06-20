mod bar;
mod data;
mod engine;
mod strategy;

use bar::DailyBar;
use chrono::NaiveDate;
use clap::Parser;
use data::{CsvSource, DataSource};
use engine::ReplayEngine;
use strategy::{MaCrossStrategy, Strategy, print_comparison_report};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(name = "stock-replay", about = "Stock data replay & backtesting tool", version)]
struct Cli {
    #[arg(short, long, help = "Path to CSV file with daily bar data")]
    file: Option<PathBuf>,

    #[arg(long, help = "PostgreSQL connection string, e.g. \"host=localhost dbname=stocks user=postgres\"")]
    db: Option<String>,

    #[arg(long, default_value = "daily_bars", help = "PostgreSQL table name")]
    table: String,

    #[arg(long, default_value = "AAPL", help = "Stock symbol for PostgreSQL query")]
    symbol: String,

    #[arg(short, long, help = "Short MA window (single strategy mode)")]
    short: Option<usize>,

    #[arg(short, long, help = "Long MA window (single strategy mode)")]
    long: Option<usize>,

    #[arg(
        long,
        help = "Multiple strategy params, format: short1/long1,short2/long2,...  e.g. 5/20,10/30,15/40"
    )]
    strategies: Option<String>,

    #[arg(short, long, default_value_t = 1_000_000.0, help = "Initial capital")]
    capital: f64,

    #[arg(short = 'x', long, default_value_t = 0.0, help = "Replay speed multiplier (0=max speed, 1=1bar/s, 100=100bars/s)")]
    speed: f64,

    #[arg(long, default_value_t = false, help = "Generate sample CSV for testing")]
    generate_sample: bool,
}

fn parse_strategy_pairs(s: &str) -> Result<Vec<(usize, usize)>, String> {
    let mut pairs = Vec::new();
    for pair_str in s.split(',') {
        let pair_str = pair_str.trim();
        let parts: Vec<&str> = pair_str.split('/').collect();
        if parts.len() != 2 {
            return Err(format!("invalid pair '{}': expected short/long format", pair_str));
        }
        let short: usize = parts[0]
            .trim()
            .parse()
            .map_err(|_| format!("invalid short value in '{}'", pair_str))?;
        let long: usize = parts[1]
            .trim()
            .parse()
            .map_err(|_| format!("invalid long value in '{}'", pair_str))?;
        if short >= long {
            return Err(format!(
                "short ({}) must be less than long ({}) in pair '{}'",
                short, long, pair_str
            ));
        }
        pairs.push((short, long));
    }
    if pairs.is_empty() {
        return Err("no strategy pairs specified".to_string());
    }
    Ok(pairs)
}

fn load_data(cli: &Cli) -> Result<Vec<DailyBar>, Box<dyn std::error::Error>> {
    if let Some(ref path) = cli.file {
        let source = CsvSource::new(path.clone());
        return source.load();
    }

    if let Some(ref conn_str) = cli.db {
        #[cfg(feature = "postgres")]
        {
            let source = data::PgSource::new(
                conn_str.clone(),
                cli.table.clone(),
                cli.symbol.clone(),
            );
            return source.load();
        }
        #[cfg(not(feature = "postgres"))]
        {
            eprintln!("error: PostgreSQL support not compiled. Rebuild with --features postgres");
            std::process::exit(1);
        }
    }

    eprintln!("error: specify --file <csv_path> or --db <conn_str> to load data");
    std::process::exit(1);
}

fn generate_sample_csv(path: &PathBuf, years: usize) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::create(path)?;
    writeln!(file, "date,open,close,high,low,volume")?;

    let mut price = 100.0_f64;
    let start = NaiveDate::from_ymd_opt(2005, 1, 4).unwrap();
    let total_days = years * 252;

    let mut rng_state: u64 = 42;
    for i in 0..total_days {
        let date =
            start + chrono::Duration::days((i as i64 / 252) * 365 + (i as i64 % 252) * 7 / 5);
        let date_str = date.format("%Y-%m-%d").to_string();

        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1);
        let u1 = ((rng_state >> 33) as f64) / (2u64.pow(31) as f64);
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1);
        let u2 = ((rng_state >> 33) as f64) / (2u64.pow(31) as f64);
        let norm = (u1.max(1e-10).ln() * -2.0).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();

        let daily_return = 0.0003 + 0.02 * norm;
        price *= 1.0 + daily_return;
        price = price.max(5.0);

        let spread = 0.003 * price * norm.abs();
        let high = price + spread;
        let low = price - spread;
        let open = (high + low) / 2.0 + spread * 0.3 * norm.signum();

        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1);
        let vol_rand = ((rng_state >> 33) as f64) / (2u64.pow(31) as f64);
        let volume = (1_000_000.0 + 500_000.0 * vol_rand) as u64;

        writeln!(
            file,
            "{},{:.2},{:.2},{:.2},{:.2},{}",
            date_str, open, price, high, low, volume
        )?;
    }

    println!(
        "generated sample CSV: {} ({} trading days, {} years)",
        path.display(),
        total_days,
        years
    );
    Ok(())
}

fn run_strategies(
    engine: &ReplayEngine,
    pairs: &[(usize, usize)],
    capital: f64,
    speed: f64,
) -> Vec<strategy::StrategyMetrics> {
    let mut all_metrics = Vec::with_capacity(pairs.len());

    for &(short, long) in pairs {
        let mut strat = MaCrossStrategy::new(short, long, capital);
        let label = strat.name();

        println!(
            "\n--- Running {} on {} bars (speed={}x) ---",
            label,
            engine.len(),
            if speed <= 0.0 {
                "max".to_string()
            } else {
                format!("{:.0}", speed)
            }
        );

        let start = Instant::now();

        if speed <= 0.0 {
            engine.run(|bar, idx| {
                let signal = strat.on_bar(bar, idx);
                if signal != strategy::Signal::Hold {
                    println!(
                        "[{}] {} - {:?} @ close={:.2}",
                        idx, bar.date, signal, bar.close
                    );
                }
            });
        } else {
            engine.run_with_speed(speed, |bar, idx| {
                let signal = strat.on_bar(bar, idx);
                if signal != strategy::Signal::Hold {
                    println!(
                        "[{}] {} - {:?} @ close={:.2}",
                        idx, bar.date, signal, bar.close
                    );
                }
            });
        }

        let elapsed = start.elapsed();
        println!(
            "{} completed in {:.3}ms",
            label,
            elapsed.as_secs_f64() * 1000.0
        );

        all_metrics.push(strat.metrics());
    }

    all_metrics
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if cli.generate_sample {
        let path = cli.file.as_ref().ok_or_else(|| {
            "error: --file <path> required with --generate-sample"
                .to_string()
        })?;
        generate_sample_csv(path, 20)?;
        return Ok(());
    }

    let pairs = resolve_strategy_pairs(&cli)?;

    println!("Loading data...");
    let load_start = Instant::now();
    let bars = load_data(&cli)?;
    let load_time = load_start.elapsed();
    println!("Data loaded in {:.3}ms", load_time.as_secs_f64() * 1000.0);

    if bars.is_empty() {
        eprintln!("error: no valid bar data loaded");
        std::process::exit(1);
    }

    let engine = ReplayEngine::new(bars);

    let all_metrics = run_strategies(&engine, &pairs, cli.capital, cli.speed);

    if pairs.len() == 1 {
        println!("\n{}", all_metrics[0]);
    } else {
        for m in &all_metrics {
            println!("{}", m);
        }
        print_comparison_report(&all_metrics);
    }

    Ok(())
}

fn resolve_strategy_pairs(cli: &Cli) -> Result<Vec<(usize, usize)>, String> {
    if let Some(ref s) = cli.strategies {
        return parse_strategy_pairs(s);
    }

    match (cli.short, cli.long) {
        (Some(short), Some(long)) => {
            if short >= long {
                return Err(format!(
                    "short window ({}) must be less than long window ({})",
                    short, long
                ));
            }
            Ok(vec![(short, long)])
        }
        (None, None) => Ok(vec![(5, 20)]),
        _ => Err("specify both --short and --long, or use --strategies short1/long1,short2/long2".to_string()),
    }
}
