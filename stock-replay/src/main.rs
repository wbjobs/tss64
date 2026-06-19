use chrono::NaiveDate;
use clap::Parser;
use csv::ReaderBuilder;
use serde::Deserialize;
use std::collections::VecDeque;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Instant;

fn parse_flex_f64<'de, D>(de: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(de)?;
    let trimmed = s.trim().trim_matches('"').replace(',', "");
    if trimmed.is_empty() {
        return Err(serde::de::Error::custom("empty numeric field"));
    }
    let value: f64 = trimmed
        .parse::<f64>().map_err(serde::de::Error::custom)?;
    if !value.is_finite() {
        return Err(serde::de::Error::custom("non-finite numeric value"));
    }
    Ok(value)
}

#[derive(Debug, Deserialize, Clone)]
pub struct DailyBar {
    pub date: String,
    #[serde(deserialize_with = "parse_flex_f64")]
    pub open: f64,
    #[serde(deserialize_with = "parse_flex_f64")]
    pub close: f64,
    #[serde(deserialize_with = "parse_flex_f64")]
    pub high: f64,
    #[serde(deserialize_with = "parse_flex_f64")]
    pub low: f64,
    #[serde(deserialize_with = "parse_flex_f64")]
    pub volume: f64,
}

impl DailyBar {
    fn is_valid(&self) -> bool {
        self.date.len() > 0
            && self.open.is_finite()
            && self.close.is_finite()
            && self.high.is_finite()
            && self.low.is_finite()
            && self.volume.is_finite()
            && self.open > 0.0
            && self.close > 0.0
            && self.high > 0.0
            && self.low > 0.0
    }
}

pub struct ReplayEngine {
    bars: Vec<DailyBar>,
}

impl ReplayEngine {
    pub fn from_csv(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let buf_reader = BufReader::with_capacity(8 * 1024 * 1024, file);
        let mut rdr = ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .buffer_capacity(8 * 1024 * 1024)
            .from_reader(buf_reader);

        let mut bars: Vec<DailyBar> = Vec::with_capacity(5000);
        let mut skipped = 0usize;
        for (row_num, result) in rdr.deserialize().enumerate() {
            match result {
                Ok(bar) => {
                    let bar: DailyBar = bar;
                    if bar.is_valid() {
                        bars.push(bar);
                    } else {
                        skipped += 1;
                        eprintln!("warning: row {} skipped: invalid bar data (date={})", row_num + 2, bar.date);
                    }
                }
                Err(e) => {
                    skipped += 1;
                    eprintln!("warning: row {} skipped: parse error: {}", row_num + 2, e);
                }
            }
        }
        bars.sort_by(|a, b| a.date.cmp(&b.date));
        bars.dedup_by(|a, b| a.date == b.date);
        println!(
            "loaded {} bars from {} (skipped {} invalid rows)",
            bars.len(),
            path.display(),
            skipped
        );
        Ok(ReplayEngine { bars })
    }

    pub fn run<F>(&self, mut on_bar: F)
    where
        F: FnMut(&DailyBar, usize),
    {
        for (i, bar) in self.bars.iter().enumerate() {
            on_bar(bar, i);
        }
    }

