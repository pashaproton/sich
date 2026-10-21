use std::process::Command;

#[test]
fn empty_config_exits_zero() {
    let out = Command::new(env!("CARGO_BIN_EXE_sich"))
        .args(["--config", "/dev/null"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn bad_config_exits_deny_unknown_fields() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/deny_unknown_fields_config.toml"
    );
    let out = Command::new(env!("CARGO_BIN_EXE_sich"))
        .args(["--config", path])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown field `listener`"),
        "stderr must name the offending value (INV-3)"
    );
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn bad_config_exits_ftp() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ftp_config.toml"
    );
    let out = Command::new(env!("CARGO_BIN_EXE_sich"))
        .args(["--config", path])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("nknown variant `ftp`"),
        "stderr must name the offending value (INV-3)"
    );
    assert_eq!(out.status.code(), Some(2));
}
