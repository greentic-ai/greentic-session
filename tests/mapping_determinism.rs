use greentic_session::mapping::{telegram_update_to_session_key, webhook_to_session_key};
use proptest::prelude::*;

#[test]
fn deterministic_telegram_mapping() {
    let key1 = telegram_update_to_session_key("bot42", "chat9001", "user5");
    let key2 = telegram_update_to_session_key("bot42", "chat9001", "user5");
    assert_eq!(key1, key2);
}

#[test]
fn deterministic_webhook_mapping() {
    let a = webhook_to_session_key("crm", "ticket", "1234");
    let b = webhook_to_session_key("crm", "ticket", "1234");
    assert_eq!(a, b);
    let c = webhook_to_session_key("crm", "ticket", "12345");
    assert_ne!(a, c);
}

proptest! {
    #[test]
    fn telegram_keys_are_stable(bot in "\\PC*", chat in "\\PC*", user in "\\PC*") {
        let key1 = telegram_update_to_session_key(&bot, &chat, &user);
        let key2 = telegram_update_to_session_key(&bot, &chat, &user);
        prop_assert_eq!(key1, key2);
    }

    #[test]
    fn webhook_keys_change_with_inputs(src in "\\PC*", subject in "\\PC*", id in "\\PC*") {
        let base = webhook_to_session_key(&src, &subject, &id);
        let alt_id = format!("{id}:alt");
        let alt_subject = format!("{subject}-alt");
        let alt_src = format!("alt-{src}");

        let id_key = webhook_to_session_key(&src, &subject, &alt_id);
        let subject_key = webhook_to_session_key(&src, &alt_subject, &id);
        let src_key = webhook_to_session_key(&alt_src, &subject, &id);

        prop_assert_ne!(base, id_key);
        prop_assert_ne!(base, subject_key);
        prop_assert_ne!(base, src_key);
    }
}
