//! Acceptance tests for file hashing. A coder agent makes these pass without editing them.

use dermixen_media::hash_file;

#[test]
fn the_hash_follows_the_bytes_and_not_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("track.wav");
    std::fs::write(&original, b"some audio bytes").unwrap();
    let hash = hash_file(&original).unwrap();
    assert_eq!(hash.0, *blake3::hash(b"some audio bytes").as_bytes());
    assert_eq!(hash.to_string().len(), 64);

    let moved = dir.path().join("renamed and moved.wav");
    std::fs::rename(&original, &moved).unwrap();
    assert_eq!(hash_file(&moved).unwrap(), hash);

    std::fs::write(&moved, b"some audio byteS").unwrap();
    assert_ne!(hash_file(&moved).unwrap(), hash);
}

#[test]
fn an_empty_file_hashes_and_a_missing_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.wav");
    std::fs::write(&empty, b"").unwrap();
    assert_eq!(hash_file(&empty).unwrap().0, *blake3::hash(b"").as_bytes());
    assert!(hash_file(&dir.path().join("missing.wav")).is_err());
}

#[test]
fn a_large_file_is_hashed_whole() {
    // Two megabytes, more than one read buffer of any plausible size.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.wav");
    let bytes: Vec<u8> = (0..2_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(
        hash_file(&path).unwrap().0,
        *blake3::hash(&bytes).as_bytes()
    );
}

#[cfg(unix)]
#[test]
fn a_device_a_pipe_and_a_folder_are_refused_at_once() {
    let folder = tempfile::tempdir().unwrap();
    let pipe = folder.path().join("pipe.wav");
    let made = std::process::Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .unwrap();
    assert!(made.success());
    let link = folder.path().join("zero.wav");
    std::os::unix::fs::symlink("/dev/zero", &link).unwrap();

    for path in [
        std::path::PathBuf::from("/dev/zero"),
        link,
        pipe,
        folder.path().to_path_buf(),
    ] {
        let shown = path.display().to_string();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let hashed = dermixen_media::hash_file(&path).map(|_| ());
            let decoded = dermixen_media::decode(&path).map(|_| ());
            sender.send((hashed, decoded))
        });
        let (hashed, decoded) = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap_or_else(|_| panic!("{shown}: no answer within two seconds"));
        assert!(hashed.is_err(), "{shown} was hashed");
        assert!(
            matches!(decoded, Err(dermixen_media::DecodeError::Read { .. })),
            "{shown}: {decoded:?}"
        );
    }
}
