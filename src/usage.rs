use crate::read::ClientId;

#[derive(Clone, Copy)]
pub enum Usage {
    /// specifies what percent of total capacity has been used
    UtilizationRatio(UsageUnit),
}
pub type UsageUnit = f32;

pub trait Capacity: Send + Sync + 'static {
    // ── Capacity service ──────────────────────────────────────────────────
    fn on_status(&self, tx: flume::Sender<Usage>);
}
