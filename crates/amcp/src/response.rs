//! Decoding AMCP responses.
//!
//! The framing is entirely determined by the status code, which is why this is
//! a small state machine rather than a line-per-response reader. From the 2.5.0
//! server source:
//!
//! - `100` — an asynchronous notification, no data.
//! - `101` — a notification with exactly one following line.
//! - `200` — success with *many* lines, terminated by an empty line.
//! - `201` — success with exactly one following line.
//! - `202` — success with no data.
//! - `4xx` / `5xx` — a failure, single line, no data.
//!
//! Getting this wrong desynchronises the stream for every later command, so the
//! rules are encoded once, here, and tested against real server transcripts.

/// A decoded response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The request id echoed back, when the command was sent as
    /// `REQ <id> <COMMAND>` and the server prefixed its reply `RES <id> `.
    ///
    /// This is how replies are correlated. It matters more than it looks:
    /// the server dispatches commands to *per-channel* queues
    /// (`AMCPProtocolStrategy.cpp:216`), so a reply for channel 2 can overtake
    /// one for channel 1 and simple FIFO matching would pair them wrongly.
    pub id: Option<String>,
    /// The AMCP status code.
    pub code: u16,
    /// The rest of the status line — usually `<COMMAND> OK` or a failure note.
    pub status: String,
    /// Data lines, without the terminating empty line of a 200.
    pub lines: Vec<String>,
}

impl Response {
    /// True for 2xx.
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.code)
    }

    /// True for the 1xx asynchronous notifications, which arrive unsolicited
    /// and must not be matched to a pending command.
    pub fn is_notification(&self) -> bool {
        (100..200).contains(&self.code)
    }

    /// True for 4xx (client fault) and 5xx (server fault).
    pub fn is_error(&self) -> bool {
        self.code >= 400
    }

    /// The single data line of a 201, if there is one.
    pub fn single(&self) -> Option<&str> {
        self.lines.first().map(String::as_str)
    }
}

/// The synthetic code given to a `PONG`, which arrives with no status code of
/// its own. 200-range so [`Response::is_ok`] treats it as the success it is.
pub const PONG_CODE: u16 = 200;

/// How many data lines a code carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    None,
    One,
    UntilBlank,
}

fn framing_for(code: u16) -> Framing {
    match code {
        // 400 echoes the offending command back on its own line
        // (`AMCPProtocolStrategy.cpp:151`). Every other error code is a bare
        // status line — 401/402/500/503 included. Verified against a live 2.5.0
        // server: treating 400 as dataless leaves the echoed line in the buffer,
        // where it is either dropped or, if it happens to begin with three
        // digits, misread as the next response.
        101 | 201 | 400 => Framing::One,
        200 => Framing::UntilBlank,
        _ => Framing::None,
    }
}

#[derive(Debug)]
struct Partial {
    id: Option<String>,
    code: u16,
    status: String,
    lines: Vec<String>,
    framing: Framing,
}

impl Partial {
    fn finish(self) -> Response {
        Response { id: self.id, code: self.code, status: self.status, lines: self.lines }
    }
}

/// Incremental decoder: feed it bytes, take whole responses out.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    partial: Option<Partial>,
}

impl Decoder {
    /// A decoder with an empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed received bytes and return every response that completed.
    ///
    /// Invalid UTF-8 is replaced rather than rejected: a malformed media
    /// filename should not take down the control connection.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Response> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(line) = self.take_line() {
            if let Some(r) = self.consume(line) {
                out.push(r);
            }
        }
        out
    }

    /// Pop one CRLF- (or LF-) terminated line from the buffer.
    fn take_line(&mut self) -> Option<String> {
        let nl = self.buf.iter().position(|&b| b == b'\n')?;
        let mut line: Vec<u8> = self.buf.drain(..=nl).collect();
        line.pop(); // \n
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        Some(String::from_utf8_lossy(&line).into_owned())
    }

    /// Apply one line to the state machine.
    fn consume(&mut self, line: String) -> Option<Response> {
        if let Some(mut p) = self.partial.take() {
            match p.framing {
                Framing::One => {
                    p.lines.push(line);
                    return Some(p.finish());
                }
                Framing::UntilBlank => {
                    if line.is_empty() {
                        return Some(p.finish());
                    }
                    p.lines.push(line);
                    self.partial = Some(p);
                    return None;
                }
                Framing::None => unreachable!("a Framing::None response is never left partial"),
            }
        }

        // `PING` is answered `PONG …` with no status code at all, and it
        // ignores the REQ id (`AMCPProtocolStrategy.cpp:126`). Surfaced as a
        // synthetic 200 so a console command line does not simply hang.
        if line.starts_with("PONG") {
            return Some(Response { id: None, code: PONG_CODE, status: line, lines: Vec::new() });
        }

        // A status line: an optional `RES <id> `, three digits, then the rest.
        let (id, code, status) = match parse_status(&line) {
            Some(v) => v,
            // Not a status line and nothing pending — the stream is out of
            // step, or the server logged something unexpected. Dropping the
            // line resynchronises at the next status code rather than
            // mis-attributing data to a later command.
            None => {
                if !line.is_empty() {
                    tracing::warn!(line = %line, "ignoring unframed AMCP line");
                }
                return None;
            }
        };

        match framing_for(code) {
            Framing::None => Some(Response { id, code, status, lines: Vec::new() }),
            framing => {
                self.partial = Some(Partial { id, code, status, lines: Vec::new(), framing });
                None
            }
        }
    }
}

