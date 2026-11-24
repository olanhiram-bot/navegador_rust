use std::process::Command;

#[test]
fn verify_log_verifica_sin_red_y_rechaza_ficheros_ausentes_o_danados() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recibos.jsonl");
    let verify = || {
        Command::new(env!("CARGO_BIN_EXE_navegador"))
            .arg("verify-log")
            .arg(&path)
            .output()
            .unwrap()
    };

    assert_eq!(verify().status.code(), Some(2));
    assert!(!path.exists());

    std::fs::write(&path, "").unwrap();
    let result = verify();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("0 recibos"));

    std::fs::write(&path, "{entrada incompleta\n").unwrap();
    assert_eq!(verify().status.code(), Some(2));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{entrada incompleta\n"
    );
}
