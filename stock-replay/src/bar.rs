use serde::Deserialize;

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
        .parse::<f64>()
        .map_err(serde::de::Error::custom)?;
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
    pub fn is_valid(&self) -> bool {
        !self.date.is_empty()
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

    pub fn new(
        date: String,
        open: f64,
        close: f64,
        high: f64,
        low: f64,
        volume: f64,
    ) -> Self {
        DailyBar {
            date,
            open,
            close,
            high,
            low,
            volume,
        }
    }
}
