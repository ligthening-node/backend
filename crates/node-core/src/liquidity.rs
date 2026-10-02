//! Says in plain words why a payment cannot be sent over the node's channels. ldk-node itself only
//! answers "Failed to send the given payment.", which hides the three common causes: no usable
//! channel, a balance that is only the reserve, and a payment as large as the channel itself.

use crate::views::ChannelView;

/// Whole sats with thousands separators, rounded down from msat: 1234000 -> "1,234".
fn sat(msat: u64) -> String {
    let digits = (msat / 1000).to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    return out;
}

fn sat_value(value: u64) -> String {
    return sat(value * 1000);
}

/// None when the best channel can carry `amount_msat`, otherwise one sentence on why not and what
/// to do about it.
pub fn explain(amount_msat: u64, channels: &[ChannelView]) -> Option<String> {
    if channels.is_empty() {
        return Some("You have no channels yet. Open one on the Channels page before paying.".to_string());
    }
    let best = channels
        .iter()
        .filter(|c| c.is_usable)
        .max_by_key(|c| c.max_send_msat);
    let Some(best) = best else {
        return Some(
            "None of your channels is usable right now: it is still confirming or the peer is offline. Check the Channels page."
                .to_string(),
        );
    };

    // A payment below the channel size may go through, one that reaches it never can.
    let largest = channels
        .iter()
        .filter(|c| c.is_usable)
        .map(|c| c.capacity_sat)
        .max()
        .unwrap_or(0);
    if amount_msat >= largest * 1000 {
        return Some(format!(
            "A {} sat channel cannot carry a payment of {} sat or more. Pay less than the channel amount, or open a bigger channel first.",
            sat_value(largest),
            sat_value(largest)
        ));
    }

    if best.max_send_msat == 0 {
        return Some(match (best.our_balance_sat, best.our_reserve_sat) {
            (Some(balance), Some(reserve)) if balance <= reserve => format!(
                "You cannot send yet: your side of the channel holds {} sat and all of it is the {} sat reserve, which cannot be spent. Receive a payment first, or open a channel from this node.",
                sat_value(balance),
                sat_value(reserve)
            ),
            _ => "Nothing can be sent over your channels right now. Your balance may be only the reserve, which cannot be spent. Receive a payment first, or open a channel from this node.".to_string(),
        });
    }

    if amount_msat > best.outbound_msat {
        let reserve = best
            .our_reserve_sat
            .map(|r| format!(" ({} sat of your balance is the reserve, which cannot be spent)", sat_value(r)))
            .unwrap_or_default();
        return Some(format!(
            "That is more than you can send. You can spend {} sat in your channel{}.",
            sat(best.outbound_msat),
            reserve
        ));
    }

    if amount_msat > best.max_send_msat {
        return Some(format!(
            "Too large for one payment. Your channel carries at most {} sat per payment right now. Pay {} sat or less, or open a larger channel.",
            sat(best.max_send_msat),
            sat(best.max_send_msat)
        ));
    }
    return None;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> ChannelView {
        return ChannelView {
            channel_id: "aa".into(),
            user_channel_id: "1".into(),
            counterparty_node_id: "02".into(),
            funding_txo: None,
            short_channel_id: None,
            capacity_sat: 10_000,
            outbound_msat: 7_340_000,
            inbound_msat: 0,
            max_send_msat: 1_000_000,
            is_outbound: true,
            is_channel_ready: true,
            is_usable: true,
            confirmations: Some(6),
            confirmations_required: Some(6),
            our_balance_sat: Some(8_056),
            our_reserve_sat: Some(1_000),
            their_reserve_sat: 1_000,
        };
    }

    #[test]
    fn formats_sats_with_separators() {
        assert_eq!(sat(0), "0");
        assert_eq!(sat(999_999), "999");
        assert_eq!(sat(1_234_000), "1,234");
        assert_eq!(sat(1_000_000_000), "1,000,000");
    }

    #[test]
    fn no_channels_points_to_the_channels_page() {
        assert!(explain(1_000, &[]).unwrap().contains("no channels yet"));
    }

    #[test]
    fn an_unusable_channel_is_not_an_open_door() {
        let mut c = channel();
        c.is_usable = false;
        assert!(explain(1_000, &[c]).unwrap().contains("None of your channels is usable"));
    }

    #[test]
    fn a_balance_that_is_only_the_reserve_cannot_send() {
        let mut c = channel();
        c.max_send_msat = 0;
        c.outbound_msat = 0;
        c.our_balance_sat = Some(1_000);
        let why = explain(500_000, &[c]).unwrap();
        assert!(why.contains("holds 1,000 sat"), "{why}");
        assert!(why.contains("1,000 sat reserve"), "{why}");
    }

    #[test]
    fn more_than_the_spendable_balance_says_how_much_can_be_spent() {
        let why = explain(8_000_000, &[channel()]).unwrap();
        assert!(why.contains("You can spend 7,340 sat"), "{why}");
        assert!(why.contains("1,000 sat of your balance is the reserve"), "{why}");
    }

    #[test]
    fn over_the_per_payment_limit_explains_the_limit() {
        let mut c = channel();
        c.max_send_msat = 1_000_000;
        let why = explain(5_000_000, &[c]).unwrap();
        assert!(why.contains("at most 1,000 sat per payment"), "{why}");
    }

    #[test]
    fn a_payment_as_large_as_the_channel_is_refused() {
        // channel() has a capacity of 10,000 sat.
        let why = explain(10_000_000, &[channel()]).unwrap();
        assert!(why.contains("A 10,000 sat channel cannot carry a payment of 10,000 sat or more"), "{why}");
        assert!(explain(12_000_000, &[channel()]).unwrap().contains("cannot carry"));
    }

    #[test]
    fn a_payment_below_the_channel_size_is_allowed() {
        let mut c = channel();
        c.max_send_msat = 7_340_000;
        assert_eq!(explain(5_000_000, &[c]), None);
    }

    #[test]
    fn a_payment_that_fits_has_no_problem() {
        assert_eq!(explain(500_000, &[channel()]), None);
        assert_eq!(explain(1_000_000, &[channel()]), None);
    }

    #[test]
    fn the_roomiest_usable_channel_decides() {
        let mut small = channel();
        small.max_send_msat = 100_000;
        small.outbound_msat = 100_000;
        let big = channel();
        assert_eq!(explain(500_000, &[small, big]), None);
    }
}
