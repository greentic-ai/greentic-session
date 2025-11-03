use crate::model::SessionKey;
use sha2::{Digest, Sha256};

/// Deterministic SessionKey from Telegram update fields.
/// Inputs are strings/numbers the caller extracts from their payload.
pub fn telegram_update_to_session_key(bot_id: &str, chat_id: &str, user_id: &str) -> SessionKey {
    let s = format!("tg:{bot_id}:{chat_id}:{user_id}");
    SessionKey(hex_sha(&s))
}

/// Deterministic SessionKey from a generic webhook (source + subject).
pub fn webhook_to_session_key(source: &str, subject: &str, id_hint: &str) -> SessionKey {
    let s = format!("wh:{source}:{subject}:{id_hint}");
    SessionKey(hex_sha(&s))
}

fn hex_sha(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_hash() {
        let key1 = telegram_update_to_session_key("bot", "chat", "user");
        let key2 = telegram_update_to_session_key("bot", "chat", "user");
        assert_eq!(key1, key2);
        assert_ne!(key1, telegram_update_to_session_key("bot", "chat", "user2"));
        let webhook = webhook_to_session_key("crm", "ticket", "42");
        assert_eq!(webhook, webhook_to_session_key("crm", "ticket", "42"));
    }
}
