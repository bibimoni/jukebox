use jukebox::yt::proto::*;

#[test]
fn search_request_serializes_to_line() {
    let r = Request::Search { q: "adele hello".into(), limit: 25 };
    let line = r.to_line();
    assert!(line.contains("\"cmd\":\"search\""));
    assert!(line.contains("\"q\":\"adele hello\""));
    assert!(!line.contains('\n')); // single line
}

#[test]
fn response_round_trips_search() {
    let wire = r#"{"ok":true,"data":{"search":[{"video_id":"v1","title":"Hello","artist":"Adele","album":null,"dur":295.0,"isrc":null}]}}"#;
    let r = Response::from_line(wire).unwrap();
    match r {
        Response::Search(v) => {
            assert_eq!(v.len(), 1);
            assert_eq!(v[0].video_id, "v1");
            assert_eq!(v[0].dur, Some(295.0));
            assert_eq!(v[0].artist, "Adele");
        }
        other => panic!("expected Search, got {other:?}"),
    }
}

#[test]
fn response_round_trips_resolve() {
    let wire = r#"{"ok":true,"data":{"resolve":{"url":"https://x","expires_at":1234.0,"codec":"AAC","abr":256,"sample_rate":48000,"container":"m4a","premium":true}}}"#;
    let r = Response::from_line(wire).unwrap();
    match r {
        Response::Resolve(u) => {
            assert_eq!(u.abr, 256);
            assert!(u.premium);
            assert_eq!(u.sample_rate, 48000);
        }
        _ => panic!("expected Resolve"),
    }
}

#[test]
fn response_error() {
    let wire = r#"{"ok":false,"error":"rate limited"}"#;
    let r = Response::from_line(wire).unwrap();
    assert!(matches!(r, Response::Error(e) if e == "rate limited"));
}

#[test]
fn response_pong_and_auth() {
    let pong = Response::from_line(r#"{"ok":true,"data":{"pong":true}}"#).unwrap();
    assert!(matches!(pong, Response::Pong));
    let auth = Response::from_line(
        r#"{"ok":true,"data":{"auth":{"ok":true,"premium":true,"account":true}}}"#,
    )
    .unwrap();
    assert!(matches!(auth, Response::Auth(a) if a.premium && a.account));
}

#[test]
fn response_playlists_and_suggestions() {
    let pl = Response::from_line(
        r#"{"ok":true,"data":{"playlists":[{"id":"PL1","name":"Liked","count":30}]}}"#,
    )
    .unwrap();
    assert!(matches!(pl, Response::Playlists(v) if v[0].name=="Liked"));
    let sg = Response::from_line(
        r#"{"ok":true,"data":{"suggestions":[{"id":"RD1","name":"Chill","count":0}]}}"#,
    )
    .unwrap();
    assert!(matches!(sg, Response::Suggestions(v) if v[0].id=="RD1"));
}

use jukebox::yt::sidecar::Sidecar;
use std::io::Write;

/// A fake sidecar script that echoes a canned "pong" for any input.
fn fake_script() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("fake-sidecar-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    writeln!(f, "import sys,json").unwrap();
    writeln!(f, "for line in sys.stdin:").unwrap();
    writeln!(f, "    line=line.strip()").unwrap();
    writeln!(f, "    if not line: continue").unwrap();
    writeln!(f, "    print(json.dumps({{'ok':True,'data':{{'pong':True}}}}), flush=True)").unwrap();
    p
}

#[test]
fn sidecar_send_then_recv_ping() {
    let python = std::path::PathBuf::from("python3");
    let script = fake_script();
    let mut s = Sidecar::spawn(&python, &script, None, None, None).unwrap();
    s.send(&Request::Ping).unwrap();
    let mut got = None;
    for _ in 0..50 {
        if let Ok(Some(r)) = s.try_recv() {
            got = Some(r);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(matches!(got, Some(Response::Pong)), "got {got:?}");
    let _ = std::fs::remove_file(&script);
}

#[test]
fn sidecar_try_recv_none_when_idle() {
    let python = std::path::PathBuf::from("python3");
    let script = fake_script();
    let mut s = Sidecar::spawn(&python, &script, None, None, None).unwrap();
    // nothing sent yet — no response pending
    assert!(matches!(s.try_recv().unwrap(), None));
    let _ = std::fs::remove_file(&script);
}

#[test]
fn session_spawn_and_auth_status_no_cookies() {
    // The fake script returns pong for any input; Session.auth_status sends
    // AuthStatus, the fake returns Pong — so we just assert no panic/hang and
    // that Session can be constructed against a fake sidecar.
    let python = std::path::PathBuf::from("python3");
    let script = fake_script();
    let s = jukebox::yt::session::Session::spawn(&python, &script, None);
    assert!(s.is_ok(), "session spawn failed");
    let _ = std::fs::remove_file(&script);
}
