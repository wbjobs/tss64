use crate::bar::DailyBar;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Signal {
    Hold,
    Buy,
    Sell,
}

pub trait Strategy {
    fn name(&self) -> String;
    fn on_bar(&mut self, bar: &DailyBar, idx: usize) -> Signal;
    fn metrics(&self) -> StrategyMetrics;
}

#[derive(Debug, Clone)]
pub struct MaCrossStrategy {
    pub short_window: usize,
    pub long_window: usize,
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

#[derive(Debug, Clone)]
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

    fn update_drawdown(&mut self, bar: &DailyBar) {
        let equity_high = self.cash + self.position * bar.high;
        let equity_low = self.cash + self.position * bar.low;
        let equity_close = self.cash + self.position * bar.close;

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
    }
}

impl Strategy for MaCrossStrategy {
    fn name(&self) -> String {
        format!("MA({}/{})", self.short_window, self.long_window)
    }

    fn on_bar(&mut self, bar: &DailyBar, _idx: usize) -> Signal {
        let close = bar.close;

        Self::update_queue(
            &mut self.short_queue,
            &mut self.short_sum,
            close,
            self.short_window,
        );
        Self::update_queue(
            &mut self.long_queue,
            &mut self.long_sum,
            close,
            self.long_window,
        );

        let mut signal = Signal::Hold;

        if self.short_queue.len() >= self.short_window
            && self.long_queue.len() >= self.long_window
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
        self.update_drawdown(bar);

        signal
    }

    fn metrics(&self) -> StrategyMetrics {
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
            name: self.name(),
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

#[derive(Debug, Clone)]
pub struct StrategyMetrics {
    pub name: String,
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
        writeln!(f, "========== {} ==========", self.name)?;
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
        writeln!(f, "{}", "=".repeat(20 + self.name.len() + 3))
    }
}

pub fn print_comparison_report(all_metrics: &[StrategyMetrics]) {
    if all_metrics.is_empty() {
        println!("No strategy results to compare.");
        return;
    }

    let col_widths = [16, 14, 14, 14, 10, 10, 10];
    let headers = [
        "Strategy",
        "Final Equity",
        "Return%",
        "MaxDD%",
        "Trades",
        "Wins",
        "WinRate%",
    ];

    let separator: String = col_widths.iter().map(|&w| "-".repeat(w + 2)).collect::<Vec<_>>().join("+");

    let best_return_idx = all_metrics
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_return.partial_cmp(&b.total_return).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(0);

    let lowest_dd_idx = all_metrics
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.max_drawdown.partial_cmp(&b.max_drawdown).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(0);

    println!("\n{}", separator);
    for (i, h) in headers.iter().enumerate() {
        print!(" {:>width$} |", h, width = col_widths[i]);
    }
    println!("\n{}", separator);

    for (i, m) in all_metrics.iter().enumerate() {
        let return_star = if i == best_return_idx && all_metrics.len() > 1 { "*" } else { "" };
        let dd_star = if i == lowest_dd_idx && all_metrics.len() > 1 && lowest_dd_idx != best_return_idx { "#" } else { "" };

        println!(
            " {:>width1$} | {:>width2$.2} | {:>width3$.2}{} | {:>width4$.2}{} | {:>width5$} | {:>width6$} | {:>width7$.2} |",
            m.name,
            m.final_equity,
            m.total_return * 100.0,
            return_star,
            m.max_drawdown * 100.0,
            dd_star,
            m.total_trades,
            m.winning_trades,
            m.win_rate * 100.0,
            width1 = col_widths[0],
            width2 = col_widths[1],
            width3 = col_widths[2],
            width4 = col_widths[3],
            width5 = col_widths[4],
            width6 = col_widths[5],
            width7 = col_widths[6],
        );
    }

    println!("{}", separator);

    if all_metrics.len() > 1 {
        println!("  * = highest return   # = lowest max drawdown");
    }
    println!();
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
