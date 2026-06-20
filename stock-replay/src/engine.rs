use crate::bar::DailyBar;
use std::thread;
use std::time::Duration;

pub struct ReplayEngine {
    bars: Vec<DailyBar>,
}

impl ReplayEngine {
    pub fn new(bars: Vec<DailyBar>) -> Self {
        ReplayEngine { bars }
    }

    pub fn run<F>(&self, mut on_bar: F)
    where
        F: FnMut(&DailyBar, usize),
    {
        for (i, bar) in self.bars.iter().enumerate() {
            on_bar(bar, i);
        }
    }

    pub fn run_with_speed<F>(&self, speed: f64, mut on_bar: F)
    where
        F: FnMut(&DailyBar, usize),
    {
        let delay = if speed <= 0.0 {
            Duration::ZERO
        } else {
            let secs = 1.0 / speed;
            Duration::from_secs_f64(secs)
        };

        for (i, bar) in self.bars.iter().enumerate() {
            on_bar(bar, i);
            if !delay.is_zero() && i < self.bars.len() - 1 {
                thread::sleep(delay);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.bars.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }
}
