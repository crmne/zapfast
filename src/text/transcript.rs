//! WhatsApp-style text for selections across messages:
//! `[22:41, 8/18/2026] Ada: message`.
//!
//! Each frame registers its message bodies and headers. The output hook adds
//! those headers when a selection spans multiple messages.

/// Rewrites copied message text in `output_hook`, after egui's selection hook.
pub struct CopyAnnotator {
    /// Message bodies drawn during the frame, in order.
    pub rows: std::sync::Arc<std::sync::Mutex<Vec<Row>>>,
}

impl egui::plugin::Plugin for CopyAnnotator {
    fn debug_name(&self) -> &'static str {
        "zapfast-transcript"
    }

    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut egui::FullOutput) {
        let rows = self.rows.lock().unwrap_or_else(|p| p.into_inner());
        if rows.is_empty() {
            return;
        }
        for command in &mut output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command
                && let Some(refined) = refine(text, &rows)
            {
                *text = refined;
            }
        }
    }
}

/// One drawn message body with its copy header and emoji placeholders.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub header: String,
    pub body: String,
    pub placements: Vec<String>,
    /// Non-text message description:
    /// "\[photo\]", "\[video\]", "\[document: notes.pdf\]", and so on.
    pub marker: Option<String>,
    /// Preformatted reactions, such as " (❤️ Roberta)".
    pub reactions: String,
    /// Preformatted reply context:
    /// `(replying to Roberta: "…")`.
    pub quote: Option<String>,
}

impl Row {
    /// Formats one transcript line from its available parts.
    pub(crate) fn line(&self, body: &str) -> String {
        let mut line = self.header.clone();
        if let Some(quote) = &self.quote {
            line.push_str(quote);
            line.push(' ');
        }
        if let Some(marker) = &self.marker {
            line.push_str(marker);
            if !body.is_empty() {
                line.push(' ');
            }
        }
        line.push_str(body);
        line.push_str(&self.reactions);
        line
    }

    /// Returns the body from `start` with emoji placeholders restored.
    fn restored(&self, start: usize, segment: &str) -> String {
        if self.placements.is_empty() {
            return segment.to_owned();
        }
        let mut next = self.body[..start]
            .chars()
            .filter(|&c| c == crate::emoji::PLACEHOLDER)
            .count();
        let mut out = String::with_capacity(segment.len());
        for character in segment.chars() {
            if character == crate::emoji::PLACEHOLDER {
                match self.placements.get(next) {
                    Some(emoji) => out.push_str(emoji),
                    None => out.push(character),
                }
                next += 1;
            } else {
                out.push(character);
            }
        }
        out
    }
}

/// Restores emoji and adds headers for multi-message selections. Returns
/// `None` for unrelated or unchanged copied text.
pub fn refine(copied: &str, rows: &[Row]) -> Option<String> {
    if let Some(annotated) = annotate(copied, rows) {
        return Some(annotated);
    }
    // Single-message selections only need emoji restored.
    if !copied.contains(crate::emoji::PLACEHOLDER) {
        return None;
    }
    for row in rows {
        if let Some(start) = row.body.find(copied) {
            return Some(row.restored(start, copied));
        }
    }
    None
}

/// Adds a header per message when copied text spans multiple rows.
///
/// Matches egui's copied tail, complete middle bodies, and final head.
pub fn annotate(copied: &str, rows: &[Row]) -> Option<String> {
    for start in 0..rows.len() {
        if let Some(lines) = walk(copied, &rows[start..])
            && lines.len() >= 2
        {
            return Some(lines.join("\n"));
        }
    }
    None
}

/// Parses a copied tail followed by later rows into annotated lines.
fn walk(copied: &str, rows: &[Row]) -> Option<Vec<String>> {
    let row = rows.first()?;
    // Try the longest tail first because the selection continues into the next row.
    for cut in 0..row.body.len() {
        if !row.body.is_char_boundary(cut) {
            continue;
        }
        let part = &row.body[cut..];
        let Some(rest) = copied.strip_prefix(part) else {
            continue;
        };
        let line = row.line(&row.restored(cut, part));
        if rest.is_empty() {
            return Some(vec![line]);
        }
        if let Some(mut tail) = follow(rest, &rows[1..]) {
            tail.insert(0, line);
            return Some(tail);
        }
    }
    None
}

