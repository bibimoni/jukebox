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