fn parse_status(line: &str) -> Option<(Option<String>, u16, String)> {
    // `RES <id> ` precedes the status code when the command carried a REQ id.
    let (id, rest) = match line.strip_prefix("RES ") {
        Some(after) => {
            let mut it = after.splitn(2, ' ');
            let id = it.next()?.to_string();
            (Some(id), it.next()?)
        }
        None => (None, line),
    };

    let digits: String = rest.chars().take(3).collect();
    if digits.len() != 3 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let code = digits.parse().ok()?;
    let status = rest[3..].trim_start().to_string();
    Some((id, code, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(s: &str) -> Vec<Response> {
        Decoder::new().feed(s.as_bytes())
    }

    #[test]
    fn code_202_has_no_data() {
        let r = decode("202 PLAY OK\r\n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].code, 202);
        assert_eq!(r[0].status, "PLAY OK");
        assert!(r[0].lines.is_empty());
        assert!(r[0].is_ok());
    }

    #[test]
    fn code_201_takes_exactly_one_line() {
        let r = decode("201 VERSION OK\r\n2.5.0.0 STABLE\r\n202 PLAY OK\r\n");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].single(), Some("2.5.0.0 STABLE"));
        assert_eq!(r[1].code, 202);
    }

    #[test]
    fn code_200_runs_until_a_blank_line() {
        // Exactly the shape info_command() emits in 2.5.0.
        let r = decode("200 INFO OK\r\n1 720p5000 PLAYING\r\n2 1080p2500 PLAYING\r\n\r\n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].lines, vec!["1 720p5000 PLAYING", "2 1080p2500 PLAYING"]);
    }

    #[test]
    fn empty_200_is_just_the_blank_line() {
        let r = decode("200 CLS OK\r\n\r\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].lines.is_empty());
    }

    #[test]
    fn errors_are_single_line() {
        let r = decode("404 PLAY ERROR\r\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].is_error());
        assert!(!r[0].is_ok());
    }

    #[test]
    fn code_400_echoes_the_offending_command() {
        // Exactly what a live 2.5.0 server sends for an unknown command. If 400
        // were treated as dataless, the echoed line would be left in the buffer.
        let r = decode("400 ERROR\r\nNOSUCHCOMMAND\r\n202 PLAY OK\r\n");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].code, 400);
        assert_eq!(r[0].single(), Some("NOSUCHCOMMAND"));
        assert_eq!(r[1].code, 202, "the next response must still be framed correctly");
    }

    #[test]
    fn other_error_codes_carry_no_data() {
        let r = decode("401 PLAY ERROR\r\n402 MIXER ERROR\r\n500 FAILED\r\n503 PLAY FAILED\r\n");
        assert_eq!(r.len(), 4);
        assert!(r.iter().all(|x| x.lines.is_empty()));
        assert_eq!(r[3].code, 503);
    }

    #[test]
    fn pong_is_surfaced_rather_than_dropped() {
        // PING is answered with no status code and no RES prefix, so without
        // this a caller would wait out the full reply timeout.
        let r = decode("PONG hello\r\n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].code, PONG_CODE);
        assert_eq!(r[0].status, "PONG hello");
        assert!(r[0].is_ok());
        assert_eq!(r[0].id, None);
    }

    #[test]
    fn split_across_reads() {
        let mut d = Decoder::new();
        assert!(d.feed(b"201 VER").is_empty());
        assert!(d.feed(b"SION OK\r\n2.5.").is_empty());
        let r = d.feed(b"0.0 STABLE\r\n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].single(), Some("2.5.0.0 STABLE"));
    }

    #[test]
    fn notifications_are_flagged_and_101_carries_a_line() {
        let r = decode("101 INFO\r\nsome event\r\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].is_notification());
        assert_eq!(r[0].single(), Some("some event"));
    }

    #[test]
    fn a_blank_line_inside_200_data_ends_it() {
        // The framing is what it is: an empty data line is indistinguishable
        // from the terminator, so the next line must parse as a status code.
        let r = decode("200 CLS OK\r\na\r\n\r\n202 PLAY OK\r\n");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].lines, vec!["a"]);
        assert_eq!(r[1].code, 202);
    }

    #[test]
    fn bare_lf_is_accepted() {
        let r = decode("202 PLAY OK\n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, None);
    }

    #[test]
    fn res_prefix_carries_the_request_id() {
        let r = decode("RES 42 202 PLAY OK\r\n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id.as_deref(), Some("42"));
        assert_eq!(r[0].code, 202);
        assert_eq!(r[0].status, "PLAY OK");
    }

    #[test]
    fn res_prefix_only_tags_the_status_line_of_a_200() {
        let r = decode("RES 7 200 INFO OK\r\n1 720p5000 PLAYING\r\n\r\n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id.as_deref(), Some("7"));
        assert_eq!(r[0].lines, vec!["1 720p5000 PLAYING"]);
    }

    #[test]
    fn replies_may_arrive_out_of_order() {
        // Per-channel queues mean this really happens; the ids are what make
        // it recoverable.
        let r = decode("RES 2 202 PLAY OK\r\nRES 1 202 STOP OK\r\n");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].id.as_deref(), Some("2"));
        assert_eq!(r[1].id.as_deref(), Some("1"));
    }

    #[test]
    fn a_batch_replies_per_command_plus_one_commit() {
        // Verified against AMCPCommandQueue::Execute in 2.5.0: every inner
        // command replies, then the group replies once under BEGIN's id.
        let r = decode("RES 2 202 PLAY OK\r\nRES 3 202 PLAY OK\r\nRES 1 202 COMMIT OK\r\n");
        assert_eq!(r.len(), 3);
        assert_eq!(r[2].status, "COMMIT OK");
    }
}
