use crate::bridge_db::{
    PendingWithdrawStats, Utxo, WithdrawTransactionInfo, UTXO_STATUS_PENDING_CHANGE,
    UTXO_STATUS_SPENT_PENDING,
};
use once_cell::sync::Lazy;
use prometheus::{
    register_gauge_vec, register_histogram_vec, register_int_counter_vec, register_int_gauge_vec,
    Encoder, GaugeVec, HistogramTimer, HistogramVec, IntCounterVec, IntGaugeVec, TextEncoder,
};
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub enum MetricFlow {
    DepositRelay,
    WithdrawBroadcast,
    WithdrawReconcile,
    HeaderRelay,
    Withdraw,
}

impl MetricFlow {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DepositRelay => "deposit_relay",
            Self::WithdrawBroadcast => "withdraw_broadcast",
            Self::WithdrawReconcile => "withdraw_reconcile",
            Self::HeaderRelay => "header_relay",
            Self::Withdraw => "withdraw",
        }
    }
}

#[derive(Clone, Copy)]
pub enum MetricEventStatus {
    Ok,
    Error,
    Rejected,
    Pending,
    Confirmed,
    Rebroadcast,
}

impl MetricEventStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Rejected => "rejected",
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Rebroadcast => "rebroadcast",
        }
    }
}

#[derive(Clone, Copy)]
pub enum MetricReason {
    None,
    Attempt,
    TxNotInBlock,
    AheadOfRelayTip,
    BelowRelayBuffer,
    TxMissingFromBlock,
    SendFailed,
    SubmitFailed,
    LoadPendingFailed,
    FinalizeDbFailed,
    AwaitingConfirmations,
    MissingRawTx,
    RebroadcastTxidMismatch,
    RebroadcastFailed,
    DepositUtxoInsertFailed,
    AmountTooSmall,
    LoadUtxosFailed,
    InsufficientUtxos,
    ChangeUtxoInsertFailed,
    BroadcastFailed,
}

impl MetricReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Attempt => "attempt",
            Self::TxNotInBlock => "tx_not_in_block",
            Self::AheadOfRelayTip => "ahead_of_relay_tip",
            Self::BelowRelayBuffer => "below_relay_buffer",
            Self::TxMissingFromBlock => "tx_missing_from_block",
            Self::SendFailed => "send_failed",
            Self::SubmitFailed => "submit_failed",
            Self::LoadPendingFailed => "load_pending_failed",
            Self::FinalizeDbFailed => "finalize_db_failed",
            Self::AwaitingConfirmations => "awaiting_confirmations",
            Self::MissingRawTx => "missing_raw_tx",
            Self::RebroadcastTxidMismatch => "rebroadcast_txid_mismatch",
            Self::RebroadcastFailed => "rebroadcast_failed",
            Self::DepositUtxoInsertFailed => "deposit_utxo_insert_failed",
            Self::AmountTooSmall => "amount_too_small",
            Self::LoadUtxosFailed => "load_utxos_failed",
            Self::InsufficientUtxos => "insufficient_utxos",
            Self::ChangeUtxoInsertFailed => "change_utxo_insert_failed",
            Self::BroadcastFailed => "broadcast_failed",
        }
    }
}

#[derive(Clone, Copy)]
pub enum MetricChain {
    BitcoinBest,
    YonaRelayTip,
}

impl MetricChain {
    const fn as_str(self) -> &'static str {
        match self {
            Self::BitcoinBest => "bitcoin_best",
            Self::YonaRelayTip => "yona_relay_tip",
        }
    }
}

#[derive(Clone, Copy)]
pub enum PendingMetricStatus {
    Broadcasted,
    SpentPending,
    PendingChange,
}

impl PendingMetricStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Broadcasted => "broadcasted",
            Self::SpentPending => "spent_pending",
            Self::PendingChange => "pending_change",
        }
    }
}

pub static BRIDGE_EVENTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "bridge_events_total",
        "Bridge state transitions and errors",
        &["flow", "status", "reason"]
    )
    .expect("bridge_events_total")
});

pub static BRIDGE_PENDING_ITEMS: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "bridge_pending_items",
        "Current number of pending bridge items by flow and status",
        &["flow", "status"]
    )
    .expect("bridge_pending_items")
});

