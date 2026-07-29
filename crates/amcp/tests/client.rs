//! Client tests against an in-process fake server.
//!
//! These exist for one reason: CasparCG replies out of order. Commands are
//! dispatched to one queue per channel, so a reply for channel 2 can overtake a
//! reply for channel 1. Anything that pairs replies by arrival order looks
//! correct in a demo and mis-attributes results in a show.

use std::time::Duration;

use amcp::{commands, Client, Command};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// Start a fake server. Returns the client, and a channel of the command lines
/// the server received.
async fn fake_server<F>(respond: F) -> (Client, mpsc::UnboundedReceiver<String>)
where
    F: Fn(&str, &mut Vec<String>) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (seen_tx, seen_rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = seen_tx.send(line.clone());
            let mut out = Vec::new();
            respond(&line, &mut out);
            for chunk in out {
                if write.write_all(chunk.as_bytes()).await.is_err() {
                    return;
                }
            }
        }
    });

    let client = Client::connect(addr).await.unwrap();
    (client, seen_rx)
}

/// Pull the `REQ <id>` out of a command line.
fn req_id(line: &str) -> String {
    line.split_whitespace().nth(1).unwrap_or_default().to_string()
}

#[tokio::test]
async fn a_command_gets_its_own_reply() {
    let (client, mut seen) = fake_server(|line, out| {
        out.push(format!("RES {} 202 PLAY OK\r\n", req_id(line)));
    })
    .await;

    let resp = client.send(commands::play(1, 10)).await.unwrap();
    assert_eq!(resp.code, 202);
    assert_eq!(seen.recv().await.unwrap(), "REQ 1 PLAY 1-10");
}

#[tokio::test]
async fn replies_are_matched_by_id_not_by_order() {
    // The server answers every command, but delays the first one so its reply
    // arrives last — exactly what per-channel queues cause.
    let (client, _seen) = fake_server(|line, out| {
        let id = req_id(line);
        if line.contains("STOP") {
            // Answer the *later* command first, then the earlier one.
            out.push(format!("RES {id} 202 STOP OK\r\n"));
            out.push("RES 1 201 VERSION OK\r\n2.5.0.0 STABLE\r\n".to_string());
        }
    })
    .await;

    let version = client.send(Command::new("VERSION"));
    // Give the first command time to be written before the second.
    tokio::time::sleep(Duration::from_millis(20)).await;
    let stop = client.send(commands::stop(1, 10));

    let (version, stop) = tokio::join!(version, stop);
    let version = version.unwrap();
    let stop = stop.unwrap();

    // If these were paired by arrival order, VERSION would have been handed
    // the STOP reply.
    assert_eq!(version.code, 201);
    assert_eq!(version.single(), Some("2.5.0.0 STABLE"));
    assert_eq!(stop.code, 202);
    assert_eq!(stop.status, "STOP OK");
}

#[tokio::test]
async fn an_error_reply_becomes_an_error() {
    let (client, _seen) = fake_server(|line, out| {
        out.push(format!("RES {} 404 PLAY ERROR\r\n", req_id(line)));
    })
    .await;

    let err = client.send(commands::play_clip(1, 10, "missing", false, None)).await.unwrap_err();
    match err {
        amcp::Error::Amcp { code, .. } => assert_eq!(code, 404),
        other => panic!("expected an AMCP error, got {other:?}"),
    }

    // …but send_raw hands the response back for callers that want to inspect it.
    let resp = client.send_raw(commands::play(1, 10)).await.unwrap();
    assert_eq!(resp.code, 404);
}

#[tokio::test]
async fn a_batch_is_wrapped_in_begin_commit_and_answered_once() {
    let (client, mut seen) = fake_server(|line, out| {
        // BEGIN is never answered by the real server.
        if line.contains("BEGIN") {
            return;
        }
        if line.trim() == "COMMIT" {
            out.push("RES 1 202 COMMIT OK\r\n".to_string());
            return;
        }
        // Inner commands each reply too, under their own id.
        out.push(format!("RES {} 202 PLAY OK\r\n", req_id(line)));
    })
    .await;

    let resp = client
        .batch(vec![commands::play_clip(1, 10, "a", false, None), commands::play_clip(2, 10, "b", false, None)])
        .await
        .unwrap();

    assert_eq!(resp.status, "COMMIT OK");

    let lines: Vec<String> =
        (0..4).map(|_| seen.try_recv().unwrap_or_default()).collect();
    assert_eq!(lines[0], "REQ 1 BEGIN");
    assert_eq!(lines[1], "REQ 2 PLAY 1-10 a");
    assert_eq!(lines[2], "REQ 3 PLAY 2-10 b");
    assert_eq!(lines[3], "COMMIT");
}

