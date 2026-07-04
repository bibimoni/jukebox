use jukebox::prompt::prompt_source_dir_with;
use tempfile::tempdir;

#[test]
fn ensure_config_runs_first_run_from_stdin() {
    let tmp = tempdir().unwrap();
    std::env::set_var("HOME", tmp.path());
    std::env::remove_var("XDG_CONFIG_HOME");
    // create a valid source dir with a flac
    let src = tmp.path().join("lossless");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.flac"), b"x").unwrap();
    // feed the path on stdin
    let input = format!("{}\n", src.display());
    // We can't easily redirect stdin in a unit test; instead call prompt_source_dir
    // with a Cursor-like reader by constructing it via a helper.
    let mut buf = std::io::Cursor::new(input.into_bytes());
    let chosen = prompt_source_dir_with(&mut buf, &src).unwrap();
    assert_eq!(chosen, src.canonicalize().unwrap());
}
