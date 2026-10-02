use invoice_core::bech32;
use invoice_core::hrp::parse_amount;
use invoice_core::words::{be_int, from_bytes, to_bytes};
use proptest::prelude::*;

proptest! {
    #[test]
    fn bytes_to_words_and_back(bytes in proptest::collection::vec(any::<u8>(), 0..700)) {
        prop_assert_eq!(to_bytes(&from_bytes(&bytes), false), bytes);
    }

    #[test]
    fn words_are_five_bits(bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
        prop_assert!(from_bytes(&bytes).iter().all(|&w| w < 32));
    }

    #[test]
    fn big_endian_int_round_trip(value in any::<u32>()) {
        let words = from_bytes(&value.to_be_bytes());
        // 32 bits become 7 words with 3 padding bits at the bottom.
        prop_assert_eq!(be_int(&words), Some(u64::from(value) << 3));
    }

    #[test]
    fn bech32_round_trip(words in proptest::collection::vec(0u8..32, 0..400)) {
        let encoded = bech32::encode("lnbcrt", &words);
        let decoded = bech32::decode(&encoded).unwrap();
        prop_assert_eq!(decoded.data, words);
    }

    #[test]
    fn any_single_character_change_breaks_the_checksum(
        words in proptest::collection::vec(0u8..32, 10..200),
        index in any::<prop::sample::Index>(),
        delta in 1u8..32,
    ) {
        let mut tampered = words.clone();
        let i = index.index(tampered.len());
        tampered[i] = (tampered[i] + delta) % 32;
        let mut encoded = bech32::encode("lnbc", &words);
        let checksum = encoded.split_off(encoded.len() - 6);
        let forged = format!("lnbc1{}{checksum}", tampered.iter().map(|&w| bech32::word_to_char(w)).collect::<String>());
        prop_assert!(bech32::decode(&forged).is_err());
    }

    #[test]
    fn micro_amounts(value in 1u64..10_000_000) {
        prop_assert_eq!(parse_amount(&format!("{value}u")).unwrap(), value * 100_000);
    }

    #[test]
    fn pico_amounts_need_a_trailing_zero(value in 1u64..1_000_000_000) {
        let amount = format!("{value}p");
        prop_assert_eq!(parse_amount(&amount).is_ok(), value % 10 == 0);
    }
}