pub static BRIDGE_CONFIRMATIONS_OBSERVED: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "bridge_confirmations_observed",
        "Observed bitcoin confirmations at bridge decision points",
        &["flow"],
        vec![0.0, 1.0, 2.0, 3.0, 6.0, 12.0, 24.0]
    )
    .expect("bridge_confirmations_observed")
});

pub static BRIDGE_BLOCK_HEIGHT: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "bridge_block_height",
        "Observed block heights for Bitcoin and Yona relay tip",
        &["chain"]
    )
    .expect("bridge_block_height")
});

pub static BRIDGE_RECONCILE_DURATION_SECONDS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "bridge_reconcile_duration_seconds",
        "Reconciler loop duration in seconds",
        &["flow"]
    )
    .expect("bridge_reconcile_duration_seconds")
});

pub static BRIDGE_OLDEST_PENDING_AGE_SECONDS: Lazy<GaugeVec> = Lazy::new(|| {
    register_gauge_vec!(
        "bridge_oldest_pending_age_seconds",
        "Age in seconds of the oldest pending bridge item",
        &["flow", "status"]
    )
    .expect("bridge_oldest_pending_age_seconds")
});

pub fn init_metrics() {
    Lazy::force(&BRIDGE_EVENTS_TOTAL);
    Lazy::force(&BRIDGE_PENDING_ITEMS);
    Lazy::force(&BRIDGE_CONFIRMATIONS_OBSERVED);
    Lazy::force(&BRIDGE_BLOCK_HEIGHT);
    Lazy::force(&BRIDGE_RECONCILE_DURATION_SECONDS);
    Lazy::force(&BRIDGE_OLDEST_PENDING_AGE_SECONDS);
}

pub fn inc_event(flow: MetricFlow, status: MetricEventStatus, reason: MetricReason) {
    BRIDGE_EVENTS_TOTAL
        .with_label_values(&[flow.as_str(), status.as_str(), reason.as_str()])
        .inc();
}

pub fn observe_confirmations(flow: MetricFlow, confirmations: u32) {
    BRIDGE_CONFIRMATIONS_OBSERVED
        .with_label_values(&[flow.as_str()])
        .observe(confirmations as f64);
}

pub fn set_block_height(chain: MetricChain, height: u32) {
    BRIDGE_BLOCK_HEIGHT
        .with_label_values(&[chain.as_str()])
        .set(height as i64);
}

pub fn start_reconcile_timer(flow: MetricFlow) -> HistogramTimer {
    BRIDGE_RECONCILE_DURATION_SECONDS
        .with_label_values(&[flow.as_str()])
        .start_timer()
}

pub async fn update_pending_metrics(pool: &SqlitePool) {
    let pending_withdrawals = WithdrawTransactionInfo::pending_stats(pool)
        .await
        .unwrap_or(PendingWithdrawStats {
            count: 0,
            oldest_created_at_unix: None,
        });
    let spent_pending = Utxo::count_by_status(pool, UTXO_STATUS_SPENT_PENDING)
        .await
        .unwrap_or(0);
    let pending_change = Utxo::count_by_status(pool, UTXO_STATUS_PENDING_CHANGE)
        .await
        .unwrap_or(0);

    BRIDGE_PENDING_ITEMS
        .with_label_values(&[
            MetricFlow::Withdraw.as_str(),
            PendingMetricStatus::Broadcasted.as_str(),
        ])
        .set(pending_withdrawals.count);
    BRIDGE_PENDING_ITEMS
        .with_label_values(&[
            MetricFlow::Withdraw.as_str(),
            PendingMetricStatus::SpentPending.as_str(),
        ])
        .set(spent_pending);
    BRIDGE_PENDING_ITEMS
        .with_label_values(&[
            MetricFlow::Withdraw.as_str(),
            PendingMetricStatus::PendingChange.as_str(),
        ])
        .set(pending_change);

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("unix time")
        .as_secs() as i64;
    let oldest_pending_age = pending_withdrawals
        .oldest_created_at_unix
        .map(|created_at| (now_unix - created_at).max(0) as f64)
        .unwrap_or(0.0);

    BRIDGE_OLDEST_PENDING_AGE_SECONDS
        .with_label_values(&[
            MetricFlow::Withdraw.as_str(),
            PendingMetricStatus::Broadcasted.as_str(),
        ])
        .set(oldest_pending_age);
}

pub fn render_metrics() -> Result<String, prometheus::Error> {
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    TextEncoder::new().encode(&metric_families, &mut buffer)?;
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}