    pub fn len(&self) -> usize {
        self.bars.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Signal {
    Hold,
    Buy,
    Sell,
}

pub struct MaCrossStrategy {
    short_window: usize,
    long_window: usize,
    short_queue: VecDeque<f64>,
    long_queue: VecDeque<f64>,
    short_sum: f64,
    long_sum: f64,
    prev_short_ma: Option<f64>,
    prev_long_ma: Option<f64>,
    position: f64,
    cash: f64,
    initial_capital: f64,
    trade_entry_price: f64,
    trades: Vec<TradeRecord>,
    equity_curve: Vec<f64>,
    running_peak: f64,
    max_drawdown: f64,
}

#[derive(Debug)]
#[allow(dead_code)]
struct TradeRecord {
    entry_price: f64,
    exit_price: f64,
    pnl: f64,
}

impl MaCrossStrategy {
    pub fn new(short_window: usize, long_window: usize, initial_capital: f64) -> Self {
        MaCrossStrategy {
            short_window,
            long_window,
            short_queue: VecDeque::with_capacity(short_window),
            long_queue: VecDeque::with_capacity(long_window),
            short_sum: 0.0,
            long_sum: 0.0,
            prev_short_ma: None,
            prev_long_ma: None,
            position: 0.0,
            cash: initial_capital,
            initial_capital,
            trade_entry_price: 0.0,
            trades: Vec::new(),
            equity_curve: Vec::new(),
            running_peak: initial_capital,
            max_drawdown: 0.0,
        }
    }

    fn update_queue(queue: &mut VecDeque<f64>, sum: &mut f64, value: f64, window: usize) {
        queue.push_back(value);
        *sum += value;
        if queue.len() > window {
            if let Some(old) = queue.pop_front() {
                *sum -= old;
            }
        }
    }

    pub fn on_bar(&mut self, bar: &DailyBar, _idx: usize) -> Signal {
        let close = bar.close;

        Self::update_queue(&mut self.short_queue, &mut self.short_sum, close, self.short_window);
        Self::update_queue(&mut self.long_queue, &mut self.long_sum, close, self.long_window);

        let mut signal = Signal::Hold;

        if self.short_queue.len() >= self.short_window && self.long_queue.len() >= self.long_window
        {
            let short_ma = self.short_sum / self.short_queue.len() as f64;
            let long_ma = self.long_sum / self.long_queue.len() as f64;

            if let (Some(prev_s), Some(prev_l)) = (self.prev_short_ma, self.prev_long_ma) {
                if prev_s < prev_l && short_ma > long_ma {
                    signal = Signal::Buy;
                } else if prev_s > prev_l && short_ma < long_ma {
                    signal = Signal::Sell;
                }
            }

            self.prev_short_ma = Some(short_ma);
            self.prev_long_ma = Some(long_ma);
        }

        self.execute_signal(signal, close);

        let equity_high = self.cash + self.position * bar.high;
        let equity_low = self.cash + self.position * bar.low;
        let equity_close = self.cash + self.position * close;

        if equity_high.is_finite() && equity_high > self.running_peak {
            self.running_peak = equity_high;
        }

        if self.running_peak > 0.0 && equity_low.is_finite() {
            let dd = (self.running_peak - equity_low) / self.running_peak;
            if dd > self.max_drawdown {
                self.max_drawdown = dd;
            }
        }

        if equity_close.is_finite() {
            self.equity_curve.push(equity_close);
        }

        signal
    }

    fn execute_signal(&mut self, signal: Signal, price: f64) {
        match signal {
            Signal::Buy if self.position == 0.0 => {
                let shares = (self.cash / price).floor();
                if shares > 0.0 {
                    self.position = shares;
                    self.cash -= shares * price;
                    self.trade_entry_price = price;
                }
            }
            Signal::Sell if self.position > 0.0 => {
                let pnl = (price - self.trade_entry_price) * self.position;
                self.trades.push(TradeRecord {
                    entry_price: self.trade_entry_price,
                    exit_price: price,
                    pnl,
                });
                self.cash += self.position * price;
                self.position = 0.0;
            }
            _ => {}
        }
    }

    pub fn metrics(&self) -> StrategyMetrics {
        let final_equity = *self.equity_curve.last().unwrap_or(&self.initial_capital);
        let total_return = (final_equity - self.initial_capital) / self.initial_capital;

        let total_trades = self.trades.len();
        let winning_trades = self.trades.iter().filter(|t| t.pnl > 0.0).count();
        let win_rate = if total_trades > 0 {
            winning_trades as f64 / total_trades as f64
        } else {
            0.0
        };

        StrategyMetrics {
            initial_capital: self.initial_capital,
            final_equity,
            total_return,
            max_drawdown: self.max_drawdown,
            total_trades,
            winning_trades,
            win_rate,
        }
    }
}

#[allow(dead_code)]
fn compute_max_drawdown(equity_curve: &[f64]) -> f64 {
    let valid: Vec<f64> = equity_curve
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    if valid.is_empty() {
        return 0.0;
    }
    let mut peak = valid[0];
    let mut max_dd = 0.0;
    for &equity in &valid {
        if equity > peak {
            peak = equity;
        }
        let dd = (peak - equity) / peak;
        if dd > max_dd {
            max_dd = dd;
        }
    }
    max_dd
}

#[derive(Debug)]
pub struct StrategyMetrics {
    pub initial_capital: f64,
    pub final_equity: f64,
    pub total_return: f64,
    pub max_drawdown: f64,
    pub total_trades: usize,
    pub winning_trades: usize,
    pub win_rate: f64,
}

impl std::fmt::Display for StrategyMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "========== Strategy Report ==========")?;
        writeln!(f, "Initial Capital : {:>14.2}", self.initial_capital)?;
        writeln!(f, "Final Equity    : {:>14.2}", self.final_equity)?;
        writeln!(
            f,
            "Total Return    : {:>13.2}%",
            self.total_return * 100.0
        )?;
        writeln!(
            f,
            "Max Drawdown    : {:>13.2}%",
            self.max_drawdown * 100.0
        )?;
        writeln!(f, "Total Trades    : {:>14}", self.total_trades)?;
        writeln!(f, "Winning Trades  : {:>14}", self.winning_trades)?;
        writeln!(f, "Win Rate        : {:>13.2}%", self.win_rate * 100.0)?;
        writeln!(f, "=====================================")
    }
}

