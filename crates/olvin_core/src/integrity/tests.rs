use super::*;

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olvin-integrity-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn signer(seed: u8) -> Signer {
    Signer::from_seed(&SecretBytes::new(vec![seed; 32])).unwrap()
}

/// A game folder as the hub lays it out, signed with `signer`.
fn signed_game(dir: &Path, signer: &Signer) {
    fs::create_dir_all(dir.join("assets/scenes")).unwrap();
    fs::write(dir.join("game"), b"\x7fELF pretend executable").unwrap();
    fs::write(dir.join("assets/scenes/main.scene.ron"), "()").unwrap();
    fs::write(dir.join("assets/a.lua"), "print('hi')").unwrap();
    fs::write(dir.join("assets/.hidden"), "x").unwrap();
    fs::write(dir.join(".olvin-build"), "marker").unwrap();
    let manifest = Manifest::build(dir, "game", "0.1.0").unwrap();
    let json = manifest.to_json();
    fs::write(dir.join(MANIFEST_FILE), &json).unwrap();
    fs::write(
        dir.join(SIGNATURE_FILE),
        format!("{}\n", signer.sign_hex(json.as_bytes())),
    )
    .unwrap();
}

#[test]
fn manifest_lists_shipped_files_sorted_without_hidden_ones() {
    let dir = temp_dir("list");
    signed_game(&dir, &signer(1));
    let manifest = Manifest::parse(&fs::read(dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    let paths: Vec<_> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        ["assets/a.lua", "assets/scenes/main.scene.ron", "game"]
    );
    let exe = manifest.files.iter().find(|f| f.path == "game").unwrap();
    assert_eq!(manifest.build_id, exe.sha256[..16]);
    assert_eq!(exe.sha256.len(), 64);
    assert_eq!(manifest.files[1].size, 2);
    // Same folder, same bytes: what gets signed is reproducible.
    let again = Manifest::build(&dir, "game", "0.1.0").unwrap().to_json();
    assert_eq!(again, fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap());
    assert!(Manifest::build(&dir, "nope", "0.1.0").is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_untouched_folder_verifies() {
    let dir = temp_dir("ok");
    let key = signer(1);
    signed_game(&dir, &key);
    let report = verify_dir(&dir, Some(&key.public_key()));
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(report.signature, SignatureState::Valid);
    assert_eq!(report.files_checked, 3);
    assert!(report.bytes_hashed > 0 && report.build_id.is_some());
    // Without a key only the hashes are compared, and the report says so.
    let report = verify_dir(&dir, None);
    assert!(report.ok());
    assert_eq!(report.signature, SignatureState::NotChecked);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn changed_missing_and_added_files_are_found() {
    let dir = temp_dir("tamper");
    let key = signer(1);
    signed_game(&dir, &key);
    let public = key.public_key();

    // Same size, other bytes: the hash catches it, not just the length.
    fs::write(dir.join("assets/scenes/main.scene.ron"), "[]").unwrap();
    // A longer file.
    fs::write(dir.join("assets/a.lua"), "os.execute('x')").unwrap();
    fs::remove_file(dir.join("game")).unwrap();
    fs::write(dir.join("assets/evil.lua"), "x").unwrap();
    // Outside assets/ extra files are the game's own (logs, saves): not reported.
    fs::create_dir_all(dir.join("logs")).unwrap();
    fs::write(dir.join("logs/game.log"), "x").unwrap();

    let report = verify_dir(&dir, Some(&public));
    assert_eq!(
        report.signature,
        SignatureState::Valid,
        "the manifest itself is fine"
    );
    let mut problems: Vec<_> = report.problems.iter().map(ToString::to_string).collect();
    problems.sort();
    assert_eq!(
        problems,
        [
            "missing: game",
            "modified: assets/a.lua",
            "modified: assets/scenes/main.scene.ron",
            "not in the manifest: assets/evil.lua",
        ]
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_forged_or_edited_manifest_fails_the_signature() {
    let dir = temp_dir("forged");
    let key = signer(1);
    signed_game(&dir, &key);

    // Someone else's key: signature invalid, and nothing else is trusted or checked.
    let other = signer(2).public_key();
    let report = verify_dir(&dir, Some(&other));
    assert_eq!(report.signature, SignatureState::Failed);
    assert_eq!(report.problems, [Problem::SignatureInvalid]);
    assert_eq!(report.files_checked, 0);

    // The manifest edited to match a patched file, with the old signature.
    let path = dir.join(MANIFEST_FILE);
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("0.1.0", "0.1.1")).unwrap();
    let report = verify_dir(&dir, Some(&key.public_key()));
    assert_eq!(report.problems, [Problem::SignatureInvalid]);

    // A stripped signature is not a way around the check.
    fs::remove_file(dir.join(SIGNATURE_FILE)).unwrap();
    let report = verify_dir(&dir, Some(&key.public_key()));
    assert_eq!(report.problems, [Problem::SignatureMissing]);

    // Nor is a stripped manifest.
    fs::remove_file(&path).unwrap();
    let report = verify_dir(&dir, Some(&key.public_key()));
    assert_eq!(report.problems, [Problem::ManifestMissing]);
    assert_eq!(report.signature, SignatureState::Failed);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn garbage_signatures_and_manifests_are_rejected_not_panicked_on() {
    let dir = temp_dir("garbage");
    let key = signer(1);
    signed_game(&dir, &key);
    let public = key.public_key();
    for sig in ["", "zz", "00", &"0".repeat(128), &"f".repeat(129)] {
        fs::write(dir.join(SIGNATURE_FILE), sig).unwrap();
        let report = verify_dir(&dir, Some(&public));
        assert_eq!(report.problems, [Problem::SignatureInvalid], "{sig:?}");
    }
    // A validly signed but broken manifest.
    let broken = b"{ not json";
    fs::write(dir.join(MANIFEST_FILE), broken).unwrap();
    fs::write(dir.join(SIGNATURE_FILE), key.sign_hex(broken)).unwrap();
    let report = verify_dir(&dir, Some(&public));
    assert!(matches!(report.problems[..], [Problem::ManifestInvalid(_)]));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn manifest_paths_cannot_leave_the_folder() {
    let dir = temp_dir("traversal");
    let key = signer(1);
    signed_game(&dir, &key);
    fs::write(dir.parent().unwrap().join("outside.txt"), "x").unwrap();
    let mut manifest = Manifest::parse(&fs::read(dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    for path in [
        "../outside.txt",
        "/etc/passwd",
        "assets/../../outside.txt",
        "",
    ] {
        manifest.files.push(FileEntry {
            path: path.into(),
            size: 1,
            sha256: "0".repeat(64),
        });
    }
    let json = manifest.to_json();
    fs::write(dir.join(MANIFEST_FILE), &json).unwrap();
    fs::write(dir.join(SIGNATURE_FILE), key.sign_hex(json.as_bytes())).unwrap();
    let report = verify_dir(&dir, Some(&key.public_key()));
    let unsafe_paths = report
        .problems
        .iter()
        .filter(|p| matches!(p, Problem::UnsafePath(_)))
        .count();
    assert_eq!(unsafe_paths, 4, "{:?}", report.problems);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn big_files_are_hashed_in_a_stream() {
    let dir = temp_dir("big");
    let data: Vec<u8> = (0..3_000_017u32).map(|i| (i * 7 % 251) as u8).collect();
    fs::write(dir.join("big.bin"), &data).unwrap();
    let expected = encode_hex(&Sha256::digest(&data));
    assert_eq!(sha256_file(&dir.join("big.bin")).unwrap(), expected);
    // Known vector: SHA-256("abc").
    fs::write(dir.join("abc"), "abc").unwrap();
    assert_eq!(
        sha256_file(&dir.join("abc")).unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn keys_parse_from_hex_and_never_print_the_seed() {
    let key = signer(7);
    let public = key.public_key();
    let hex = public.to_hex();
    assert_eq!(hex.len(), 64);
    assert_eq!(PublicKey::from_hex(&format!(" {hex}\n")).unwrap(), public);
    for bad in ["", "abc", &"g".repeat(64), &"00".repeat(31)] {
        assert!(PublicKey::from_hex(bad).is_err(), "{bad:?}");
    }
    assert!(Signer::from_seed(&SecretBytes::new(vec![1; 31])).is_none());
    assert_eq!(format!("{key:?}"), "Signer([hidden])");
    assert!(!format!("{key:?}{public:?}").contains(&"07".repeat(8)));
    assert_eq!(decode_hex("00ff10"), Some(vec![0, 255, 16]));
    assert_eq!(decode_hex("0"), None);
    assert_eq!(decode_hex("é1"), None);
}

fn plugin_run(dir: &Path, key: &str, policy: Policy) -> App {
    let mut app = App::new();
    app.add_plugin(IntegrityPlugin::new(Some(key)).policy(policy).dir(dir));
    app
}

#[test]
fn enforce_stops_the_app_warn_does_not() {
    let dir = temp_dir("plugin");
    let key = signer(1);
    signed_game(&dir, &key);
    let hex = key.public_key().to_hex();

    let app = plugin_run(&dir, &hex, Policy::Enforce);
    assert!(!app.exit_requested());
    let report = app.resources.fetch::<IntegrityReport>();
    assert!(report.ok() && report.skipped.is_none());
    drop(report);

    fs::write(dir.join("assets/a.lua"), "tampered").unwrap();
    let app = plugin_run(&dir, &hex, Policy::Enforce);
    assert!(app.exit_requested(), "a modified file stops the game");
    assert!(!app.resources.fetch::<IntegrityReport>().ok());
    let app = plugin_run(&dir, &hex, Policy::Warn);
    assert!(!app.exit_requested(), "warn only logs");
    assert!(!app.resources.fetch::<IntegrityReport>().ok());

    // A broken embedded key is a failure, not a pass.
    let app = plugin_run(&dir, "not a key", Policy::Enforce);
    assert!(app.exit_requested());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn without_a_key_the_plugin_is_inert() {
    let mut app = App::new();
    app.add_plugin(IntegrityPlugin::new(None));
    assert!(!app.exit_requested());
    let report = app.resources.fetch::<IntegrityReport>();
    assert!(report.skipped.is_some() && report.ok());
}
