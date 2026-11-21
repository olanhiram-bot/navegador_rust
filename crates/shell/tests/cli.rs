//! Interfaz de línea de comandos de la shell.

use std::process::Command;

#[test]
fn la_version_se_puede_consultar_sin_abrir_ventana() {
    let output = Command::new(env!("CARGO_BIN_EXE_navegador-shell"))
        .arg("--version")
        .output()
        .expect("ejecutar la shell");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("navegador-shell 0.1.0"),
        "salida inesperada: {text:?}"
    );
}
