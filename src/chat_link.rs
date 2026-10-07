//! WhatsApp click-to-chat links. Opening one prepares a draft, never a send.

/// A validated recipient and optional URL-decoded draft.
#[derive(Clone, PartialEq, Eq)]
pub struct ChatLink {
    phone: String,
    pub text: Option<String>,
}

// Actions and command-line diagnostics must not disclose the link's contents.
impl std::fmt::Debug for ChatLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChatLink { recipient and draft redacted }")
    }
}

impl ChatLink {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let url = reqwest::Url::parse(value).map_err(|_| "Invalid WhatsApp chat link")?;
        let send = match url.host_str() {
            Some(host) => host.eq_ignore_ascii_case("send") && matches!(url.path(), "" | "/"),
            None => url.path() == "send",
        };
        if url.scheme() != "whatsapp"
            || !send
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
        {
            return Err("Only whatsapp://send chat links are supported");
        }
        let mut phone = None;
        let mut text = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "phone" if phone.is_none() => phone = Some(value.into_owned()),
                "text" if text.is_none() => text = Some(value.into_owned()),
                "phone" | "text" => return Err("Ambiguous WhatsApp chat link"),
                _ => {}
            }
        }
        let phone = phone.ok_or("The WhatsApp chat link needs an international phone number")?;
        let phone = phone.trim().trim_start_matches('+');
        if phone.is_empty()
            || phone.len() > 15
            || phone.starts_with('0')
            || !phone.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("The WhatsApp chat link needs an international phone number");
        }
        let link = Self {
            phone: phone.to_owned(),
            text,
        };
        // fastframe-instance accepts a 16 KiB line, including its prefix and
        // the Windows token. Leave room for those instead of losing a draft
        // only when ZapFast was already running.
        if link.request().len() > 15 * 1024 {
            return Err("The WhatsApp chat link is too long");
        }
        Ok(link)
    }

    pub fn chat_id(&self) -> crate::model::ChatId {
        format!("{}@s.whatsapp.net", self.phone)
    }

    /// URL encoding keeps even multiline drafts inside one private IPC line.
    pub fn request(&self) -> String {
        let mut url = reqwest::Url::parse("whatsapp://send").expect("a fixed WhatsApp URL");
        url.query_pairs_mut().append_pair("phone", &self.phone);
        if let Some(text) = &self.text {
            url.query_pairs_mut().append_pair("text", text);
        }
        format!("open-link {url}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_links_preserve_the_recipient_and_decoded_draft() {
        let link = ChatLink::parse(
            "whatsapp://send/?phone=15550100123&text=Hola%2C+informaci%C3%B3n+%26+precio+%F0%9F%98%8A%0Ahttps%3A%2F%2Fexample.com%2F%3Fa%3Db%26c%3Dd&app_absent=0",
        )
        .unwrap();
        assert_eq!(link.chat_id(), "15550100123@s.whatsapp.net");
        assert_eq!(
            link.text.as_deref(),
            Some("Hola, información & precio 😊\nhttps://example.com/?a=b&c=d")
        );
    }

    #[test]
    fn links_without_a_draft_and_encoded_plus_signs_are_understood() {
        let link = ChatLink::parse("whatsapp:send?phone=%2B15550100123").unwrap();
        assert_eq!(link.chat_id(), "15550100123@s.whatsapp.net");
        assert_eq!(link.text, None);
        let link = ChatLink::parse("whatsapp://send?phone=15550100123&text=a%2Bb%2520c").unwrap();
        assert_eq!(link.text.as_deref(), Some("a+b%20c"));
    }

    #[test]
    fn invalid_or_ambiguous_links_do_not_name_a_recipient() {
        for value in [
            "https://example.com/send?phone=15550100123",
            "whatsapp://call?phone=15550100123",
            "whatsapp://send/path?phone=15550100123",
            "whatsapp://send@elsewhere?phone=15550100123",
            "whatsapp://send:1234?phone=15550100123",
            "whatsapp://send?text=Hello",
            "whatsapp://send?phone=",
            "whatsapp://send?phone=phone",
            "whatsapp://send?phone=0123",
            "whatsapp://send?phone=1234567890123456",
            "whatsapp://send?phone=15550100123&phone=15550100124",
            "whatsapp://send?phone=15550100123&text=one&text=two",
            "whatsapp://send?phone=15550100123#fragment",
        ] {
            assert!(ChatLink::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn private_requests_round_trip_without_newlines_or_debug_payloads() {
        let link =
            ChatLink::parse("whatsapp://send?phone=15550100123&text=one%0Atwo%0Dthree").unwrap();
        let request = link.request();
        assert!(!request.contains(['\n', '\r']));
        assert_eq!(
            ChatLink::parse(request.strip_prefix("open-link ").unwrap()).unwrap(),
            link
        );
        let debug = format!("{link:?}");
        assert!(!debug.contains("15550100123"));
        assert!(!debug.contains("three"));
    }

    #[test]
    fn oversized_drafts_are_refused_before_a_launch_or_handoff() {
        let link = format!(
            "whatsapp://send?phone=15550100123&text={}",
            "a".repeat(16 * 1024)
        );
        assert!(ChatLink::parse(&link).is_err());
    }
}