#[derive(Parser, Debug)]
#[command(name = "stock-replay", about = "Stock data replay & backtesting tool")]
struct Cli {
    #[arg(short, long, help = "Path to the CSV file with daily bar data")]
    file: PathBuf,

    #[arg(short, long, default_value_t = 5, help = "Short MA window (days)")]
    short: usize,

    #[arg(short, long, default_value_t = 20, help = "Long MA window (days)")]
    long: usize,

    #[arg(short, long, default_value_t = 1_000_000.0, help = "Initial capital")]
    capital: f64,

    #[arg(long, default_value_t = false, help = "Generate sample CSV for testing")]
    generate_sample: bool,
}

fn generate_sample_csv(path: &PathBuf, years: usize) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    let mut file = File::create(path)?;
    writeln!(file, "date,open,close,high,low,volume")?;

    let mut price = 100.0_f64;
    let start = NaiveDate::from_ymd_opt(2005, 1, 4).unwrap();
    let total_days = years * 252;

    let mut rng_state: u64 = 42;
    for i in 0..total_days {
        let date = start + chrono::Duration::days((i as i64 / 252) * 365 + (i as i64 % 252) * 7 / 5);
        let date_str = date.format("%Y-%m-%d").to_string();

        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let u1 = ((rng_state >> 33) as f64) / (2u64.pow(31) as f64);
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let u2 = ((rng_state >> 33) as f64) / (2u64.pow(31) as f64);
        let norm = (u1.max(1e-10).ln() * -2.0).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();

        let daily_return = 0.0003 + 0.02 * norm;
        price *= 1.0 + daily_return;
        price = price.max(5.0);

        let spread = 0.003 * price * norm.abs();
        let high = price + spread;
        let low = price - spread;
        let open = (high + low) / 2.0 + spread * 0.3 * norm.signum();

        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if cli.generate_sample {
        generate_sample_csv(&cli.file, 20)?;
        return Ok(());
    }

    if cli.short >= cli.long {
        eprintln!("error: short window ({}) must be less than long window ({})", cli.short, cli.long);
        std::process::exit(1);
    }

    println!("Loading CSV data...");
    let load_start = Instant::now();
    let engine = ReplayEngine::from_csv(&cli.file)?;
    let load_time = load_start.elapsed();
    println!("CSV loaded in {:.3}ms", load_time.as_secs_f64() * 1000.0);

    let mut strategy = MaCrossStrategy::new(cli.short, cli.long, cli.capital);

    println!(
        "Replaying {} bars with MA({}/{}) strategy...",
        engine.len(),
        cli.short,
        cli.long
    );
    let replay_start = Instant::now();

    engine.run(|bar, idx| {
        let signal = strategy.on_bar(bar, idx);
        if signal != Signal::Hold {
            println!(
                "[{}] {} - {:?} @ close={:.2}",
                idx, bar.date, signal, bar.close
            );
        }
    });

    let replay_time = replay_start.elapsed();
    println!(
        "Replay completed in {:.3}ms",
        replay_time.as_secs_f64() * 1000.0
    );
    println!(
        "Total time: {:.3}ms",
        (load_time + replay_time).as_secs_f64() * 1000.0
    );

    let metrics = strategy.metrics();
    println!("\n{}", metrics);

    Ok(())
}
