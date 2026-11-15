//! Cards in the venue's two-character notation (<https://docs.openpoker.ai/llms-full.txt>, "Card
//! Format"): a rank, then a suit.

use svanbot_core::card::{Card, CardSet, Rank, Suit};

#[test]
fn the_venue_examples_name_the_cards_they_say() {
    assert_eq!("Ah".parse(), Ok(Card::new(Rank::Ace, Suit::Hearts)));
    assert_eq!("Ts".parse(), Ok(Card::new(Rank::Ten, Suit::Spades)));
    assert_eq!("2c".parse(), Ok(Card::new(Rank::Two, Suit::Clubs)));
}

#[test]
fn a_card_is_written_in_the_venue_notation() {
    assert_eq!(Card::new(Rank::Ace, Suit::Hearts).to_string(), "Ah");
    assert_eq!(Card::new(Rank::Ten, Suit::Spades).to_string(), "Ts");
    assert_eq!(Card::new(Rank::Two, Suit::Clubs).to_string(), "2c");
}

#[test]
fn every_card_reads_back_as_itself() {
    for rank in Rank::ALL {
        for suit in Suit::ALL {
            let card = Card::new(rank, suit);
            assert_eq!(card.to_string().parse(), Ok(card));
            assert_eq!((card.rank(), card.suit()), (rank, suit));
        }
    }
}

#[test]
fn text_that_is_not_one_card_is_refused() {
    // `??` is the venue's hidden card: runtime reads it as "hidden", never as a card.
    for text in ["??", "", "A", "Ahh", "ah", "AH", "1h", "10h", "Ax", " Ah"] {
        assert!(text.parse::<Card>().is_err(), "{text:?} was read as a card");
    }
}

fn card(text: &str) -> Card {
    text.parse().unwrap()
}

#[test]
fn a_board_reads_as_the_set_of_its_cards() {
    let board: CardSet = "AhKhQh".parse().unwrap();
    assert_eq!(board.len(), 3);
    assert!(board.contains(card("Kh")));
    assert!(!board.contains(card("Ks")));
}

#[test]
fn a_set_lists_its_cards_lowest_first() {
    let set: CardSet = "Ah2cKdAs".parse().unwrap();
    let cards: Vec<Card> = set.iter().collect();
    assert_eq!(cards, [card("2c"), card("Kd"), card("Ah"), card("As")]);
    assert_eq!(set.to_string(), "2cKdAhAs");
}

#[test]
fn a_set_names_each_card_once() {
    assert!("AhKdAh".parse::<CardSet>().is_err());
    assert!("AhK".parse::<CardSet>().is_err());
    assert_eq!("".parse::<CardSet>(), Ok(CardSet::EMPTY));
}

#[test]
fn adding_a_card_the_set_holds_changes_nothing() {
    let mut set = CardSet::EMPTY;
    assert!(set.insert(card("7d")));
    assert!(!set.insert(card("7d")));
    assert_eq!(set.len(), 1);
}