/// Parses complete middle bodies and the final partial body after a separator.
/// Non-text rows crossed by the selection contribute their message kind.
fn follow(copied: &str, rows: &[Row]) -> Option<Vec<String>> {
    let remaining = copied
        .strip_prefix("\n\n")
        .or_else(|| copied.strip_prefix('\n'))?;
    if remaining.is_empty() {
        return None;
    }
    let mut passed: Vec<String> = Vec::new();
    for (skipped, row) in rows.iter().enumerate() {
        if row.body.is_empty() {
            if row.marker.is_some() {
                passed.push(row.line(""));
            }
            continue;
        }
        // The remaining text is the final body's prefix.
        if row.body.starts_with(remaining) {
            passed.push(row.line(&row.restored(0, remaining)));
            return Some(passed);
        }
        // This complete body is followed by more copied text.
        if let Some(rest) = remaining.strip_prefix(row.body.as_str())
            && let Some(tail) = follow(rest, &rows[skipped + 1..])
        {
            passed.push(row.line(&row.restored(0, &row.body)));
            passed.extend(tail);
            return Some(passed);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(header: &str, body: &str) -> Row {
        Row {
            header: header.to_owned(),
            body: body.to_owned(),
            placements: body
                .chars()
                .filter(|&c| c == crate::emoji::PLACEHOLDER)
                .map(|_| "🎉".to_owned())
                .collect(),
            ..Default::default()
        }
    }

    fn rows() -> Vec<Row> {
        vec![
            row("[22:41, 8/18/2026] Carmine: ", "the analyzer is crazy good"),
            row(
                "[18:21, 8/30/2026] Ada: ",
                "Hello from France!\nSee you soon",
            ),
            row("[18:27, 8/30/2026] Carmine: ", "Sure I'll take a look"),
        ]
    }

    #[test]
    fn a_span_of_messages_gets_a_header_each() {
        let copied = "crazy good\nHello from France!\nSee you soon\nSure";
        assert_eq!(
            annotate(copied, &rows()).as_deref(),
            Some(
                "[22:41, 8/18/2026] Carmine: crazy good\n\
                 [18:21, 8/30/2026] Ada: Hello from France!\nSee you soon\n\
                 [18:27, 8/30/2026] Carmine: Sure"
            )
        );
        // egui may insert a blank line between bubbles.
        let gapped = "good\n\nHello from France!\nSee you soon";
        assert_eq!(
            annotate(gapped, &rows()).as_deref(),
            Some(
                "[22:41, 8/18/2026] Carmine: good\n\
                 [18:21, 8/30/2026] Ada: Hello from France!\nSee you soon"
            )
        );
    }

    #[test]
    fn within_one_message_nothing_changes() {
        assert_eq!(annotate("analyzer is crazy", &rows()), None);
        assert_eq!(annotate("the analyzer is crazy good", &rows()), None);
        assert_eq!(annotate("something else entirely", &rows()), None);
        assert_eq!(annotate("", &rows()), None);
        assert_eq!(annotate("anything", &[]), None);
    }

    #[test]
    fn emoji_come_back_out_of_their_placeholders() {
        let placeholder = crate::emoji::PLACEHOLDER;
        let rows = vec![Row {
            header: "[9:00, 1/2/2026] Ada: ".to_owned(),
            body: format!("well {placeholder} done {placeholder}"),
            placements: vec!["🎂".to_owned(), "🎈".to_owned()],
            ..Default::default()
        }];
        // Single-message copy: restore emoji without a header.
        assert_eq!(
            refine(&format!("{placeholder} done {placeholder}"), &rows).as_deref(),
            Some("🎂 done 🎈")
        );
        assert_eq!(
            refine(&format!("done {placeholder}"), &rows).as_deref(),
            Some("done 🎈"),
            "the offset counts placeholders before the selection"
        );
        assert_eq!(refine("well", &rows), None, "nothing to do");
        // Multi-message copy: restore emoji and add headers.
        let mut both = rows.clone();
        both.push(Row {
            header: "[9:01, 1/2/2026] Ada: ".to_owned(),
            body: "and again".to_owned(),
            placements: Vec::new(),
            ..Default::default()
        });
        assert_eq!(
            refine(&format!("done {placeholder}\nand again"), &both).as_deref(),
            Some("[9:00, 1/2/2026] Ada: done 🎈\n[9:01, 1/2/2026] Ada: and again")
        );
    }

    #[test]
    fn a_swept_over_picture_is_named_and_an_unknown_stays_silent() {
        let mut photo = row("[9:01, 1/2/2026] Ada: ", "");
        photo.marker = Some("[photo]".to_owned());
        let with_gap = vec![
            row("[9:00, 1/2/2026] Ada: ", "before the picture"),
            photo,
            row("[9:02, 1/2/2026] Ada: ", "after the picture"),
        ];
        let copied = "the picture\n\nafter the picture";
        assert_eq!(
            annotate(copied, &with_gap).as_deref(),
            Some(
                "[9:00, 1/2/2026] Ada: the picture\n[9:01, 1/2/2026] Ada: [photo]\n[9:02, 1/2/2026] Ada: after the picture"
            )
        );
        let silent = vec![
            row("[9:00, 1/2/2026] Ada: ", "before the picture"),
            row("[9:01, 1/2/2026] Ada: ", ""),
            row("[9:02, 1/2/2026] Ada: ", "after the picture"),
        ];
        assert_eq!(
            annotate(copied, &silent).as_deref(),
            Some("[9:00, 1/2/2026] Ada: the picture\n[9:02, 1/2/2026] Ada: after the picture")
        );
    }

    #[test]
    fn reactions_quotes_and_kinds_ride_along() {
        let mut first = row("[9:00, 1/2/2026] Ada: ", "look at this");
        first.marker = Some("[photo]".to_owned());
        first.reactions = " (❤️ You)".to_owned();
        let mut second = row("[9:05, 1/2/2026] You: ", "lovely");
        second.quote = Some("(replying to Ada: \"look at this\")".to_owned());
        let copied = "at this\nlovely";
        assert_eq!(
            annotate(copied, &[first, second]).as_deref(),
            Some(
                "[9:00, 1/2/2026] Ada: [photo] at this (❤️ You)\n[9:05, 1/2/2026] You: (replying to Ada: \"look at this\") lovely"
            )
        );
    }
}
