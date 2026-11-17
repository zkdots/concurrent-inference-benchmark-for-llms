#[derive(Default)]
pub(crate) struct Report {
    pub(crate) latency: Vec<f64>,
    pub(crate) tokens: u64,
    pub(crate) failures: usize,
    pub(crate) cancelled: usize,
    pub(crate) wall: f64,
}
impl Report {
    pub(crate) fn throughput(&self) -> f64 {
        self.tokens as f64 / self.wall.max(f64::EPSILON)
    }
    pub(crate) fn print(&mut self, clients: usize) {
        self.latency.sort_by(f64::total_cmp);
        println!(
            "\nclients={clients} completed={} errors={} cancelled={} wall={:.3}s aggregate={:.2} tokens/s requests/s={:.2}",
            self.latency.len(),
            self.failures,
            self.cancelled,
            self.wall,
            self.throughput(),
            self.latency.len() as f64 / self.wall.max(f64::EPSILON)
        );
        if !self.latency.is_empty() {
            let p95 = self.latency[(self.latency.len() * 95).div_ceil(100) - 1];
            println!(
                "latency mean={:.3}s p50={:.3}s p95={p95:.3}s",
                self.latency.iter().sum::<f64>() / self.latency.len() as f64,
                self.latency[(self.latency.len() - 1) / 2]
            );
        }
    }
}
