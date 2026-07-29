//! Building AMCP commands for the wire.
//!
//! The escaping rules here are taken from the server's own tokenizer
//! (`src/protocol/util/tokenize.cpp` in CasparCG 2.5.0), not from the wiki:
//! the server splits on spaces, `"` toggles a quoted run, and `\` starts an
//! escape where only `\\`, `\"` and `\n` mean anything (any other escaped
//! character is *dropped*). Parentheses group a parameter list into one token
//! while unquoted, which is why a value containing them has to be quoted.

use std::fmt;

/// What a command is aimed at.
///
/// AMCP puts the target in the first parameter slot: `PLAY 1-10 AMB` targets
/// channel 1 layer 10. Server-wide commands (`CLS`, `VERSION`) have no target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// No target token — a server-wide command.
    Server,
    /// A whole channel, 1-based.
    Channel(u32),
    /// A layer within a channel, both 1-based as AMCP counts them.
    Layer(u32, u32),
}

impl Target {
    fn token(self) -> Option<String> {
        match self {
            Target::Server => None,
            Target::Channel(c) => Some(c.to_string()),
            Target::Layer(c, l) => Some(format!("{c}-{l}")),
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.token() {
            Some(t) => f.write_str(&t),
            None => f.write_str("-"),
        }
    }
}

/// One AMCP command: a name, an optional target, and parameters.
///
/// Built fluently so call sites read like the protocol does:
/// `Command::new("PLAY").layer(1, 10).arg("AMB").arg("LOOP")`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    name: String,
    target: Target,
    params: Vec<String>,
}

impl Command {
    /// A new command. `name` may contain spaces for the two-word commands
    /// (`MIXER OPACITY`, `DATA STORE`, `CG ADD`) — it is emitted verbatim.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), target: Target::Server, params: Vec::new() }
    }

    /// Aim the command at a whole channel.
    pub fn channel(mut self, ch: u32) -> Self {
        self.target = Target::Channel(ch);
        self
    }

    /// Aim the command at a layer within a channel.
    pub fn layer(mut self, ch: u32, layer: u32) -> Self {
        self.target = Target::Layer(ch, layer);
        self
    }

    /// Aim the command at an explicit [`Target`].
    pub fn target(mut self, target: Target) -> Self {
        self.target = target;
        self
    }

    /// Append a parameter. It is escaped and quoted only if it needs to be.
    pub fn arg(mut self, value: impl fmt::Display) -> Self {
        self.params.push(value.to_string());
        self
    }

    /// Append a parameter only when `value` is `Some`, for AMCP's many
    /// optional trailing parameters.
    pub fn opt(self, value: Option<impl fmt::Display>) -> Self {
        match value {
            Some(v) => self.arg(v),
            None => self,
        }
    }

    /// Append several parameters.
    pub fn args<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: fmt::Display,
    {
        self.params.extend(values.into_iter().map(|v| v.to_string()));
        self
    }

    /// The command name as given.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Serialise to the wire, including the trailing CRLF.
    ///
    /// The target's position is not where it looks like it should be. The
    /// server's parser (`amcp_command_repository.cpp:165`) pops **one** token as
    /// the command name, *then* parses the channel spec, and only then joins the
    /// next token as a sub-command. So a two-word command aimed at a channel has
    /// the target **between** its two words:
    ///
    /// ```text
    /// MIXER 1-10 FILL 0 0 0.5 0.5      ✓ 202 MIXER OK
    /// MIXER FILL 1-10 0 0 0.5 0.5      ✗ 400 ERROR
    /// CG 1-20 ADD 1 lower-third 1      ✓
    /// ```
    ///
    /// Global two-word commands (`DATA STORE`, `INFO CONFIG`, `CLEAR ALL`) have
    /// no target and are unaffected. Verified against a live 2.5.0 server.
    pub fn to_wire(&self) -> String {
        let mut out = String::with_capacity(32 + self.name.len());

        match (self.name.split_once(' '), self.target.token()) {
            // Two words *and* a target: the target splits the name.
            (Some((head, tail)), Some(target)) => {
                out.push_str(head);
                out.push(' ');
                out.push_str(&target);
                out.push(' ');
                out.push_str(tail);
            }
            (_, target) => {
                out.push_str(&self.name);
                if let Some(t) = target {
                    out.push(' ');
                    out.push_str(&t);
                }
            }
        }

        for p in &self.params {
            out.push(' ');
            out.push_str(&escape(p));
        }
        out.push_str("\r\n");
        out
    }
}

impl fmt::Display for Command {
    /// The wire form without the CRLF — for logs and the command console.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_wire().trim_end_matches("\r\n"))
    }
}

