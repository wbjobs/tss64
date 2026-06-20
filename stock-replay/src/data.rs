use crate::bar::DailyBar;
use csv::ReaderBuilder;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

pub trait DataSource {
    fn load(&self) -> Result<Vec<DailyBar>, Box<dyn std::error::Error>>;
}

pub struct CsvSource {
    path: PathBuf,
}

impl CsvSource {
    pub fn new(path: PathBuf) -> Self {
        CsvSource { path }
    }
}

impl DataSource for CsvSource {
    fn load(&self) -> Result<Vec<DailyBar>, Box<dyn std::error::Error>> {
        let file = File::open(&self.path)?;
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
                        eprintln!(
                            "warning: row {} skipped: invalid bar data (date={})",
                            row_num + 2,
                            bar.date
                        );
                    }
                }
                Err(e) => {
                    skipped += 1;
                    eprintln!(
                        "warning: row {} skipped: parse error: {}",
                        row_num + 2,
                        e
                    );
                }
            }
        }
        bars.sort_by(|a, b| a.date.cmp(&b.date));
        bars.dedup_by(|a, b| a.date == b.date);
        println!(
            "loaded {} bars from CSV {} (skipped {} invalid rows)",
            bars.len(),
            self.path.display(),
            skipped
        );
        Ok(bars)
    }
}

#[cfg(feature = "postgres")]
pub struct PgSource {
    conn_str: String,
    table: String,
    symbol: String,
}

#[cfg(feature = "postgres")]
impl PgSource {
    pub fn new(conn_str: String, table: String, symbol: String) -> Self {
        PgSource {
            conn_str,
            table,
            symbol,
        }
    }
}

#[cfg(feature = "postgres")]
impl DataSource for PgSource {
    fn load(&self) -> Result<Vec<DailyBar>, Box<dyn std::error::Error>> {
        use postgres::Client;

        let mut client = Client::connect(&self.conn_str, postgres::NoTls)?;

        let query = format!(
            "SELECT date::text, open, close, high, low, volume FROM {} WHERE symbol = $1 ORDER BY date",
            self.table
        );

        let rows = client.query(query.as_str(), &[&self.symbol])?;

        let mut bars: Vec<DailyBar> = Vec::with_capacity(rows.len());
        let mut skipped = 0usize;

        for row in rows {
            let date_val: String = row.get(0);
            let open: f64 = row.get(1);
            let close: f64 = row.get(2);
            let high: f64 = row.get(3);
            let low: f64 = row.get(4);
            let volume: f64 = row.get(5);

            let bar = DailyBar::new(
                date_val,
                open,
                close,
                high,
                low,
                volume,
            );

            if bar.is_valid() {
                bars.push(bar);
            } else {
                skipped += 1;
            }
        }

        println!(
            "loaded {} bars from PostgreSQL [{}/{}] (skipped {} invalid rows)",
            bars.len(),
            self.table,
            self.symbol,
            skipped
        );
        Ok(bars)
    }
}
