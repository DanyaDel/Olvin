//! Network statistics for the profiler panel and for tuning: round-trip
//! time, bandwidth, loss, tick lag.

use crate::net::Net;

/// A snapshot of how the connection is doing. Rates cover the last second or so.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetStats {
    /// Round-trip time to the server, seconds (client; 0 on a server).
    pub rtt: f32,
    /// Bytes per second sent and received by this process.
    pub bytes_out_per_sec: f32,
    pub bytes_in_per_sec: f32,
    /// Share of snapshots that never arrived, 0..1 (client; 0 on a server).
    pub loss: f32,
    /// Server ticks the client's view is behind the server's newest tick it knows.
    pub tick_lag: u32,
    pub clients: usize,
    pub replicated: usize,
    pub tick: u32,
    pub prediction: crate::predict::PredictionStats,
}

/// Rolling window for the byte rates.
#[derive(Default)]
pub(crate) struct RateWindow {
    start: f64,
    out0: u64,
    in0: u64,
    pub out: f32,
    pub inn: f32,
}

impl RateWindow {
    pub fn update(&mut self, now: f64, bytes_out: u64, bytes_in: u64) {
        let span = now - self.start;
        if span >= 1.0 {
            self.out = (bytes_out - self.out0) as f32 / span as f32;
            self.inn = (bytes_in - self.in0) as f32 / span as f32;
            self.start = now;
            self.out0 = bytes_out;
            self.in0 = bytes_in;
        }
    }
}

impl Net {
    /// Totals of bytes sent and received so far (all clients, for a server).
    fn byte_totals(&self) -> (u64, u64) {
        let (mut out, mut inn) = (0, 0);
        if let Some(s) = &self.server {
            out += s.bytes_out;
            inn += s.bytes_in;
        }
        if let Some(c) = &self.client {
            out += c.bytes_out;
            inn += c.bytes_in;
        }
        (out, inn)
    }

    pub(crate) fn update_stats(&mut self) {
        let (out, inn) = self.byte_totals();
        self.rates.update(self.now, out, inn);
    }

    /// Current statistics. Cheap; call it every frame.
    pub fn stats(&self) -> NetStats {
        let client = &self.repl.client;
        let loss = client.loss();
        let known = self.client.as_ref().map_or(0, |c| c.server_tick);
        NetStats {
            rtt: self.rtt(),
            bytes_out_per_sec: self.rates.out,
            bytes_in_per_sec: self.rates.inn,
            loss,
            tick_lag: known.saturating_sub(client.newest_tick()),
            clients: self.server.as_ref().map_or(0, |s| {
                s.clients.values().filter(|c| c.peer.is_some()).count()
            }),
            replicated: self.replicated_count(),
            tick: self.tick(),
            prediction: self.prediction_stats(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_follow_the_window() {
        let mut w = RateWindow::default();
        w.update(0.5, 100, 50);
        assert_eq!(w.out, 0.0);
        w.update(1.0, 1000, 500);
        assert_eq!((w.out, w.inn), (1000.0, 500.0));
        w.update(2.0, 1500, 500);
        assert_eq!((w.out, w.inn), (500.0, 0.0));
    }
}