#[tokio::test]
async fn a_single_command_batch_skips_the_wrapper() {
    let (client, mut seen) = fake_server(|line, out| {
        out.push(format!("RES {} 202 PLAY OK\r\n", req_id(line)));
    })
    .await;

    client.batch(vec![commands::play(1, 10)]).await.unwrap();
    assert_eq!(seen.recv().await.unwrap(), "REQ 1 PLAY 1-10");
    assert!(seen.try_recv().is_err(), "no BEGIN/COMMIT should be sent");
}

#[tokio::test]
async fn an_untagged_reply_is_broadcast_rather_than_mismatched() {
    let (client, _seen) = fake_server(|line, out| {
        // A 100-series notification arrives unsolicited, then the real reply.
        out.push("100 INFO\r\n".to_string());
        out.push(format!("RES {} 202 PLAY OK\r\n", req_id(line)));
    })
    .await;

    let mut notes = client.notifications();
    let resp = client.send(commands::play(1, 10)).await.unwrap();
    assert_eq!(resp.code, 202);

    let note = tokio::time::timeout(Duration::from_secs(1), notes.recv()).await.unwrap().unwrap();
    assert_eq!(note.code, 100);
}

#[tokio::test]
async fn losing_the_connection_wakes_waiters() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        // Read one command, then hang up without replying.
        let mut lines = BufReader::new(socket).lines();
        let _ = lines.next_line().await;
    });

    let client = Client::connect(addr).await.unwrap();
    let err = client.send(commands::play(1, 10)).await.unwrap_err();
    assert!(
        matches!(err, amcp::Error::Disconnected),
        "a dropped connection must not leave the caller waiting for the timeout, got {err:?}"
    );
}

#[tokio::test]
async fn commands_are_written_in_order_from_concurrent_callers() {
    let (client, mut seen) = fake_server(|line, out| {
        out.push(format!("RES {} 202 OK\r\n", req_id(line)));
    })
    .await;

    // from_stream is exercised implicitly by connect; this checks the writer
    // does not interleave two commands' bytes.
    let a = client.send(Command::new("ONE"));
    let b = client.send(Command::new("TWO"));
    let _ = tokio::join!(a, b);

    let mut lines = vec![seen.recv().await.unwrap(), seen.recv().await.unwrap()];
    lines.sort();
    assert_eq!(lines, vec!["REQ 1 ONE".to_string(), "REQ 2 TWO".to_string()]);
}

/// `from_stream` is public so a caller can drive an already-open socket; make
/// sure that path works too.
#[tokio::test]
async fn from_stream_drives_an_existing_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let id = req_id(&line);
            let _ = write.write_all(format!("RES {id} 202 OK\r\n").as_bytes()).await;
        }
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let client = Client::from_stream(stream);
    assert_eq!(client.send(Command::new("CLS")).await.unwrap().code, 202);
}

#[tokio::test]
async fn ping_is_matched_despite_having_no_status_code() {
    // The server answers PING with `PONG …` — no status code, no RES prefix,
    // and the REQ id discarded. Without special handling the caller would wait
    // out the full reply timeout on a command that did in fact succeed.
    let (client, mut seen) = fake_server(|line, out| {
        if line.starts_with("PING") {
            out.push("PONG\r\n".to_string());
        } else {
            out.push(format!("RES {} 202 OK\r\n", req_id(line)));
        }
    })
    .await;

    let resp = tokio::time::timeout(Duration::from_secs(5), client.send(Command::new("PING")))
        .await
        .expect("PING must not hang")
        .unwrap();
    assert!(resp.is_ok());
    assert_eq!(resp.status, "PONG");

    // Sent bare — a REQ prefix would be discarded by the server anyway.
    assert_eq!(seen.recv().await.unwrap(), "PING");

    // The connection is still usable afterwards.
    assert_eq!(client.send(Command::new("CLS")).await.unwrap().code, 202);
}
