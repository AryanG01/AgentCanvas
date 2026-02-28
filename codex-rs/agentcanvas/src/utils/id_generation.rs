//! ID generation utilities

use crate::schema::{ItemId, TurnId};

/// Generate a turn ID from a counter
pub fn generate_turn_id(counter: usize) -> TurnId {
    format!("turn_{}", counter)
}

/// Generate an item ID from a counter
pub fn generate_item_id(counter: usize) -> ItemId {
    format!("item_{}", counter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_turn_id() {
        assert_eq!(generate_turn_id(0), "turn_0");
        assert_eq!(generate_turn_id(5), "turn_5");
        assert_eq!(generate_turn_id(123), "turn_123");
    }

    #[test]
    fn test_generate_item_id() {
        assert_eq!(generate_item_id(0), "item_0");
        assert_eq!(generate_item_id(5), "item_5");
        assert_eq!(generate_item_id(123), "item_123");
    }
}