/// Escape a single parameter, quoting it when the tokenizer would otherwise
/// split or mangle it.
///
/// Quoting is applied for the empty string (so a caller can send a deliberate
/// empty parameter), whitespace, quotes, backslashes, and parentheses — the
/// last because unquoted parens open a parameter-list token on the server.
pub fn escape(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\\' | '(' | ')'));

    let mut out = String::with_capacity(value.len() + 2);
    if needs_quotes {
        out.push('"');
    }
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            // A bare CR would terminate the command line early; the server has
            // no escape for it, and \n is the closest faithful rendering.
            '\r' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    if needs_quotes {
        out.push('"');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_parameters_are_not_quoted() {
        assert_eq!(escape("AMB"), "AMB");
        assert_eq!(escape("1.0"), "1.0");
        assert_eq!(escape("-0.5"), "-0.5");
    }

    #[test]
    fn spaces_and_specials_force_quotes() {
        assert_eq!(escape("my clip"), "\"my clip\"");
        assert_eq!(escape(""), "\"\"");
        assert_eq!(escape("hue=h=120"), "hue=h=120");
        assert_eq!(escape("scale=(iw/2)"), "\"scale=(iw/2)\"");
    }

    #[test]
    fn escapes_match_the_server_tokenizer() {
        // Only \\ \" and \n mean anything to the server; everything else that
        // follows a backslash is dropped, so we must never emit a bare one.
        assert_eq!(escape(r"C:\media\clip"), r#""C:\\media\\clip""#);
        assert_eq!(escape("say \"hi\""), r#""say \"hi\"""#);
        assert_eq!(escape("two\nlines"), "\"two\\nlines\"");
        assert_eq!(escape("cr\rlf"), "\"cr\\nlf\"");
    }

    #[test]
    fn targets_render_in_amcp_form() {
        assert_eq!(Command::new("CLS").to_wire(), "CLS\r\n");
        assert_eq!(Command::new("CLEAR").channel(1).to_wire(), "CLEAR 1\r\n");
        assert_eq!(
            Command::new("PLAY").layer(1, 10).arg("AMB").arg("LOOP").to_wire(),
            "PLAY 1-10 AMB LOOP\r\n"
        );
    }

    #[test]
    fn a_targeted_two_word_command_puts_the_target_in_the_middle() {
        // The server's parser takes one token as the name, then the channel
        // spec, then the sub-command. Verified against a live 2.5.0 server:
        // the other order returns 400 ERROR.
        assert_eq!(
            Command::new("MIXER OPACITY").layer(1, 10).arg(0.5).to_wire(),
            "MIXER 1-10 OPACITY 0.5\r\n"
        );
        assert_eq!(
            Command::new("MIXER FILL").layer(1, 10).args(["0", "0", "0.5", "0.5"]).to_wire(),
            "MIXER 1-10 FILL 0 0 0.5 0.5\r\n"
        );
        assert_eq!(
            Command::new("MIXER CLEAR").channel(2).to_wire(),
            "MIXER 2 CLEAR\r\n"
        );
        assert_eq!(
            Command::new("CG ADD").layer(1, 20).arg(1).arg("tpl").arg(1).to_wire(),
            "CG 1-20 ADD 1 tpl 1\r\n"
        );
    }

    #[test]
    fn an_untargeted_two_word_command_keeps_its_words_together() {
        assert_eq!(Command::new("CLEAR ALL").to_wire(), "CLEAR ALL\r\n");
        assert_eq!(Command::new("INFO CONFIG").to_wire(), "INFO CONFIG\r\n");
        assert_eq!(
            Command::new("DATA STORE").arg("name").arg("payload").to_wire(),
            "DATA STORE name payload\r\n"
        );
        assert_eq!(
            Command::new("OSC SUBSCRIBE").arg(6250).to_wire(),
            "OSC SUBSCRIBE 6250\r\n"
        );
    }

    #[test]
    fn a_one_word_command_puts_the_target_after_the_name() {
        assert_eq!(Command::new("INFO").channel(1).to_wire(), "INFO 1\r\n");
        assert_eq!(Command::new("PLAY").layer(1, 10).to_wire(), "PLAY 1-10\r\n");
    }

    #[test]
    fn opt_skips_none() {
        let c = Command::new("PLAY").layer(1, 10).arg("AMB").opt(None::<String>);
        assert_eq!(c.to_wire(), "PLAY 1-10 AMB\r\n");
        let c = Command::new("PLAY").layer(1, 10).arg("AMB").opt(Some("LOOP"));
        assert_eq!(c.to_wire(), "PLAY 1-10 AMB LOOP\r\n");
    }

    #[test]
    fn display_omits_the_crlf() {
        assert_eq!(Command::new("PLAY").channel(1).to_string(), "PLAY 1");
    }
}
